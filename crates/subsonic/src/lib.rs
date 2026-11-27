//! A Subsonic or OpenSubsonic server, such as Navidrome, as a music source: the library and
//! its libraries, smart playlists, lyrics, and streams the server converts on request.

pub mod auth;
mod client;
mod lyrics;
mod playback;
mod wire;

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use music::{
    Capabilities, Catalog, Libraries, Library, MusicApi as _, MusicProvider, PlaybackFactory,
    ProviderSession, RadioSeed, Shape, SignIn, credentials,
};

use crate::playback::Factory;
pub use client::SubsonicClient;
pub use lyrics::SubsonicLyrics;

/// The name the provider and its lyrics source go by. It is saved in settings and in the lyrics
/// cache, so it must not change.
pub(crate) const NAME: &str = "Subsonic";
/// The slug of the provider and of its first account.
pub(crate) const SLUG: &str = "subsonic";

/// The collections `getSimilarSongs` takes the id of. A playlist is not one, so its radio is
/// blended from the radios of its songs.
const RADIO_SEEDS: &[RadioSeed] = &[RadioSeed::Album, RadioSeed::Artist];

/// One account on a Subsonic server. The first account goes by the provider's own slug, and
/// every further one by a slug of its own, which also names the folder its sign-in is kept in.
pub struct SubsonicProvider {
    slug: &'static str,
}

impl SubsonicProvider {
    pub fn new() -> Self {
        Self { slug: SLUG }
    }

    /// The account kept under `slug`, one of the provider's further accounts.
    fn slot(slug: &'static str) -> Self {
        Self { slug }
    }

    async fn connect(
        slug: &'static str,
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
            Vec::new(),
        )?;
        let profile = client
            .profile()
            .await
            .context("cannot reach the subsonic server")?;
        let covers = client.takes_playlist_covers().await;
        let smart = client.takes_smart_playlists().await;
        auth::store(
            slug,
            &auth::Credentials {
                server,
                username,
                password,
                signature: Some(signature),
                // a fresh sign-in starts on the whole server; libraries are chosen afterwards
                library: None,
                libraries: Vec::new(),
            },
        )?;
        Ok(ProviderSession {
            profile,
            api: Arc::new(client.clone()),
            playback: Arc::new(Factory::new(client)),
            shape: Shape::Catalog(Catalog::Fixed),
            authenticated: true,
            capabilities: Capabilities {
                collection_radio: RADIO_SEEDS,
                transcode: true,
                playlist_covers: covers,
                smart_playlists: smart,
                playlist_order: true,
                plain_downloads: true,
                ..Capabilities::ALL
            },
        })
    }

    async fn restore_stored(slug: &'static str) -> Result<Option<ProviderSession>> {
        let Some(mut remembered) = auth::load(slug) else {
            return Ok(None);
        };
        if remembered.signature.is_none() {
            remembered.signature = Some(auth::sign(&remembered.username, &remembered.password));
            if let Err(error) = auth::store(slug, &remembered) {
                log::warn!("subsonic: cannot keep the cover signature: {error:#}");
            }
        }
        let signature = remembered.signature.clone().unwrap_or_default();
        let client = SubsonicClient::new(
            remembered.server.clone(),
            remembered.username.clone(),
            remembered.password.clone(),
            &signature,
            remembered
                .libraries
                .iter()
                .map(|library| library.id)
                .collect(),
        )?;
        match client.profile().await {
            Ok(profile) => Ok(Some(ProviderSession {
                capabilities: Capabilities {
                    collection_radio: RADIO_SEEDS,
                    transcode: true,
                    playlist_covers: client.takes_playlist_covers().await,
                    smart_playlists: client.takes_smart_playlists().await,
                    playlist_order: true,
                    plain_downloads: true,
                    ..Capabilities::ALL
                },
                profile,
                api: Arc::new(client.clone()),
                playback: Arc::new(Factory::new(client)),
                shape: Shape::Catalog(Catalog::Fixed),
                authenticated: true,
            })),
            Err(error) if music::trouble::offline(&format!("{error:#}")) => Err(error),
            Err(error) => {
                log::warn!("subsonic: the stored session is no longer usable: {error:#}");
                Ok(None)
            }
        }
    }
}

