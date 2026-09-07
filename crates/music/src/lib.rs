mod audio;
pub mod binimum;
pub mod credentials;
pub mod kugou;
#[cfg(test)]
mod live_tests;
pub mod local;
pub mod lrclib;
pub mod lyrics;
mod models;
pub mod musixmatch;
pub mod netease;
mod sink;
mod spectrum;
pub mod spotify;
pub mod subsonic;
pub mod youtube;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

pub use models::{
    Album, AlbumDetail, Artist, ArtistProfile, ArtistRef, Contributor, Credit, Genre, GenreDetail,
    GenreItem, GenreSection, HomeFeed, LibraryItem, LibraryItemKind, LibraryOrder,
    LibraryPinResult, Lyrics, LyricsHit, LyricsLane, LyricsLine, LyricsQuery, LyricsWord, Playlist,
    PlaylistDetail, ReleaseType, RomanizedText, SavedArtist, Track, TrackKey, TrackTags,
    UserDetail, UserProfile, Voice, WritingSystem,
};
pub use spectrum::Spectrum;

pub const LOCAL_TRACK_PREFIX: &str = "local:";
pub const LOCAL_ALBUM_PREFIX: &str = "local-album:";
pub const LOCAL_ARTIST_PREFIX: &str = "local-artist:";
pub const LOCAL_PLAYLIST_PREFIX: &str = "local-playlist:";

pub fn is_local_id(id: &str) -> bool {
    id.starts_with(LOCAL_TRACK_PREFIX)
        || id.starts_with(LOCAL_ALBUM_PREFIX)
        || id.starts_with(LOCAL_ARTIST_PREFIX)
        || id.starts_with(LOCAL_PLAYLIST_PREFIX)
}

pub fn distinct_covers(tracks: &[Track], wanted: usize) -> Vec<String> {
    let mut covers: Vec<String> = Vec::with_capacity(wanted);
    for cover in tracks.iter().filter_map(|track| track.cover.as_deref()) {
        if covers.len() == wanted {
            break;
        }
        if !covers.iter().any(|kept| kept == cover) {
            covers.push(cover.to_owned());
        }
    }

    covers
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Track,
    Album,
    Artist,
    Playlist,
}

#[async_trait]
pub trait MusicApi: Send + Sync {
    fn alive(&self) -> bool {
        true
    }

    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String>;
    async fn profile(&self) -> Result<UserProfile>;

