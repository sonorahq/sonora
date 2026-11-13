//! Tidal: sign-in through the device flow a TV app uses, catalog and account data over the
//! `api.tidal.com` the official players use, and playback at the best quality the account
//! holds, up to 24-bit FLAC.
//!
//! Lossless arrives as one FLAC file and hi-res as DASH segments of FLAC in fMP4. Both decode
//! in Sonora's own engine, so the equalizer, normalisation and the gapless join apply to Tidal
//! as to any other provider.

mod auth;
mod client;
mod links;
mod manifest;
mod playback;
mod wire;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::tidal::auth::{Credentials, Lane};
use crate::tidal::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PlaybackFactory, PromptSink,
    ProviderSession, Shape, SignIn, SignInFailure, SignInProblem,
};

pub use client::TidalClient;

/// Tidal as the app sees it before an account is chosen: how to sign in, and the session a
/// sign-in or a stored one opens.
pub struct TidalProvider;

impl TidalProvider {
    pub fn new() -> Self {
        Self
    }

    /// Opens the stored sign-in. When Tidal refuses it, the stored sign-in is forgotten and the
    /// refusal returned, so the listener is told once and the app stops trying it. A lost
    /// connection or Tidal being down is an error the app can try again from.
    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some((stored, generation)) = auth::restore() else {
            return Ok(None);
        };
        let first = stored.refresh_token.clone();
        let client = TidalClient::new(client::http()?, stored, Some(generation));
        let opened = match client.connect().await {
            Ok(()) => session(client.clone()).await,
            Err(error) => Err(error),
        };
        match opened {
            Ok(session) => Ok(Some(session)),
            Err(error) if auth::refused(&error) => {
                log::warn!("tidal: the stored session is no longer usable: {error:#}");
                let last = client.credentials().await.refresh_token;
                auth::abandon(generation, &[&first, &last]);
                Err(error)
            }
            // The app keeps a session it only could not reach, and tries again once the
            // network answers, which is the right answer for Tidal being down too.
            Err(error) if auth::unavailable(&error) => {
                Err(error.context(SignInFailure(SignInProblem::Network)))
            }
            Err(error) => Err(error),
        }
    }

    /// The device flow from the code to an open session. Nothing is stored until all of it has
    /// worked, so dropping this future part way leaves no sign-in behind. When Tidal refuses
    /// the session moved to the app client, the sign-in carries on as the device client.
    async fn sign_in_by_device(prompt: &PromptSink) -> Result<ProviderSession> {
        let http = client::http()?;
        let approved = auth::device(&http, prompt).await?;
        let upgraded = auth::upgrade(&http, approved.clone()).await;
        let moved = upgraded.lane == Lane::App;
        let client = match open(&http, upgraded).await {
            Ok(client) => client,
            Err(error) if moved && auth::refused(&error) => {
                log::warn!(
                    "tidal: the app client session was refused, signing in as the device client: {error:#}"
                );
                let device = Credentials {
                    lane: Lane::Refused,
                    ..approved
                };
                open(&http, device).await?
            }
            Err(error) => return Err(error),
        };
        let session = session(client.clone()).await?;
        client.stored_at(auth::store(&client.credentials().await)?);
        Ok(session)
    }
}

impl Default for TidalProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for TidalProvider {
    fn name(&self) -> &'static str {
        "Tidal"
    }

    fn slug(&self) -> &'static str {
        "tidal"
    }

    fn link(&self, link: &str) -> Option<crate::link::Linked> {
        links::read(link)
    }

    fn reach(&self) -> Option<String> {
        Some("api.tidal.com".to_owned())
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Default]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn public_art(&self) -> bool {
        // covers live on resources.tidal.com, open to anyone with the url
        true
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let (stored, generation) = auth::restore()?;
        let http = client::http()
            .inspect_err(|error| log::warn!("tidal: cannot build an offline client: {error:#}"))
            .ok()?;
        let client = TidalClient::new(http, stored, Some(generation));
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
            // The app cancels by closing `input`, which nothing is ever sent on.
            SignIn::Default => tokio::select! {
                signed_in = Self::sign_in_by_device(&prompt) => signed_in,
                () = closed(&mut input) => Err(SignInFailure(SignInProblem::Cancelled))
                    .context("the tidal sign-in was cancelled"),
            },
            SignIn::Secret | SignIn::Anonymous | SignIn::Path(_) | SignIn::Credentials { .. } => {
                Err(anyhow::anyhow!("tidal only signs in with a device code"))
            }
        }
    }

    fn sign_out(&self) {
        auth::forget();
    }
}

/// A new sign-in's session, checked with Tidal and holding nothing on disk yet.
async fn open(http: &reqwest::Client, credentials: Credentials) -> Result<TidalClient> {
    let client = TidalClient::new(http.clone(), credentials, None);
    client
        .connect()
        .await
        .context("cannot reach tidal with that sign-in")?;
    Ok(client)
}

async fn session(client: TidalClient) -> Result<ProviderSession> {
    let profile = client.profile().await?;
    Ok(ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Saved,
        authenticated: true,
        capabilities: Capabilities {
            playcounts: false,
            playlist_covers: true,
            qualities: client::QUALITIES,
            ..Capabilities::ALL
        },
    })
}

/// Waits until the app closes the sign-in's input, which is how it says the listener cancelled.
async fn closed(input: &mut InputSource) {
    while input.recv().await.is_some() {}
}

/// Locks a mutex whose guarded value is still sound after a panic elsewhere.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
