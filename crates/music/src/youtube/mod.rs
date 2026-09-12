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
    InputSource, MusicProvider, PromptSink, ProviderSession, Shape, SignIn, SignInPrompt,
    UserProfile, WebSignIn, credentials,
};
pub use client::YouTubeClient;

const GUEST_ID: &str = "youtube-guest";
/// Google's sign-in page, told to come back to YouTube Music once the account is in: the same url
/// YouTube Music's own Sign in button opens, `www.youtube.com/signin` hop included, so that the
/// second load the sign-in window falls back on runs exactly what that button would.
const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";
const LANDING: &str = "music.youtube.com";
const COOKIE_DOMAIN: &str = "youtube.com";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Saved {
    Cookies {
        cookies: String,
        authuser: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_id: Option<String>,
    },
    Guest,
}

pub struct YouTubeProvider {
    credentials: PathBuf,
    /// The cookie store ytmusic writes back to as Google rotates the session. The pasted
    /// header in the credential file seeds it when it is missing.
    cookies: PathBuf,
    resolved: PathBuf,
    player: PathBuf,
}

impl YouTubeProvider {
    pub fn new() -> Self {
        let cache = credentials::dir("youtube");
        Self {
            credentials: cache.join(credentials::FILE),
            cookies: cache.join("cookies.json"),
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

    fn cookie_client(&self, cookies: &str, authuser: usize, page_id: Option<&str>) -> Arc<YtMusic> {
        let api = YtMusic::with_cookies(cookies).as_user(authuser);
        let api = match page_id {
            Some(page) => api.as_page(page),
            None => api,
        };
        Arc::new(
            api.persist_cookies(self.cookies.clone())
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
            shape: Shape::Saved,
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
            shape: Shape::Saved,
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

        let page_id = account.identity.page_id.clone();
        let profile = wire::profile(account.identity.profile.clone());
        credentials::remove(&self.cookies);
        let api = self.cookie_client(&cookies, account.index, page_id.as_deref());
        self.store_cookies(&cookies, account.index, page_id.clone())?;
        log::debug!(
            "youtube: cookie sign-in succeeded for authuser {} page {page_id:?}",
            account.index
        );
        Ok(self.authenticated_session(api, profile))
    }

    fn store_cookies(&self, cookies: &str, authuser: usize, page_id: Option<String>) -> Result<()> {
        self.save(&Saved::Cookies {
            cookies: cookies.to_owned(),
            authuser,
            page_id,
        })
        .context("cannot store youtube cookies")
    }

    async fn restore_cookies(
        &self,
        cookies: &str,
        authuser: usize,
        page_id: Option<&str>,
    ) -> Option<ProviderSession> {
        let api = self.cookie_client(cookies, authuser, page_id);
        match api.profile().await {
            Ok(profile) => {
                log::debug!(
                    "youtube: restored the session for authuser {authuser} page {page_id:?}"
                );
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
                page_id: None,
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
        .find(|account| account.id() == picked.trim())
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

    fn public_art(&self) -> bool {
        true
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Anonymous, SignIn::Secret]
    }

    fn stored(&self) -> bool {
        self.credentials.exists()
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        match self.saved() {
            Some(Saved::Cookies {
                cookies,
                authuser,
                page_id,
            }) => Ok(self
                .restore_cookies(&cookies, authuser, page_id.as_deref())
                .await),
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
            SignIn::Credentials { .. } => Err(anyhow::anyhow!(
                "youtube does not sign in with a server address"
            )),
        }
    }

    fn sign_out(&self) {
        credentials::remove(&self.credentials);
        credentials::remove(&self.cookies);
    }

    fn web_sign_in(&self) -> Option<WebSignIn> {
        Some(WebSignIn {
            label: Some("login-sign-in-google"),
            url: SIGN_IN_URL,
            landing: LANDING,
            domain: COOKIE_DOMAIN,
            proof: auth::PROOF,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Saved, YouTubeProvider};
    use crate::{MusicProvider, SignIn, WebSignIn};

    #[test]
    fn cookie_credentials_keep_the_selected_identity() {
        let saved = Saved::Cookies {
            cookies: "SAPISID=secret".to_owned(),
            authuser: 3,
            page_id: Some("brand-page".to_owned()),
        };
        let encoded = serde_json::to_vec(&saved).unwrap();
        let restored: Saved = serde_json::from_slice(&encoded).unwrap();
        match restored {
            Saved::Cookies {
                cookies,
                authuser,
                page_id,
            } => {
                assert_eq!(cookies, "SAPISID=secret");
                assert_eq!(authuser, 3);
                assert_eq!(page_id.as_deref(), Some("brand-page"));
            }
            Saved::Guest => panic!("cookie credentials became a guest session"),
        }
    }

    #[test]
    fn guest_credentials_round_trip() {
        let encoded = serde_json::to_vec(&Saved::Guest).unwrap();
        assert!(matches!(
            serde_json::from_slice::<Saved>(&encoded).unwrap(),
            Saved::Guest
        ));
    }

    #[test]
    fn browser_sign_in_is_google_labeled_without_changing_sign_in_methods() {
        let provider = YouTubeProvider::new();
        let browser = provider.web_sign_in().unwrap();
        assert_eq!(browser.label, Some("login-sign-in-google"));
        assert_eq!(
            provider.sign_in_options(),
            vec![SignIn::Anonymous, SignIn::Secret]
        );
        let _: WebSignIn = browser;
    }
}