/// The libraries the stored account reads, empty when it reads the whole server. The picker
/// names the choice from this before the server is asked anything.
fn selected_libraries(slug: &str) -> Vec<Library> {
    auth::load(slug).map_or_else(Vec::new, |remembered| remembered.libraries)
}

/// Every library the stored server offers the account, for the settings picker, and whether
/// the server reads several of them at once.
async fn libraries(slug: &str) -> Result<Libraries> {
    let remembered = auth::load(slug).context("no subsonic account is stored")?;
    let signature = remembered
        .signature
        .clone()
        .unwrap_or_else(|| auth::sign(&remembered.username, &remembered.password));
    let client = SubsonicClient::new(
        remembered.server,
        remembered.username,
        remembered.password,
        &signature,
        Vec::new(),
    )?;
    let (all, several) = tokio::join!(client.music_folders(), client.reads_several_folders());
    Ok(Libraries { all: all?, several })
}

/// Points the stored account at some libraries, or back at the whole server when `libraries`
/// is empty. This only persists the choice: the live session rebuilds itself to apply it, so no
/// request in flight changes scope under the user.
fn select_libraries(slug: &str, libraries: Vec<Library>) -> Result<()> {
    let mut remembered = auth::load(slug).context("no subsonic account is stored")?;
    remembered.libraries = libraries;
    auth::store(slug, &remembered)
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
        self.slug
    }

    fn keeps_several(&self) -> bool {
        true
    }

    fn spare(&self) -> Option<Arc<dyn MusicProvider>> {
        Some(Arc::new(Self::slot(credentials::fresh(SLUG))))
    }

    fn siblings(&self) -> Vec<Arc<dyn MusicProvider>> {
        match self.slug == SLUG {
            true => credentials::slots(SLUG)
                .into_iter()
                .map(|slug| Arc::new(Self::slot(slug)) as Arc<dyn MusicProvider>)
                .collect(),
            false => Vec::new(),
        }
    }

    fn has_libraries(&self) -> bool {
        true
    }

    fn chosen_libraries(&self) -> Vec<Library> {
        selected_libraries(self.slug)
    }

    async fn libraries(&self) -> Result<Libraries> {
        libraries(self.slug).await
    }

    fn choose_libraries(&self, libraries: Vec<Library>) -> Result<()> {
        select_libraries(self.slug, libraries)
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Credentials {
            server: String::new(),
            username: String::new(),
            password: String::new(),
        }]
    }

    fn stored(&self) -> bool {
        auth::load(self.slug).is_some()
    }

    fn location(&self) -> Option<String> {
        auth::load(self.slug).map(|credentials| credentials.server)
    }

    /// The username, since a Subsonic server has no name of its own to show.
    fn account_name(&self) -> Option<String> {
        auth::load(self.slug).map(|credentials| credentials.username)
    }

    /// The configured server's host, since a self-hosted library has no address in common with
    /// anyone else's.
    fn reach(&self) -> Option<String> {
        let server = auth::load(self.slug)?.server;
        let host = server
            .split_once("://")
            .map_or(server.as_str(), |(_, rest)| rest);
        let host = host.split(['/', ':', '?']).next()?;
        (!host.is_empty()).then(|| host.to_owned())
    }

    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        let remembered = auth::load(self.slug)?;
        let signature = remembered
            .signature
            .clone()
            .unwrap_or_else(|| auth::sign(&remembered.username, &remembered.password));
        let client = SubsonicClient::new(
            remembered.server,
            remembered.username,
            remembered.password,
            &signature,
            remembered
                .libraries
                .iter()
                .map(|library| library.id)
                .collect(),
        )
        .inspect_err(|error| log::warn!("subsonic: cannot build an offline client: {error:#}"))
        .ok()?;
        Some(Arc::new(Factory::new(client)))
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        Self::restore_stored(self.slug).await
    }

    async fn sign_in(
        &self,
        method: SignIn,
        _prompt: music::PromptSink,
        _input: music::InputSource,
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
        Self::connect(self.slug, server, username, password).await
    }

    fn sign_out(&self) {
        auth::forget(self.slug);
    }
}
