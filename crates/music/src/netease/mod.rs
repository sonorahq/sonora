//! NetEase Cloud Music: sign-in with the `MUSIC_U` cookie a browser holds, the catalog over
//! the endpoints the site's own pages call, and the account's likes, playlists and stream
//! urls beside them.
//!
//! The lyrics source in [`lyrics`] is the same site, and needs only to find a song by name.

mod auth;
mod client;
mod lyrics;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use client::NeteaseClient;
pub use lyrics::NetEase;

use crate::netease::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PromptSink, ProviderSession, Shape,
    SignIn, SignInPrompt, UserProfile, WebSignIn,
};

/// The page the sign-in window opens. The site's login is a route inside its own app, so the
/// window lands on the page that offers the QR code and the password form both.
const SIGN_IN_URL: &str = "https://music.163.com/#/login";
/// Once the cookies land, the window navigates here on its own.
const LANDING: &str = "music.163.com";
/// The cookie filter runs on bare domains (see `webview`'s `matches`), so no leading dot.
const COOKIE_DOMAIN: &str = "music.163.com";

pub struct NeteaseProvider;

impl NeteaseProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(cookies: &str) -> Result<ProviderSession> {
        let credentials = auth::cookies(cookies)?;
        let client = NeteaseClient::connect(&credentials)
            .await
            .context("cannot reach netease with those cookies")?;
        let profile = client.profile().await?;
        auth::store(&credentials)?;
        Ok(session(client, profile))
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(remembered) = auth::load() else {
            return Ok(None);
        };
        match NeteaseClient::connect(&remembered).await {
            Ok(client) => {
                let profile = client.profile().await?;
                Ok(Some(session(client, profile)))
            }
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) => {
                log::warn!("netease: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

fn session(client: NeteaseClient, profile: UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        // the library is what the listener has liked, collected and followed
        shape: Shape::Saved,
        authenticated: true,
        capabilities: Capabilities {
            // the site keeps no play count the catalog endpoints hand out
            playcounts: false,
            ..Capabilities::ALL
        },
    }
}

impl Default for NeteaseProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for NeteaseProvider {
    fn name(&self) -> &'static str {
        // the service's own name, as the lyrics source beside it is named for the site
        "NetEase Cloud Music"
    }

    fn slug(&self) -> &'static str {
        "netease"
    }

    fn reach(&self) -> Option<String> {
        Some(COOKIE_DOMAIN.to_owned())
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Secret]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn public_art(&self) -> bool {
        // covers live on p*.music.126.net, open to anyone with the url
        true
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        Self::restore_stored().await
    }

    async fn sign_in(
        &self,
        method: SignIn,
        prompt: PromptSink,
        mut input: InputSource,
    ) -> Result<ProviderSession> {
        match method {
            SignIn::Secret | SignIn::Default => {
                prompt(SignInPrompt::Secret);
                let cookies = input.recv().await.context("sign-in was cancelled")?;
                Self::connect(&cookies).await
            }
            SignIn::Anonymous => Err(anyhow::anyhow!("netease has no anonymous sign-in")),
            SignIn::Path(_) => Err(anyhow::anyhow!(
                "netease does not sign in with a folder path"
            )),
            SignIn::Credentials { .. } => Err(anyhow::anyhow!(
                "netease does not sign in with a server address"
            )),
        }
    }

    fn sign_out(&self) {
        auth::forget();
    }

    fn web_sign_in(&self) -> Option<WebSignIn> {
        Some(WebSignIn {
            url: SIGN_IN_URL,
            landing: LANDING,
            domain: COOKIE_DOMAIN,
            proof: auth::PROOF,
            agent: None,
        })
    }
}
