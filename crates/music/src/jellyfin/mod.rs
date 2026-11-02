//! A Jellyfin server as a music source: browse, search and play the user's own library.
//!
//! The first version only reads. Favorites, playlist changes, play reporting, stations and
//! downloads are left out, and their capabilities are off so the app offers none of them.

pub(crate) mod auth;
mod client;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

pub use client::JellyfinClient;

use crate::jellyfin::playback::Factory;
use crate::{
    Capabilities, Catalog, MusicApi as _, MusicProvider, ProviderSession, Shape, SignIn,
    SignInFailure, SignInProblem,
};

/// The name the provider goes by. It is saved in settings, so it must not change.
pub(crate) const NAME: &str = "Jellyfin";

/// What the first version offers: listing and playing, with nothing that writes to the server
/// or keeps tracks on disk.
const CAPABILITIES: Capabilities = Capabilities::NONE;

pub struct JellyfinProvider;

impl JellyfinProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(
        server: String,
        username: String,
        password: String,
    ) -> Result<ProviderSession> {
        let server = auth::normalize_server(&server)?;
        let credentials = client::authenticate(
            &server,
            username.trim(),
            &password,
            &auth::device_name(),
            &auth::device_id(),
        )
        .await?;
        let client = JellyfinClient::new(&credentials);
        let profile = client.profile().await?;
        auth::store(&credentials)?;
        Ok(session(client, profile))
    }

    /// Opens the kept sign-in. A server that is only out of reach keeps the account, and one
    /// that refuses the token asks for the sign-in again.
    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(credentials) = auth::load() else {
            return Ok(None);
        };
        let client = JellyfinClient::new(&credentials);
        match client.profile().await {
            Ok(profile) => Ok(Some(session(client, profile))),
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) if error.downcast_ref::<SignInFailure>().is_some() => {
                log::warn!("jellyfin: the server refused the stored session");
                Err(SignInFailure(SignInProblem::Credentials).into())
            }
            Err(error) => {
                log::warn!("jellyfin: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

impl Default for JellyfinProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for JellyfinProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    fn slug(&self) -> &'static str {
        "jellyfin"
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
            anyhow::bail!("jellyfin signs in with a server address, username and password")
        };
        if username.trim().is_empty() {
            anyhow::bail!("the jellyfin username is empty");
        }
        Self::connect(server, username, password).await
    }

    fn sign_out(&self) {
        auth::forget();
    }
}

fn session(client: JellyfinClient, profile: crate::UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Catalog(Catalog::Fixed),
        authenticated: true,
        capabilities: CAPABILITIES,
    }
}
