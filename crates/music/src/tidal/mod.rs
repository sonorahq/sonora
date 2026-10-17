//! Tidal: sign-in through the device flow a TV app uses, catalog and account data over the
//! `api.tidal.com` the official players use, and playback at the best quality the account
//! holds, up to 24-bit FLAC.
//!
//! Lossless arrives as one FLAC file and hi-res as DASH segments of FLAC in fMP4. Both decode
//! in Sonora's own engine, so the equalizer, normalisation and the gapless join apply to Tidal
//! as to any other provider.

mod auth;
mod client;
mod manifest;
mod playback;
mod wire;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::tidal::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PromptSink, ProviderSession, Shape,
    SignIn, SignInFailure, SignInProblem,
};

pub use client::TidalClient;

/// Tidal as the app sees it before an account is chosen: how to sign in, and the session a
/// sign-in or a stored one opens.
pub struct TidalProvider;

impl TidalProvider {
    pub fn new() -> Self {
        Self
    }

    /// Opens the stored sign-in. It is given up on only when Tidal refuses it; a lost
    /// connection or Tidal being down is an error the app can try again from.
    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some((stored, generation)) = auth::restore() else {
            return Ok(None);
        };
        match TidalClient::connect(client::http()?, stored, generation).await {
            Ok(client) => Ok(Some(session(client).await?)),
            Err(error) if auth::refused(&error) => {
                log::warn!("tidal: the stored session is no longer usable: {error:#}");
                Ok(None)
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
    /// worked, so dropping this future part way leaves no sign-in behind.
    async fn sign_in_by_device(prompt: &PromptSink) -> Result<ProviderSession> {
        let http = client::http()?;
        let credentials = auth::device(&http, prompt).await?;
        let client = TidalClient::connect(http, credentials.clone(), auth::generation())
            .await
            .context("cannot reach tidal with that sign-in")?;
        let session = session(client).await?;
        auth::store(&credentials)?;
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
