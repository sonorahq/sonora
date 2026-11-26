use std::path::{Path, PathBuf};
use std::sync::Mutex as SyncMutex;

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::Deserialize;
use spotifyapis::access_point::AudioKeyError;
use spotifyapis::auth::{AuthError, OAuth};
use tokio::sync::Mutex;

use music::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, credentials};

use super::session::Session;

pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";
/// librespot's `AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS`, the kind of credential it saved.
const STORED_CREDENTIALS: i32 = 1;

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub client_id: String,
    pub redirect_uri: String,
    pub cache_dir: PathBuf,
}

/// Shares a restored Spotify session across the provider and lyrics lookups.
pub(super) struct Account {
    pub(super) config: AuthConfig,
    restoring: Mutex<()>,
    cached: SyncMutex<Cached>,
}

#[derive(Default)]
struct Cached {
    session: Option<Session>,
    generation: u64,
}

impl Account {
    pub(super) fn new(config: AuthConfig) -> Self {
        Self {
            config,
            restoring: Mutex::new(()),
            cached: SyncMutex::default(),
        }
    }

    pub(super) async fn login(&self, prompt: PromptSink, success_message: &str) -> Result<Session> {
        let path = self.config.credential_path()?;
        let device = device_id(&self.config.cache_dir)?;
        let (before, login) = {
            let cached = self.cached.lock().unwrap();
            let login = OAuth::login(
                path,
                &self.config.client_id,
                &self.config.redirect_uri,
                success_message,
                |url| {
                    prompt(SignInPrompt::Url(url.to_owned()));
                    open::that_in_background(url);
                },
            );
            (cached.generation, login)
        };
        let oauth = login.await.map_err(sign_in_error)?;
        let mut cached = self.cached.lock().unwrap();
        if before != cached.generation || !oauth.has_current_grant() {
            return Err(SignInFailure(SignInProblem::Cancelled).into());
        }
        let session = Session::new(oauth, device);
        cached.session = Some(session.clone());
        Ok(session)
    }

    pub(super) async fn session(&self) -> Result<Option<Session>> {
        let _restore = self.restoring.lock().await;
        let before = {
            let mut cached = self.cached.lock().unwrap();
            if let Some(session) = &cached.session
                && session.api().has_current_grant()
            {
                return Ok(Some(session.clone()));
            }
            cached.session = None;
            cached.generation
        };
        let restored = restore(&self.config).await;
        let mut cached = self.cached.lock().unwrap();
        // Only publish this restore if no sign-out happened while it was validating.
        if cached.generation == before && cached.session.is_none() {
            cached.session = restored?.filter(|session| session.api().has_current_grant());
        }
        Ok(cached.session.clone())
    }

    pub(super) fn forget(&self) {
        let mut cached = self.cached.lock().unwrap();
        cached.generation += 1;
        cached.session = None;
        OAuth::forget(&self.config.oauth_file());
        credentials::remove(&self.config.cache_dir.join(credentials::FILE));
        credentials::remove(&credentials::root().join("spotify").join(credentials::FILE));
    }

    pub(super) fn cancel_login(&self) {
        let mut cached = self.cached.lock().unwrap();
        cached.generation += 1;
        OAuth::cancel_login(&self.config.oauth_file());
    }
}

impl AuthConfig {
    pub fn from_env() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_owned(),
            redirect_uri: std::env::var("SONORA_REDIRECT_URI")
                .unwrap_or_else(|_| DEFAULT_REDIRECT_URI.to_owned()),
            cache_dir: credentials::dir("spotify"),
        }
    }

    /// Whether a Spotify sign-in has been saved, counting one an older Sonora left for
    /// [`restore`] to carry over.
    pub fn stored(&self) -> bool {
        OAuth::stored(&self.oauth_file()) || self.legacy_file().is_file()
    }

    /// The reusable credential librespot saved, which [`restore`] carries over once.
    fn legacy_file(&self) -> PathBuf {
        self.cache_dir.join(credentials::FILE)
    }

    fn oauth_file(&self) -> PathBuf {
        self.cache_dir.join("spotify-oauth.json")
    }

    fn credential_path(&self) -> Result<PathBuf> {
        let private =
            dirs::data_local_dir().context("cannot find a private folder for credentials")?;
        let path = self.oauth_file();
        anyhow::ensure!(
            path.starts_with(&private),
            "cannot store credentials outside {}",
            private.display()
        );
        Ok(path)
    }
}