    async fn user(&self, _user_id: &str) -> Result<UserDetail> {
        anyhow::bail!("user profiles are not supported")
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist>;
    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile>;
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>>;

    /// The tracks the user starred. On a `Shape::Saved` provider this is the whole songs
    /// library; on a `Shape::Catalog` one it only feeds the hearts and the favorites filter.
    async fn saved_tracks(&self, limit: u32) -> Result<Vec<Track>>;

    /// Every track the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_tracks(&self, _limit: u32) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()>;

    /// Reads what the file itself says, for a provider whose tracks are files.
    async fn track_tags(&self, _track_id: &str) -> Result<TrackTags> {
        anyhow::bail!("this provider cannot edit tags")
    }

    async fn set_track_tags(&self, _track_id: &str, _tags: TrackTags) -> Result<()> {
        anyhow::bail!("this provider cannot edit tags")
    }
    async fn track(&self, track_id: &str) -> Result<Track>;

    /// Reads an arbitrary file on disk as a track, for a provider whose tracks are files. Used
    /// by file-association opens, which may point outside any scanned folder.
    async fn track_from_path(&self, _path: &Path) -> Result<Track> {
        anyhow::bail!("cannot open arbitrary files")
    }

    /// Delete a track file from disk (only for local provider)
    async fn delete_track_file(&self, _track_id: &str) -> Result<()> {
        anyhow::bail!("this provider does not support file deletion")
    }
    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>>;
    async fn track_lyrics(&self, _track_id: &str) -> Result<Option<Lyrics>> {
        Ok(None)
    }
    async fn playlists(&self, limit: u32) -> Result<Vec<Playlist>>;
    /// Change a provider's own library pin, rather than a local sidebar shortcut.
    async fn set_library_item_pinned(&self, _uri: &str, _pinned: bool) -> Result<LibraryPinResult> {
        anyhow::bail!("library pinning is not supported")
    }

    /// The provider's mixed library, including pins and its recent-play ordering.
    /// None means this provider exposes only the separate saved collections.
    async fn library_items(&self, _order: LibraryOrder) -> Result<Option<Vec<LibraryItem>>> {
        Ok(None)
    }
    async fn create_playlist(&self, name: &str) -> Result<String>;
    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()>;
    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()>;
    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()>;
    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()>;
    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn saved_albums(&self, limit: u32) -> Result<Vec<Album>>;

    /// Every album the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_albums(&self, _limit: u32) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()>;
    async fn saved_artists(&self, limit: u32) -> Result<Vec<SavedArtist>>;

    /// Every artist the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_artists(&self, _limit: u32) -> Result<Vec<SavedArtist>> {
        Ok(Vec::new())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()>;
    async fn album(&self, album_id: &str) -> Result<AlbumDetail>;
    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>>;
    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail>;
    async fn playlist_continuation(
        &self,
        _continuation: &str,
    ) -> Result<(Vec<Track>, Option<String>)> {
        anyhow::bail!("playlist pagination is not supported")
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>>;
    async fn track_radio(&self, track_id: &str) -> Result<Vec<Track>>;
    async fn search(&self, query: &str) -> Result<Vec<Track>>;

    async fn search_albums(&self, _query: &str) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn search_playlists(&self, _query: &str) -> Result<Vec<Playlist>> {
        Ok(Vec::new())
    }

    async fn home(&self) -> Result<HomeFeed> {
        Ok(HomeFeed::default())
    }

    async fn name_home_playlists(&self, sections: Vec<GenreSection>) -> Vec<GenreSection> {
        sections
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        Ok(Vec::new())
    }

    async fn genre(&self, _genre_id: &str) -> Result<GenreDetail> {
        Ok(GenreDetail::default())
    }
}

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>>;
}

#[derive(Clone, Copy, Debug)]
pub struct PlaybackConfig {
    pub normalisation: bool,
    pub gapless: bool,
    pub position_interval: Duration,
    pub gain: f32,
}

/// What an engine reports back. `Playing` and `Seeked` mean audio from `at` is reaching the
/// output, not that a decoder is ready, so whatever follows the sound can start on them.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackEvent {
    Loading {
        id: Option<String>,
        at: Duration,
    },
    Playing {
        id: Option<String>,
        at: Duration,
    },
    Paused {
        id: Option<String>,
        at: Duration,
    },
    /// A progress report from the decoder. It runs ahead of what is audible by whatever the
    /// output has queued, and none arrive while the engine is busy with a seek.
    Position {
        id: Option<String>,
        at: Duration,
    },
    /// Audio from the new position has reached the output after a seek.
    Seeked {
        id: Option<String>,
        at: Duration,
    },
    Length {
        id: Option<String>,
        duration: Duration,
    },
    Ended {
        id: Option<String>,
    },
    Unavailable {
        id: Option<String>,
    },
    Refused,
    Gated,
    OutputChanged,
}

impl PlaybackEvent {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Loading { id, .. }
            | Self::Playing { id, .. }
            | Self::Paused { id, .. }
            | Self::Position { id, .. }
            | Self::Seeked { id, .. }
            | Self::Length { id, .. }
            | Self::Ended { id, .. }
            | Self::Unavailable { id, .. } => id.as_deref(),
            _ => None,
        }
    }
}

/// Transport control of one engine. Every call is fire-and-forget: the outcome arrives as a
/// `PlaybackEvent`, never as a return value.
pub trait Player: Send + Sync {
    /// Fetches a track and plays it from `at`. A `seamless` load is a queue segue: a gapless
    /// engine keeps what it has queued so the join has no gap, any other load drops it.
    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()>;

    /// Fetches a track and leaves it paused at `at`, ready for `play`.
    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()>;

