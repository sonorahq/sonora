pub(crate) mod auth;
mod client;
mod lyrics;
mod playback;
mod tv;
mod wire;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

pub use client::PlexClient;
pub use lyrics::PlexLyrics;

use crate::plex::auth::{Credentials, Section, Server, User};
use crate::plex::playback::Factory;
use crate::plex::wire::Resource;
use crate::{
    AccountChoice, Capabilities, Catalog, Choosing, InputSource, MusicApi as _, MusicProvider,
    PlaybackFactory, PromptSink, ProviderSession, Shape, SignIn, SignInFailure, SignInProblem,
    SignInPrompt,
};

/// The name the provider and its lyrics source go by. It is saved in settings and in the lyrics
/// cache, so it must not change.
pub(crate) const NAME: &str = "Plex";

/// How often the sign-in pin is asked whether the listener has approved it yet.
const POLL: Duration = Duration::from_millis(1500);
/// How long the listener has to approve the pin. Plex drops an unapproved one after this.
const PATIENCE: Duration = Duration::from_secs(15 * 60);
/// What Plex calls a music library.
const MUSIC: &str = "artist";

pub struct PlexProvider;

impl PlexProvider {
    pub fn new() -> Self {
        Self
    }

    /// Signs in to plex.tv in the listener's browser window, then settles on one server and one
    /// music library, asking only when the account has more than one of either.
    async fn link(prompt: &PromptSink, input: &mut InputSource) -> Result<ProviderSession> {
        let http = reqwest::Client::new();
        let client_id = auth::client_id()?;

        let pin = tv::create_pin(&http, &client_id).await?;
        prompt(SignInPrompt::Window(tv::approval_url(&client_id, &pin)));
        let token = tokio::time::timeout(PATIENCE, async {
            loop {
                if let Some(token) = tv::claimed(&http, &client_id, &pin).await? {
                    return Ok::<_, anyhow::Error>(token);
                }
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .map_err(|_| SignInFailure(SignInProblem::Cancelled))??;

        let account = tv::account(&http, &client_id, &token).await?;
        let mut servers = tv::servers(&http, &client_id, &token).await?;
        if servers.is_empty() {
            anyhow::bail!("this plex account has no media server");
        }
        let server = match servers.len() {
            1 => servers.remove(0),
            _ => {
                let choices = servers.iter().map(server_choice).collect();
                let picked = pick(prompt, input, Choosing::Server, choices).await?;
                servers
                    .into_iter()
                    .find(|server| server.client_identifier == picked)
                    .context("that plex server is not on the account")?
            }
        };

        let server_token = server.access_token.clone().unwrap_or_else(|| token.clone());
        let uri = tv::reach(&http, &client_id, &server_token, &server.connections).await?;
        let mut stored = Credentials {
            token,
            server: Server {
                id: server.client_identifier.clone(),
                name: server.name.clone(),
                token: server_token,
                uri,
            },
            section: Section {
                key: String::new(),
                title: String::new(),
            },
            user: User {
                id: account.id.to_string(),
                name: match account.title.is_empty() {
                    true => account.username,
                    false => account.title,
                },
                avatar: account.thumb,
            },
        };

        stored.section = Self::section(&http, &client_id, &stored.server, prompt, input).await?;
        let client = PlexClient::new(stored.clone(), client_id)?;
        let profile = client
            .profile()
            .await
            .context("cannot reach the plex server")?;
        auth::store(&stored)?;
        Ok(session(client, profile))
    }

    /// The music library to show: the only one the server has, or the one the listener picks.
    async fn section(
        http: &reqwest::Client,
        client_id: &str,
        server: &Server,
        prompt: &PromptSink,
        input: &mut InputSource,
    ) -> Result<Section> {
        let mut headers = tv::identity(client_id);
        headers.insert("x-plex-token", server.token.parse()?);
        let listed: wire::Envelope = http
            .get(format!("{}/library/sections", server.uri))
            .headers(headers)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .context("cannot reach the plex server")?
            .error_for_status()
            .context("the plex server refused to list its libraries")?
            .json()
            .await
            .context("cannot read the plex libraries")?;
        let mut music: Vec<Section> = listed
            .container
            .directory
            .into_iter()
            .filter(|directory| directory.kind == MUSIC)
            .map(|directory| Section {
                key: directory.key,
                title: directory.title,
            })
            .collect();
        match music.len() {
            0 => anyhow::bail!("the plex server has no music library"),
            1 => Ok(music.remove(0)),
            _ => {
                let choices = music
                    .iter()
                    .map(|section| AccountChoice {
                        id: section.key.clone(),
                        name: section.title.clone(),
                        detail: None,
                    })
                    .collect();
                let picked = pick(prompt, input, Choosing::Library, choices).await?;
                music
                    .into_iter()
                    .find(|section| section.key == picked)
                    .context("that plex library is not on the server")
            }
        }
    }

    async fn restore_stored() -> Result<Option<ProviderSession>> {
        let Some(stored) = auth::load() else {
            return Ok(None);
        };
        let client = PlexClient::new(stored, auth::client_id()?)?;
        match client.profile().await {
            Ok(profile) => Ok(Some(session(client, profile))),
            Err(error) if crate::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) if error.chain().any(|cause| cause.is::<SignInFailure>()) => {
                log::warn!("plex: the stored sign-in was refused: {error:#}");
                auth::forget();
                Err(error)
            }
            Err(error) => {
                log::warn!("plex: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

impl Default for PlexProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for PlexProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    fn slug(&self) -> &'static str {
        "plex"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Default]
    }

    fn stored(&self) -> bool {
        auth::load().is_some()
    }

    fn location(&self) -> Option<String> {
        auth::load().map(|stored| stored.server.name)
    }

    /// The host of the server's last address, since a self-hosted library has no address in common
    /// with anyone else's.
    fn reach(&self) -> Option<String> {
        let uri = auth::load()?.server.uri;
        let host = uri.split_once("://").map_or(uri.as_str(), |(_, rest)| rest);
        let host = host.split(['/', ':', '?']).next()?;
        (!host.is_empty()).then(|| host.to_owned())
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let stored = auth::load()?;
        let client = auth::client_id()
            .and_then(|id| PlexClient::new(stored, id))
            .inspect_err(|error| log::warn!("plex: cannot build an offline client: {error:#}"))
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
        if !matches!(method, SignIn::Default) {
            anyhow::bail!("plex signs in through plex.tv");
        }
        Self::link(&prompt, &mut input).await
    }

    fn sign_out(&self) {
        auth::forget();
    }
}

fn session(client: PlexClient, profile: crate::UserProfile) -> ProviderSession {
    ProviderSession {
        profile,
        api: Arc::new(client.clone()),
        playback: Arc::new(Factory::new(client)),
        shape: Shape::Catalog(Catalog::Fixed),
        authenticated: true,
        capabilities: Capabilities {
            playlist_visibility: false,
            ..Capabilities::ALL
        },
    }
}

fn server_choice(server: &Resource) -> AccountChoice {
    AccountChoice {
        id: server.client_identifier.clone(),
        name: server.name.clone(),
        detail: match (server.owned, &server.source_title) {
            (false, Some(owner)) => Some(owner.clone()),
            _ => None,
        },
    }
}

/// Puts a choice to the listener and answers with the id they picked.
async fn pick(
    prompt: &PromptSink,
    input: &mut InputSource,
    what: Choosing,
    choices: Vec<AccountChoice>,
) -> Result<String> {
    prompt(SignInPrompt::Accounts(what, choices));
    input.recv().await.context("sign-in was cancelled")
}
