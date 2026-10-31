pub(crate) mod auth;
mod client;
mod lyrics;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

pub use client::SubsonicClient;
pub use lyrics::SubsonicLyrics;

use crate::subsonic::playback::Factory;
use crate::{
    Capabilities, Catalog, MusicApi as _, MusicProvider, PlaybackFactory, ProviderSession, Shape,
    SignIn,
};

/// The name the provider and its lyrics source go by. It is saved in settings and in the lyrics
/// cache, so it must not change.
pub(crate) const NAME: &str = "Subsonic";

pub struct SubsonicProvider;

impl SubsonicProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(
        server: String,
        username: String,
        password: String,
    ) -> Result<ProviderSession> {
        let server = auth::normalize_server(&server)?;
        let signature = auth::sign(&username, &password);
        let client = SubsonicClient::new(
            server.clone(),
            username.clone(),
            password.clone(),
            &signature,
        )?;
        let profile = client
            .profile()
            .await
            .context("cannot reach the subsonic server")?;
        let covers = client.takes_playlist_covers().await;
        auth::store(&auth::Credentials {
            server,
            username,
            password,
            signature: Some(signature),
        })?;
        Ok(ProviderSession {
            profile,
            api: Arc::new(client.clone()),
            playback: Arc::new(Factory::new(client)),
            shape: Shape::Catalog(Catalog::Fixed),
            authenticated: true,
            capabilities: Capabilities {
                transcode: true,
                playlist_covers: covers,
                ..Capabilities::ALL
            },
        })
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(mut remembered) = auth::load() else {
            return Ok(None);
        };
        if remembered.signature.is_none() {
            remembered.signature = Some(auth::sign(&remembered.username, &remembered.password));
            if let Err(error) = auth::store(&remembered) {
                log::warn!("subsonic: cannot keep the cover signature: {error:#}");
            }
        }
        let signature = remembered.signature.clone().unwrap_or_default();
        let client = SubsonicClient::new(
            remembered.server.clone(),
            remembered.username.clone(),
            remembered.password.clone(),
            &signature,
        )?;
        match client.profile().await {
            Ok(profile) => Ok(Some(ProviderSession {
                capabilities: Capabilities {
                    transcode: true,
                    playlist_covers: client.takes_playlist_covers().await,
                    ..Capabilities::ALL
                },
                profile,
                api: Arc::new(client.clone()),
                playback: Arc::new(Factory::new(client)),
                shape: Shape::Catalog(Catalog::Fixed),
                authenticated: true,
            })),
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) => {
                log::warn!("subsonic: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

impl Default for SubsonicProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for SubsonicProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    fn slug(&self) -> &'static str {
        "subsonic"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Credentials {
            server: String::new(),
            username: String::new(),
            password: String::new(),
        }]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn location(&self) -> Option<String> {
        auth::load().map(|credentials| credentials.server)
    }

    /// The configured server's host, since a self-hosted library has no address in common with
    /// anyone else's.
    fn reach(&self) -> Option<String> {
        let server = auth::load()?.server;
        let host = server
            .split_once("://")
            .map_or(server.as_str(), |(_, rest)| rest);
        let host = host.split(['/', ':', '?']).next()?;
        (!host.is_empty()).then(|| host.to_owned())
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let remembered = auth::load()?;
        let signature = remembered
            .signature
            .clone()
            .unwrap_or_else(|| auth::sign(&remembered.username, &remembered.password));
        let client = SubsonicClient::new(
            remembered.server,
            remembered.username,
            remembered.password,
            &signature,
        )
        .inspect_err(|error| log::warn!("subsonic: cannot build an offline client: {error:#}"))
        .ok()?;
        Some(Arc::new(Factory::new(client)))
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        Self::restore_stored().await
    }

    async fn sign_in(
        &self,
        method: SignIn,
        _prompt: crate::PromptSink,
        _input: crate::InputSource,
    ) -> Result<ProviderSession> {
        let SignIn::Credentials {
            server,
            username,
            password,
        } = method
        else {
            anyhow::bail!("subsonic signs in with a server address, username and password")
        };
        if username.trim().is_empty() {
            anyhow::bail!("the subsonic username is empty");
        }
        if password.is_empty() {
            anyhow::bail!("the subsonic password is empty");
        }
        Self::connect(server, username, password).await
    }

    fn sign_out(&self) {
        auth::forget();
    }
}
