//! Yandex Music: sign-in with an OAuth token, the catalogue, the account's likes and playlists
//! over the mobile api the Android app uses, and files that arrive AES-encrypted.

mod auth;
mod client;
mod decrypt;
mod links;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PlaybackFactory, PromptSink,
    ProviderSession, Shape, SignIn, SignInPrompt,
};

pub use auth::{TOKEN_PAGE, token};
pub use client::YandexClient;
pub use decrypt::Sealed;
pub use playback::Factory;

/// A Yandex track downloading and decrypting as it plays.
pub type Stream = crate::stream::Stream<Sealed>;

pub struct YandexProvider;

impl YandexProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(input: &str) -> Result<ProviderSession> {
        let token = auth::token(input)?;
        let client = YandexClient::connect(&token)
            .await
            .context("cannot reach yandex music with that token")?;
        let profile = client.profile().await?;
        auth::store(&auth::Credentials { token })?;
        Ok(session(client, profile))
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(remembered) = auth::load() else {
            return Ok(None);
        };
        match YandexClient::connect(&remembered.token).await {
            Ok(client) => {
                let profile = client.profile().await?;
                Ok(Some(session(client, profile)))
            }
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) => {
                log::warn!("yandex: the stored token is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

fn session(client: YandexClient, profile: crate::UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Saved,
        authenticated: true,
        capabilities: Capabilities {
            playcounts: false,
            qualities: client::QUALITIES,
            ..Capabilities::ALL
        },
    }
}

impl Default for YandexProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for YandexProvider {
    fn name(&self) -> &'static str {
        "Yandex Music"
    }

    fn slug(&self) -> &'static str {
        "yandex"
    }

    fn link(&self, link: &str) -> Option<crate::link::Linked> {
        links::read(link)
    }

    fn reach(&self) -> Option<String> {
        Some("api.music.yandex.net".to_owned())
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Secret]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn public_art(&self) -> bool {
        // covers live on avatars.yandex.net, open to anyone with the url
        true
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let remembered = auth::load()?;
        let client = YandexClient::offline(&remembered.token)
            .inspect_err(|error| log::warn!("yandex: cannot build an offline client: {error:#}"))
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
                let pasted = input.recv().await.context("sign-in was cancelled")?;
                Self::connect(&pasted).await
            }
            SignIn::Anonymous => Err(anyhow::anyhow!("yandex music has no anonymous sign-in")),
            SignIn::Path(_) => Err(anyhow::anyhow!(
                "yandex music does not sign in with a folder path"
            )),
            SignIn::Credentials { .. } => Err(anyhow::anyhow!(
                "yandex music does not sign in with a server address"
            )),
        }
    }

    fn sign_out(&self) {
        auth::forget();
    }
}