    /// Fetches a track ahead of time so a later `load` starts at once. `segue` marks the next
    /// queue item, which a gapless engine may already line up behind the current one.
    fn preload(&self, track_id: &str, segue: bool) -> Result<()>;
    fn play(&self);
    fn pause(&self);

    /// Moves to `position`. While loading, the track starts there instead; while playing, a
    /// `Seeked` event follows once audio from there reaches the output.
    fn seek(&self, position: Duration);
    fn set_gain(&self, gain: f32);

    fn spectrum(&self) -> Option<Spectrum> {
        None
    }
}

#[async_trait]
pub trait PlaybackEvents: Send {
    async fn next(&mut self) -> Option<PlaybackEvent>;
}

pub trait PlaybackFactory: Send + Sync {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>);
}

/// What a provider's library is made of. It decides which `MusicApi` methods fill the library
/// pages and whether a favorites filter is offered on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The library is what the user starred, read through the `saved_*` methods.
    Saved,
    /// The library is everything the provider has, read through the `all_*` methods, with the
    /// `saved_*` set drawn on top as hearts and a filter.
    Catalog,
}

pub struct ProviderSession {
    pub profile: UserProfile,
    pub api: Arc<dyn MusicApi>,
    pub playback: Arc<dyn PlaybackFactory>,
    pub shape: Shape,
    pub authenticated: bool,
    pub playcounts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn {
    Default,
    Anonymous,
    Secret,
    Path(Vec<PathBuf>),
    Credentials {
        server: String,
        username: String,
        password: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInProblem {
    Premium,
    Region,
    Credentials,
    Network,
    Cancelled,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignInFailure(pub SignInProblem);

impl std::fmt::Display for SignInFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.0 {
            SignInProblem::Premium => "the account has no Spotify Premium",
            SignInProblem::Region => "the account is out of its home region",
            SignInProblem::Credentials => "the stored credentials are no longer valid",
            SignInProblem::Network => "Spotify could not be reached",
            SignInProblem::Cancelled => "authorization was cancelled in the browser",
            SignInProblem::Refused => "Spotify refused the session",
        };
        write!(f, "{reason}")
    }
}

impl std::error::Error for SignInFailure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountChoice {
    pub id: String,
    pub name: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInPrompt {
    Accounts(Vec<AccountChoice>),
    Code { code: String, url: String },
    Url(String),
    Secret,
}

pub type PromptSink = Arc<dyn Fn(SignInPrompt) + Send + Sync>;
pub type InputSource = tokio::sync::mpsc::UnboundedReceiver<String>;

/// A cookie sign-in the app runs in its own browser window. `url` opens first and `landing` scopes
/// URL-based cookie reads. The user is through once the cookies for `domain` carry one of the
/// `proof` names. The header those cookies make is what `SignInPrompt::Secret` then receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebSignIn {
    pub url: &'static str,
    pub landing: &'static str,
    pub domain: &'static str,
    pub proof: &'static [&'static str],
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn slug(&self) -> &'static str;
    fn sign_in_options(&self) -> Vec<SignIn>;
    fn stored(&self) -> bool;
    fn location(&self) -> Option<String> {
        None
    }
    /// What a status calls this provider after "listening to". A service answers with its own
    /// name; one that is only the user's own files says what the files are instead.
    fn listening_to(&self) -> &'static str {
        self.name()
    }
    /// Whether the artwork urls this provider hands out can be given to another service. A path
    /// on disk means nothing elsewhere, and a self-hosted url carries the credentials that fetch
    /// it, so the default is no.
    fn public_art(&self) -> bool {
        false
    }
    async fn restore(&self) -> Result<Option<ProviderSession>>;
    async fn sign_in(
        &self,
        method: SignIn,
        prompt: PromptSink,
        input: InputSource,
    ) -> Result<ProviderSession>;
    fn abandon(&self) {}
    fn sign_out(&self);
    /// How to run `SignIn::Secret` in a browser window. `None` means the provider has no cookie
    /// sign-in, and the app offers no `Secret` option for it.
    fn web_sign_in(&self) -> Option<WebSignIn> {
        None
    }
}
