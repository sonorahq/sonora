//! Deezer: sign-in with the browser's `arl` cookie, catalog over the public REST api, and
//! account data and stream urls over the web gateway the official player uses.

mod auth;
mod client;
mod decrypt;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::deezer::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PromptSink, ProviderSession, Shape,
    SignIn, SignInPrompt, WebSignIn,
};

pub use auth::arl;
pub use client::DeezerClient;
pub use decrypt::Striped;

/// A Deezer track downloading and decrypting as it plays.
pub type Stream = crate::stream::Stream<Striped>;

/// The page the sign-in window opens.
const SIGN_IN_URL: &str = "https://www.deezer.com/login";
/// Once the cookies land, the window navigates here on its own.
const LANDING: &str = "www.deezer.com";
/// The cookie filter runs on bare domains (see `webview`'s `matches`), so no leading dot.
const COOKIE_DOMAIN: &str = "deezer.com";
/// What a WebKit engine says about itself; consistent with the webview's actual engine.
const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";

pub struct DeezerProvider;

impl DeezerProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(arl: &str) -> Result<ProviderSession> {
        let arl = auth::arl(arl)?;
        let client = DeezerClient::connect(&arl)
            .await
            .context("cannot reach deezer with that session")?;
        let profile = client.profile().await?;
        auth::store(&auth::Credentials { arl })?;
        Ok(session(client, profile))
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(remembered) = auth::load() else {
            return Ok(None);
        };
        match DeezerClient::connect(&remembered.arl).await {
            Ok(client) => {
                let profile = client.profile().await?;
                Ok(Some(session(client, profile)))
            }
            Err(error) => {
                log::warn!("deezer: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

fn session(client: DeezerClient, profile: crate::UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Saved,
        authenticated: true,
        capabilities: Capabilities {
            playcounts: false,
            ..Capabilities::ALL
        },
    }
}

impl Default for DeezerProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for DeezerProvider {
    fn name(&self) -> &'static str {
        "Deezer"
    }

    fn slug(&self) -> &'static str {
        "deezer"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Secret]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn public_art(&self) -> bool {
        // covers live on cdn-images.dzcdn.net, open to anyone with the url
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
            SignIn::Anonymous => Err(anyhow::anyhow!("deezer has no anonymous sign-in")),
            SignIn::Path(_) => Err(anyhow::anyhow!(
                "deezer does not sign in with a folder path"
            )),
            SignIn::Credentials { .. } => Err(anyhow::anyhow!(
                "deezer does not sign in with a server address"
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
            // Deezer's anti-bot rejects the default Firefox agent on the WebKit window the
            // Linux backend drives; the Safari string matches the engine it actually runs.
            agent: Some(AGENT),
        })
    }
}
