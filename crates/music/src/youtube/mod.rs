mod accounts;
mod auth;
mod client;
mod genres;
mod playback;
mod subscriptions;
mod trim;
mod wire;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ytmusic::YtMusic;

use crate::youtube::playback::Factory;

use crate::{
    InputSource, MusicProvider, PromptSink, ProviderSession, SignIn, SignInPrompt, UserProfile,
    credentials,
};
pub use client::YouTubeClient;

const GUEST_ID: &str = "youtube-guest";

/// What the credential file remembers between launches: a browser sign-in with the
/// Google account index it belongs to, or the choice to listen as a guest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Saved {
    Cookies { cookies: String, authuser: usize },
    Guest,
}

pub struct YouTubeProvider {
    credentials: PathBuf,
    resolved: PathBuf,
    player: PathBuf,
}

impl YouTubeProvider {
    pub fn new() -> Self {
        let cache = credentials::dir("youtube");
        Self {
            credentials: cache.join(credentials::FILE),
            resolved: cache.join("resolved.json"),
            player: cache.join("player.json"),
        }
    }

    fn save(&self, saved: &Saved) -> Result<()> {
        save(&self.credentials, saved)
    }

    fn saved(&self) -> Option<Saved> {
        let body = std::fs::read(&self.credentials).ok()?;
        match serde_json::from_slice(&body) {
            Ok(saved) => Some(saved),
            Err(error) => {
                log::warn!("youtube: cannot read the stored credentials: {error}");
                None
            }
        }
    }

    fn cookie_client(&self, cookies: &str, authuser: usize) -> Arc<YtMusic> {
        Arc::new(
            YtMusic::with_cookies(cookies)
                .as_user(authuser)
                .cache_resolutions(self.resolved.clone())
                .cache_player(self.player.clone()),
        )
    }

    fn guest_client(&self) -> Arc<YtMusic> {
        Arc::new(YtMusic::anonymous().cache_player(self.player.clone()))
    }

    fn authenticated_session(&self, api: Arc<YtMusic>, profile: UserProfile) -> ProviderSession {
        let client = YouTubeClient::new(api.clone()).owned_by(profile.display_name.clone());
        ProviderSession {
            profile,
            api: Arc::new(client),
            playback: Arc::new(Factory::new(api)),
            authenticated: true,
            playcounts: false,
        }
    }

    fn guest_session(&self, api: Arc<YtMusic>) -> ProviderSession {
        ProviderSession {
            profile: UserProfile {
                id: GUEST_ID.to_string(),
                display_name: "YouTube Music".to_string(),
            },
            api: Arc::new(YouTubeClient::new(api.clone())),
            playback: Arc::new(Factory::new(api)),
            authenticated: false,
            playcounts: false,
        }
    }

    async fn connect(
        &self,
        cookies: &str,
        prompt: &PromptSink,
        input: &mut InputSource,
    ) -> Result<ProviderSession> {
        let cookies = auth::header(cookies)?;

        let found = accounts::list(&cookies).await;
        let account = match found.len() {
            0 => anyhow::bail!("cookies were not accepted; sign in to the browser first"),
            1 => &found[0],
            _ => pick(&found, prompt, input).await?,
        };

        let profile = wire::profile(account.profile.clone());
        let api = self.cookie_client(&cookies, account.index);
        self.store_cookies(&cookies, account.index)?;
        log::debug!(
            "youtube: cookie sign-in succeeded for authuser {}",
            account.index
        );
        Ok(self.authenticated_session(api, profile))
    }

    fn store_cookies(&self, cookies: &str, authuser: usize) -> Result<()> {
        self.save(&Saved::Cookies {
            cookies: cookies.to_owned(),
            authuser,
        })
        .context("cannot store youtube cookies")
    }

    async fn restore_cookies(&self, cookies: &str, authuser: usize) -> Option<ProviderSession> {
        let api = self.cookie_client(cookies, authuser);
        match api.profile().await {
            Ok(profile) => {
                log::debug!("youtube: restored the session for authuser {authuser}");
                Some(self.authenticated_session(api, wire::profile(profile)))
            }
            Err(error) => {
                log::warn!("youtube: the cached cookies are no longer usable: {error:#}");
                None
            }
        }
    }

    fn store_guest(&self) {
        if let Err(error) = self.save(&Saved::Guest) {
            log::warn!("youtube: cannot remember the guest session: {error:#}");
        }
    }
}

/// Folds the `cookies.txt`, `authuser.txt` and `guest` files releases before 0.31 kept
/// into the single credential file, then removes them. Part of the startup migration pass.
pub(crate) fn migrate() {
    let cache = credentials::dir("youtube");
    let file = cache.join(credentials::FILE);
    let cookies = cache.join("cookies.txt");
    let authuser = cache.join("authuser.txt");
    let guest = cache.join("guest");
    if !file.exists() {
        let legacy = match std::fs::read_to_string(&cookies) {
            Ok(text) if !text.trim().is_empty() => Some(Saved::Cookies {
                cookies: text.trim().to_owned(),
                authuser: std::fs::read_to_string(&authuser)
                    .ok()
                    .and_then(|stored| stored.trim().parse().ok())
                    .unwrap_or(0),
            }),
            _ if guest.exists() => Some(Saved::Guest),
            _ => None,
        };
        if let Some(saved) = legacy
            && let Err(error) = save(&file, &saved)
        {
            log::warn!("youtube: cannot adopt the old credential files: {error:#}");
            return;
        }
    }
    for path in [&cookies, &authuser, &guest] {
        credentials::remove(path);
    }
}

fn save(file: &std::path::Path, saved: &Saved) -> Result<()> {
    let body = serde_json::to_vec_pretty(saved).context("cannot encode youtube credentials")?;
    credentials::write(file, &body)
}

async fn pick<'a>(
    found: &'a [accounts::Account],
    prompt: &PromptSink,
    input: &mut InputSource,
) -> Result<&'a accounts::Account> {
    prompt(SignInPrompt::Accounts(
        found.iter().map(accounts::Account::choice).collect(),
    ));
    let picked = input.recv().await.context("sign-in was cancelled")?;
    found
        .iter()
        .find(|account| account.index.to_string() == picked.trim())
        .context("that account is no longer signed in")
}

impl Default for YouTubeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for YouTubeProvider {
    fn name(&self) -> &'static str {
        "YouTube Music"
    }

    fn slug(&self) -> &'static str {
        "youtube"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Anonymous, SignIn::Secret]
    }

    fn stored(&self) -> bool {
        self.credentials.exists()
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        match self.saved() {
            Some(Saved::Cookies { cookies, authuser }) => {
                Ok(self.restore_cookies(&cookies, authuser).await)
            }
            Some(Saved::Guest) => {
                log::debug!("youtube: restoring guest session");
                Ok(Some(self.guest_session(self.guest_client())))
            }
            None => Ok(None),
        }
    }

    async fn sign_in(
        &self,
        method: SignIn,
        prompt: crate::PromptSink,
        mut input: InputSource,
    ) -> Result<ProviderSession> {
        match method {
            SignIn::Anonymous | SignIn::Default => {
                self.store_guest();
                Ok(self.guest_session(self.guest_client()))
            }
            SignIn::Secret => {
                prompt(SignInPrompt::Secret);
                let cookies = input.recv().await.context("sign-in was cancelled")?;
                self.connect(&cookies, &prompt, &mut input).await
            }
            SignIn::Path(_) => Err(anyhow::anyhow!(
                "youtube does not sign in with a folder path"
            )),
        }
    }

    fn sign_out(&self) {
        credentials::remove(&self.credentials);
    }
}