pub async fn restore(config: &AuthConfig) -> Result<Option<Session>> {
    if !config.oauth_file().try_exists()? {
        return carry_over(config).await;
    }
    let restored = match OAuth::restore(config.credential_path()?, &config.client_id) {
        Ok(Some(oauth)) => oauth.validate().await.map(|()| oauth),
        Ok(None) => return Ok(None),
        Err(error) => Err(error),
    };
    match restored {
        Ok(oauth) => Ok(Some(Session::new(oauth, device_id(&config.cache_dir)?))),
        // A grant Spotify rejects gives way to an older credential still on disk.
        Err(error)
            if matches!(error.downcast_ref(), Some(AuthError::Credentials))
                && config.legacy_file().is_file() =>
        {
            OAuth::forget(&config.oauth_file());
            carry_over(config).await
        }
        Err(error) => Err(sign_in_error(error)),
    }
}

/// The reusable credential librespot kept in `credentials.json`.
#[derive(Deserialize)]
struct Legacy {
    username: String,
    auth_type: i32,
    auth_data: String,
}

/// Signs in with the credential an older Sonora saved through librespot, so updating keeps the
/// account signed in. The credential stays for an older build to use until signing out removes
/// it, unless Spotify rejects it, which removes it at once.
async fn carry_over(config: &AuthConfig) -> Result<Option<Session>> {
    let legacy_path = config.legacy_file();
    let body = match std::fs::read(&legacy_path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot read the previous Spotify sign-in"),
    };
    let legacy = match serde_json::from_slice::<Legacy>(&body) {
        Ok(legacy) if legacy.auth_type == STORED_CREDENTIALS && !legacy.username.is_empty() => {
            legacy
        }
        Ok(_) => return Ok(None),
        Err(error) => {
            log::warn!("spotify: cannot read the previous sign-in: {error}");
            return Ok(None);
        }
    };
    let credential = STANDARD
        .decode(&legacy.auth_data)
        .context("the previous Spotify sign-in is damaged")?;
    let device = device_id(&config.cache_dir)?;
    let adopted = OAuth::adopt(
        config.credential_path()?,
        &config.client_id,
        &device,
        &legacy.username,
        &credential,
    )
    .await;
    let oauth = match adopted {
        Ok(oauth) => oauth,
        Err(error) => {
            if matches!(error.downcast_ref(), Some(AuthError::Credentials)) {
                credentials::remove(&legacy_path);
            }
            return Err(sign_in_error(error));
        }
    };
    log::info!("spotify: carried the sign-in over from the previous version");
    Ok(Some(Session::new(oauth, device)))
}

/// Maps Spotify OAuth failures to the app's existing sign-in outcomes.
pub(super) fn sign_in_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(problem) = error.downcast_ref::<AudioKeyError>() {
        let problem = match problem {
            AudioKeyError::Throttled => return error.context(music::Throttled),
            AudioKeyError::Denied => return error,
            AudioKeyError::Premium => SignInProblem::Premium,
            AudioKeyError::Region => SignInProblem::Region,
            AudioKeyError::Credentials => SignInProblem::Credentials,
            AudioKeyError::Refused => SignInProblem::Refused,
            AudioKeyError::Network => SignInProblem::Network,
        };
        return error.context(SignInFailure(problem));
    }
    let problem = match error.downcast_ref::<AuthError>() {
        Some(AuthError::Cancelled) => SignInProblem::Cancelled,
        Some(AuthError::Credentials) => SignInProblem::Credentials,
        Some(AuthError::Network) => SignInProblem::Network,
        Some(AuthError::Refused) => SignInProblem::Refused,
        None => return error,
    };
    error.context(SignInFailure(problem))
}

/// Persists the Connect device ID so it survives app restarts.
fn device_id(dir: &Path) -> Result<String> {
    let file = dir.join("device-id");
    let kept = std::fs::read_to_string(&file)
        .map(|id| id.trim().to_owned())
        .ok()
        .filter(|id| !id.is_empty());
    if let Some(id) = kept {
        return Ok(id);
    }

    let mut bytes = [0; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow!("cannot generate Spotify device ID"))?;
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let id = format!("{:032x}", u128::from_be_bytes(bytes));
    let id = format!(
        "{}-{}-{}-{}-{}",
        &id[..8],
        &id[8..12],
        &id[12..16],
        &id[16..20],
        &id[20..]
    );
    if let Err(error) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, &id)) {
        log::warn!("auth: cannot keep the device id: {error}");
    }
    Ok(id)
}
