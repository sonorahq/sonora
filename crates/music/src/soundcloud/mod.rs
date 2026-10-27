//! SoundCloud: sign-in with the browser's `oauth_token` cookie, catalog and account data over
//! the internal `api-v2.soundcloud.com`, and playback as progressive MP3 through Sonora's own
//! engine, so the equalizer, normalisation and the gapless join apply as to any other provider.

mod auth;
mod client;
mod hls;
mod links;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::soundcloud::client::SoundCloudClient;
use crate::soundcloud::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PlaybackFactory, PromptSink,
    ProviderSession, Shape, SignIn, SignInPrompt, WebSignIn,
};

/// The page the sign-in window opens.
const SIGN_IN_URL: &str = "https://soundcloud.com/signin";
/// Once the cookies land, the window navigates here on its own.
const LANDING: &str = "soundcloud.com";
/// The cookie filter runs on bare domains (see `webview`'s `matches`), so no leading dot.
const COOKIE_DOMAIN: &str = "soundcloud.com";

pub struct SoundCloudProvider;

impl SoundCloudProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(token: &str) -> Result<ProviderSession> {
        let token = auth::token(token)?;
        let client = SoundCloudClient::connect(&token)
            .await
            .context("cannot reach soundcloud with that session")?;
        let profile = client.profile().await?;
        auth::store(&auth::Credentials { token })?;
        Ok(session(client, profile))
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(remembered) = auth::load() else {
            return Ok(None);
        };
        match SoundCloudClient::connect(&remembered.token).await {
            Ok(client) => {
                let profile = client.profile().await?;
                Ok(Some(session(client, profile)))
            }
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) => {
                log::warn!("soundcloud: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

fn session(client: SoundCloudClient, profile: crate::UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Saved,
        authenticated: true,
        capabilities: Capabilities {
            qualities: client::QUALITIES,
            ..Capabilities::ALL
        },
    }
}

impl Default for SoundCloudProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for SoundCloudProvider {
    fn name(&self) -> &'static str {
        "SoundCloud"
    }

    fn slug(&self) -> &'static str {
        "soundcloud"
    }

    fn link(&self, link: &str) -> Option<crate::link::Linked> {
        links::read(link)
    }

    fn reach(&self) -> Option<String> {
        Some("api-v2.soundcloud.com".to_owned())
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Secret]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn public_art(&self) -> bool {
        // covers live on i1.sndcdn.net, open to anyone with the url
        true
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let remembered = auth::load()?;
        let client = SoundCloudClient::offline(&remembered.token)
            .inspect_err(|error| {
                log::warn!("soundcloud: cannot build an offline client: {error:#}")
            })
            .ok()?;
        Some(Arc::new(Factory::new(client)))
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
            SignIn::Anonymous => Err(anyhow::anyhow!("soundcloud has no anonymous sign-in")),
            SignIn::Path(_) => Err(anyhow::anyhow!(
                "soundcloud does not sign in with a folder path"
            )),
            SignIn::Credentials { .. } => Err(anyhow::anyhow!(
                "soundcloud does not sign in with a server address"
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
