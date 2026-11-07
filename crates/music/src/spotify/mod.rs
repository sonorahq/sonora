pub mod auth;

mod albums;
mod artists;
mod client;
mod collection;
mod collection2;
mod connect;
mod links;
mod lyrics;
mod offline;
mod pace;
mod pathfinder;
mod pb;
mod playback;
mod playlists;
mod profiles;
mod radio;
mod search;
mod session;
mod sink;
mod web;
mod webplayback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::spotify::playback::Factory;
use crate::{
    Capabilities, MusicApi as _, MusicProvider, PlaybackFactory, ProviderSession, Shape,
    SignInFailure, SignInProblem,
};

pub use auth::AuthConfig;
pub use client::LibrespotClient;
pub use lyrics::SpotifyLyrics;

pub struct SpotifyProvider {
    config: AuthConfig,
}

impl SpotifyProvider {
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        Self::new(AuthConfig::from_env())
    }

    fn drop_free(&self, error: anyhow::Error) -> anyhow::Error {
        if matches!(
            error.downcast_ref::<SignInFailure>(),
            Some(SignInFailure(SignInProblem::Premium))
        ) {
            auth::forget(&self.config);
        }
        error
    }

    async fn session(
        &self,
        client: LibrespotClient,
        web: Option<web::Web>,
    ) -> Result<ProviderSession> {
        let profile = client.profile().await?;
        let cookie = web.is_some();
        let playback: Arc<dyn PlaybackFactory> = match web {
            Some(web) => Arc::new(webplayback::Factory(web)),
            None => Arc::new(Factory::new(client.session().native().clone())),
        };
        Ok(ProviderSession {
            profile,
            api: Arc::new(client),
            playback,
            shape: Shape::Saved,
            authenticated: true,
            capabilities: Capabilities {
                pins: true,
                playlist_covers: true,
                download_gap: playback::DOWNLOAD_GAP,
                qualities: if cookie {
                    webplayback::QUALITIES
                } else {
                    playback::QUALITIES
                },
                keeps: !cookie,
                downloads: !cookie,
                ..Capabilities::ALL
            },
        })
    }
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    fn name(&self) -> &'static str {
        "Spotify"
    }

    fn slug(&self) -> &'static str {
        "spotify"
    }

    fn reach(&self) -> Option<String> {
        Some("apresolve.spotify.com".to_owned())
    }

    fn public_art(&self) -> bool {
        true
    }

    fn link(&self, link: &str) -> Option<crate::link::Linked> {
        links::read(link)
    }

    fn sign_in_options(&self) -> Vec<crate::SignIn> {
        vec![crate::SignIn::Default, crate::SignIn::Secret]
    }

    fn abandon(&self) {
        auth::release(&self.config);
    }

    fn stored(&self) -> bool {
        self.config.file().exists() || web::Web::stored(&self.config)
    }

    fn protected(&self) -> bool {
        web::Web::stored(&self.config)
    }

    /// A session that never connects, which plays downloads and nothing else. It is built on
    /// the caller's tokio runtime.
    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        if !self.stored() || web::Web::stored(&self.config) {
            return None;
        }
        let session = auth::session(&self.config)
            .inspect_err(|error| log::warn!("spotify: cannot open an offline session: {error:#}"))
            .ok()?;
        Some(Arc::new(Factory::new(session)))
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        if web::Web::stored(&self.config) {
            let web = web::Web::open(&self.config, &web::Web::read(&self.config)?).await?;
            let client =
                LibrespotClient::new(session::Session::cookie(web.session().await?, web.clone()));
            return self.session(client, Some(web)).await.map(Some);
        }
        let Some(session) = auth::restore(&self.config)
            .await
            .map_err(|error| self.drop_free(error))?
        else {
            return Ok(None);
        };
        self.session(LibrespotClient::new(session), None)
            .await
            .map(Some)
    }

    async fn sign_in(
        &self,
        method: crate::SignIn,
        prompt: crate::PromptSink,
        mut input: crate::InputSource,
    ) -> Result<ProviderSession> {
        if method == crate::SignIn::Secret {
            prompt(crate::SignInPrompt::Secret);
            let cookie = input.recv().await.context("sign-in was cancelled")?;
            let web = web::Web::open(&self.config, &cookie).await?;
            let client =
                LibrespotClient::new(session::Session::cookie(web.session().await?, web.clone()));
            let session = self.session(client, Some(web.clone())).await?;
            web.save(&self.config)?;
            auth::forget(&self.config);
            return Ok(session);
        }
        let session = auth::login(&self.config, prompt)
            .await
            .map_err(|error| self.drop_free(error))?;
        let session = self.session(LibrespotClient::new(session), None).await?;
        web::Web::forget(&self.config);
        Ok(session)
    }

    fn sign_out(&self) {
        auth::forget(&self.config);
        web::Web::forget(&self.config);
    }

    fn web_sign_in(&self) -> Option<crate::WebSignIn> {
        Some(crate::WebSignIn {
            url: "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
            landing: "open.spotify.com",
            domain: "spotify.com",
            proof: &["sp_dc"],
            agent: None,
        })
    }
}
