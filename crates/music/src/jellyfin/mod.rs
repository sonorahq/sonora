//! A Jellyfin server as a music source: the user's own library to browse, search, play, keep
//! and edit, with favorites, playlists, instant mixes and the server's lyrics.

pub(crate) mod auth;
mod client;
mod lyrics;
mod playback;
mod remote;
mod session;
mod wire;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use reqwest::header::AUTHORIZATION;

pub use client::JellyfinClient;
pub use lyrics::JellyfinLyrics;

use crate::jellyfin::playback::Factory;
use crate::{
    Capabilities, Catalog, MusicApi as _, MusicProvider, PlaybackFactory, PromptSink,
    ProviderSession, RadioSeed, Shape, SignIn, SignInFailure, SignInProblem, SignInPrompt,
};

/// The name the provider and its lyrics source go by. It is saved in settings and in the lyrics
/// cache, so it must not change.
pub(crate) const NAME: &str = "Jellyfin";

/// Everything a streaming service offers, with the server converting on request and keeping
/// playlist pictures of its own, downloads as plain files and an instant mix of any collection.
const CAPABILITIES: Capabilities = Capabilities {
    collection_radio: &[RadioSeed::Album, RadioSeed::Playlist, RadioSeed::Artist],
    transcode: true,
    playlist_covers: true,
    plain_downloads: true,
    ..Capabilities::ALL
};
/// How often a Quick Connect sign-in asks whether the code was approved, and for how long.
const QUICK_CONNECT_POLL: Duration = Duration::from_secs(3);
const QUICK_CONNECT_PATIENCE: Duration = Duration::from_secs(5 * 60);
/// How long a sign-out waits for the server to revoke the token.
const LOG_OUT_PATIENCE: Duration = Duration::from_secs(5);

pub struct JellyfinProvider;

impl JellyfinProvider {
    pub fn new() -> Self {
        Self
    }

    async fn connect(
        server: String,
        username: String,
        password: String,
        prompt: PromptSink,
    ) -> Result<ProviderSession> {
        let server = auth::normalize_server(&server)?;
        let device = auth::device_name();
        let device_id = auth::device_id();
        let credentials = match username.trim().is_empty() && password.is_empty() {
            true => quick_connect(&server, &device, &device_id, prompt).await?,
            false => {
                client::authenticate(&server, username.trim(), &password, &device, &device_id)
                    .await?
            }
        };
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

    /// Kept tracks are plain files, so they play from the stored sign-in alone.
    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let credentials = auth::load()?;
        Some(Arc::new(Factory::new(JellyfinClient::new(&credentials))))
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        Self::restore_stored().await
    }

    /// Signs in with a username and password, or by Quick Connect when both are left empty:
    /// the server hands out a code that the user approves from a Jellyfin app already signed in.
    async fn sign_in(
        &self,
        method: SignIn,
        prompt: PromptSink,
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
        if username.trim().is_empty() && !password.is_empty() {
            anyhow::bail!("the jellyfin username is empty");
        }
        Self::connect(server, username, password, prompt).await
    }

    /// Forgets the sign-in and, on a thread of its own, tells the server to revoke the token, so
    /// the device leaves the server's device list. A server out of reach keeps the token until
    /// it expires there.
    fn sign_out(&self) {
        if let Some(credentials) = auth::load() {
            std::thread::spawn(move || log_out(credentials));
        }
        auth::forget();
    }
}

/// Revokes the token by `POST /Sessions/Logout`. It runs outside the app's runtime, since a
/// sign-out comes from the interface thread, and gives up after a few seconds.
fn log_out(credentials: auth::Credentials) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::warn!("jellyfin: cannot start the sign-out: {error}");
            return;
        }
    };
    let client = JellyfinClient::new(&credentials);
    let sent = runtime.block_on(async {
        tokio::time::timeout(LOG_OUT_PATIENCE, async {
            client
                .http
                .post(format!("{}/Sessions/Logout", credentials.server))
                .header(AUTHORIZATION, &client.header)
                .send()
                .await
        })
        .await
    });
    match sent {
        Ok(Ok(response)) => log::info!(
            "jellyfin: the server ended the session, {}",
            response.status()
        ),
        Ok(Err(error)) => log::warn!("jellyfin: cannot end the session on the server: {error}"),
        Err(_) => log::warn!("jellyfin: the server did not answer the sign-out in time"),
    }
}

/// A signed-in session. Its client carries the remote control, whose worker lives as long as the
/// session's client does.
fn session(client: JellyfinClient, profile: crate::UserProfile) -> ProviderSession {
    let client = client.clone().with_remote(remote::start(client));
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Catalog(Catalog::Fixed),
        authenticated: true,
        capabilities: CAPABILITIES,
    }
}

/// Signs in by Quick Connect. The server hands out a code, the sign-in page shows it with the
/// server's address while the browser opens the server's Quick Connect page, and once the user
/// approves it from a signed-in Jellyfin app the secret buys an access token. Gives up after
/// five minutes.
async fn quick_connect(
    server: &str,
    device: &str,
    device_id: &str,
    prompt: PromptSink,
) -> Result<auth::Credentials> {
    let http = reqwest::Client::new();
    let header = auth::header(device, device_id, None);
    let enabled: bool = http
        .get(format!("{server}/QuickConnect/Enabled"))
        .header(AUTHORIZATION, &header)
        .send()
        .await
        .map_err(client::lost)?
        .json()
        .await
        .unwrap_or(false);
    if !enabled {
        anyhow::bail!("quick connect is turned off on this jellyfin server");
    }
    let started: wire::QuickConnect = http
        .post(format!("{server}/QuickConnect/Initiate"))
        .header(AUTHORIZATION, &header)
        .send()
        .await
        .map_err(client::lost)?
        .error_for_status()
        .context("the jellyfin server refused to start quick connect")?
        .json()
        .await
        .context("cannot read the quick connect code")?;
    let shown = server.split_once("://").map_or(server, |(_, rest)| rest);
    prompt(SignInPrompt::Code {
        code: started.code.clone(),
        url: shown.to_owned(),
    });
    open::that_in_background(format!("{server}/web/#/quickconnect"));
    let waited = tokio::time::timeout(QUICK_CONNECT_PATIENCE, async {
        loop {
            tokio::time::sleep(QUICK_CONNECT_POLL).await;
            let state: wire::QuickConnect = http
                .get(format!("{server}/QuickConnect/Connect"))
                .query(&[("secret", started.secret.as_str())])
                .header(AUTHORIZATION, &header)
                .send()
                .await
                .map_err(client::lost)?
                .error_for_status()
                .context("the jellyfin server forgot the quick connect code")?
                .json()
                .await
                .context("cannot read the quick connect state")?;
            if state.authenticated {
                return anyhow::Ok(());
            }
        }
    })
    .await;
    match waited {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("the quick connect code was not approved in time"),
    }
    let answer: wire::Authentication = http
        .post(format!("{server}/Users/AuthenticateWithQuickConnect"))
        .header(AUTHORIZATION, &header)
        .json(&serde_json::json!({ "Secret": started.secret }))
        .send()
        .await
        .map_err(client::lost)?
        .error_for_status()
        .context("the jellyfin server refused the approved quick connect code")?
        .json()
        .await
        .context("the quick connect sign-in answered something other than an account")?;
    client::account(server, answer, "", device_id)
}
