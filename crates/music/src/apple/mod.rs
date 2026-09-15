//! Apple Music: a proof of concept that a subscription track can reach Sonora's own output.
//!
//! The account arrives as one cookie, `media-user-token`, from the app's own sign-in window, or
//! from `SONORA_APPLE_MEDIA_USER_TOKEN` for a headless run. Playback resolves the track through
//! the web player's own endpoints, asks the system Widevine CDM that `SONORA_WIDEVINE_CDM`
//! names for a license, and decrypts each CENC sample as the decoder reaches it. What comes out
//! is an ordinary clear fMP4, so from `rodio` onwards this is the same path as every other
//! provider: the equalizer, the volume ramp, the spectrum tap, cpal.
//!
//! The library, search, albums, artists, playlists, stations and what Apple recommends all come
//! from the same endpoints the web player calls, described in [`client`]. One thing is missing
//! rather than unfinished: lyrics, which the token read off the page has no permission for.

mod auth;
mod client;
mod playback;
mod progressive;
mod stream;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::apple::playback::Factory;
use crate::{
    Capabilities, InputSource, MusicApi as _, MusicProvider, PromptSink, ProviderSession, Shape,
    SignIn, SignInPrompt, WebSignIn,
};

pub use client::AppleClient;
pub use progressive::Media;

/// The account token a run without a window uses: whatever `SONORA_APPLE_MEDIA_USER_TOKEN`
/// names, or the one the sign-in window stored.
pub fn account() -> Option<String> {
    auth::load()
}

/// The page the sign-in window opens. Apple finishes on music.apple.com with the account
/// cookie set, which is the one thing this needs.
const SIGN_IN_URL: &str = "https://music.apple.com/login";
/// Where the window ends up, and what cookie reads are scoped to.
const LANDING: &str = "music.apple.com";
/// The cookie filter runs on bare domains, so no leading dot. The account cookie is set on
/// `music.apple.com`, which this covers.
const COOKIE_DOMAIN: &str = "apple.com";

pub struct AppleProvider;

impl AppleProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(secret: &str) -> Result<ProviderSession> {
        let token = auth::user_token(secret)?;
        let client = AppleClient::connect(&token)
            .await
            .context("cannot reach apple music with that account")?;
        let profile = client.profile().await?;
        auth::store(&token)?;
        Ok(session(client, profile))
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(token) = auth::load() else {
            return Ok(None);
        };
        match AppleClient::connect(&token).await {
            Ok(client) => {
                let profile = client.profile().await?;
                Ok(Some(session(client, profile)))
            }
            Err(error) => {
                log::warn!("apple: the stored account is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

fn session(client: AppleClient, profile: crate::UserProfile) -> ProviderSession {
    if !widevine::available() {
        // Metadata still works, so the account is worth keeping; only playback will refuse.
        log::warn!("apple: no widevine module is here yet, so tracks cannot be decrypted");
    }
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Saved,
        authenticated: true,
        // Apple keeps no play counts and has no followed artists at all: a library artist is
        // one whose music you added. Stations it does have, through the same endpoint the web
        // player's autoplay uses.
        capabilities: Capabilities {
            radio: true,
            ..Capabilities::NONE
        },
    }
}

impl Default for AppleProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for AppleProvider {
    fn name(&self) -> &'static str {
        "Apple Music"
    }

    fn slug(&self) -> &'static str {
        "apple"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Secret]
    }

    fn stored(&self) -> bool {
        auth::stored()
    }

    fn public_art(&self) -> bool {
        // artwork lives on mzstatic, open to anyone with the url
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
            SignIn::Anonymous => anyhow::bail!("apple music has no anonymous sign-in"),
            SignIn::Path(_) => anyhow::bail!("apple music does not sign in with a folder path"),
            SignIn::Credentials { .. } => {
                anyhow::bail!("apple music does not sign in with a server address")
            }
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
            // Apple's sign-in page is happy with whatever the backend's own engine says.
            agent: None,
        })
    }
}
