use std::path::{Path, PathBuf};
use std::sync::Mutex as SyncMutex;

use anyhow::{Context as _, Result, anyhow};
use ring::rand::{SecureRandom as _, SystemRandom};
use spotifyapis::access_point::AudioKeyError;
use spotifyapis::auth::{AuthError, OAuth};
use tokio::sync::Mutex;

use music::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, credentials};

use super::session::Session;

pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

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

    /// Whether a Spotify OAuth sign-in has been saved.
    pub fn stored(&self) -> bool {
        OAuth::stored(&self.oauth_file())
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
        return Ok(None);
    }
    let Some(oauth) =
        OAuth::restore(config.credential_path()?, &config.client_id).map_err(sign_in_error)?
    else {
        return Ok(None);
    };
    oauth.validate().await.map_err(sign_in_error)?;
    Ok(Some(Session::new(oauth, device_id(&config.cache_dir)?)))
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
