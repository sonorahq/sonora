pub mod apple;
pub mod artwork;
mod audio;
pub mod binimum;
pub mod credentials;
pub mod deezer;
pub mod drm;
pub mod engine;
pub mod equalizer;
pub mod escape;
pub mod kugou;
#[cfg(test)]
mod live_tests;
pub mod local;
pub mod lrclib;
pub mod lyrics;
mod models;
pub mod musixmatch;
pub mod netease;
pub mod potoken;
pub mod progress;
pub mod scrobble;
mod sink;
mod spectrum;
pub mod spotify;
mod stream;
pub mod subsonic;
mod trim;
pub mod trouble;
pub mod youtube;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use async_trait::async_trait;

pub use equalizer::Equalizer;
pub use models::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, ArtistRef,
    Contributor, Credit, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, Lyrics, LyricsHit,
    LyricsLane, LyricsLine, LyricsQuery, LyricsWord, PinOutcome, PinTarget, PinTargetKind,
    Playlist, PlaylistDetail, PlaylistEntry, PlaylistFolder, ReleaseType, RomanizedText,
    SavedArtist, Track, TrackKey, TrackTags, UserDetail, UserProfile, Voice, WritingSystem,
};
pub use spectrum::Spectrum;

pub const LOCAL_TRACK_PREFIX: &str = "local:";
pub const LOCAL_ALBUM_PREFIX: &str = "local-album:";
pub const LOCAL_ARTIST_PREFIX: &str = "local-artist:";
pub const LOCAL_PLAYLIST_PREFIX: &str = "local-playlist:";

/// The most recommendations a provider hands one list of an album or artist page, so a
/// rail never asks for or draws more than this many releases or artists.
pub const SUGGESTIONS: usize = 10;

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

/// What `MusicApi::report` tells the provider's server about the current track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Playing,
    Paused,
    Stopped,
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

    /// The rest of an artist page, fetched once `artist` has put the overview up: the whole
    /// discography and the popular tracks that only the discography can rank. `known` is the
    /// top tracks already on the page, so the provider can rank around them. A provider whose
    /// `artist` already answers with everything leaves the default, which is nothing more to
    /// fetch.
    async fn artist_catalogue(
        &self,
        _artist_id: &str,
        _known: &[Track],
    ) -> Result<ArtistCatalogue> {
        Ok(ArtistCatalogue::default())
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile>;
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>>;

    /// The tracks the user starred. On a `Shape::Saved` provider this is the whole songs
    /// library; on a `Shape::Catalog` one it only feeds the hearts and the favorites filter.
    async fn saved_tracks(&self) -> Result<Vec<Track>>;

    /// The favorites a page at a time, for a library page that shows its first rows while the
    /// rest arrive. A provider that lists in one go leaves the default, which is one page.
    async fn saved_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(whole(self.saved_tracks().await?))
    }

    /// Every track the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_tracks(&self) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn all_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(whole(self.all_tracks().await?))
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

    /// Tells the provider's own server whether a track is playing and where it is. It is sent on
    /// every start, pause, seek and stop, and never counts as a listen. A provider that keeps no
    /// listening record keeps the default and makes no request.
    async fn report(&self, _track_id: &str, _report: Report, _position: Duration) -> Result<()> {
        Ok(())
    }

    /// Records a finished listen that started at `at` on the provider's own server. It is sent
    /// at the same moment and under the same rules as a scrobble. A provider that keeps no
    /// listening record keeps the default and makes no request.
    async fn played(&self, _track_id: &str, _at: SystemTime) -> Result<()> {
        Ok(())
    }
    async fn playlists(&self) -> Result<Vec<PlaylistEntry>>;
    /// Changes the provider's own pin for `uri`, one of the uris `pin_targets` lists or
    /// `pin_uri` builds.
    async fn set_pinned(&self, _uri: &str, _pinned: bool) -> Result<PinOutcome> {
        anyhow::bail!("pinning is not supported")
    }

    /// What the provider can pin, each saying whether it is pinned now. A provider may list
    /// only its pinned items and answer `pin_uri` for the rest. `None` means the provider
    /// keeps no pins of its own.
    async fn pin_targets(&self) -> Result<Option<Vec<PinTarget>>> {
        Ok(None)
    }

    /// The uri `set_pinned` takes for an item `pin_targets` does not list. `None` means only
    /// a listed item can be pinned on the provider's side.
    fn pin_uri(&self, _kind: PinTargetKind, _id: &str) -> Option<String> {
        None
    }
    async fn create_playlist(&self, name: &str) -> Result<String>;
    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()>;
    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()>;
    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()>;

    /// Puts a track or an album into the listener's library, or takes it out, on a provider
    /// whose library is apart from its favorites. Only a `Capabilities::library` provider
    /// answers. An artist is never added: a library artist is one whose music is there.
    async fn set_in_library(&self, _kind: MediaKind, _id: &str, _present: bool) -> Result<()> {
        anyhow::bail!("this provider has no library apart from its favorites")
    }
    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()>;
    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn saved_albums(&self) -> Result<Vec<Album>>;

    /// Every album the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_albums(&self) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn saved_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(whole(self.saved_albums().await?))
    }

    async fn all_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(whole(self.all_albums().await?))
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()>;
    async fn saved_artists(&self) -> Result<Vec<SavedArtist>>;

    /// Every artist the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        Ok(Vec::new())
    }

    async fn saved_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(whole(self.saved_artists().await?))
    }

    async fn all_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(whole(self.all_artists().await?))
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()>;
    async fn album(&self, album_id: &str) -> Result<AlbumDetail>;
    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>>;

    /// The rest of an album page, fetched once `album` has put the tracks up: the releases
    /// the provider lists as related, with more from the same artist first and similar
    /// artists' releases topping the rail up. `artist_id` is
    /// the page's artist when the album names one the app can follow. A provider whose
    /// `album` already answers with everything leaves the default, which is nothing more
    /// to fetch.
    async fn album_catalogue(
        &self,
        _album_id: &str,
        _artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        Ok(AlbumCatalogue::default())
    }
    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail>;
    async fn playlist_continuation(
        &self,
        _continuation: &str,
    ) -> Result<(Vec<Track>, Option<String>)> {
        anyhow::bail!("playlist pagination is not supported")
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>>;
    /// The station seeded by `track_id`, from its start or from `from`, a continuation an
    /// earlier call answered with. The continuation that comes back fetches the next stretch,
    /// and `None` means the provider has no more. A provider that serves a station in one go
    /// ignores `from` and answers `None`, which it is then never handed.
    async fn track_radio(
        &self,
        track_id: &str,
        from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)>;

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

    /// The home feed as it fills, so a page draws its first shelves before its last have
    /// arrived. Defaults to `home` delivered at once, so a provider that has the feed in one go
    /// writes nothing.
    async fn home_paged(&self) -> Result<Feed> {
        Ok(at_once(self.home().await?))
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

/// What an engine is started with. `equalizer` is shared rather than copied: the engine keeps
/// reading it, so a change reaches the output without a restart.
#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    pub normalisation: bool,
    pub gapless: bool,
    pub position_interval: Duration,
    pub gain: f32,
    pub equalizer: Equalizer,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// The library is what the user starred, read through the `saved_*` methods.
    Saved,
    /// The library is everything the provider has, read through the `all_*` methods, with the
    /// `saved_*` set drawn on top as hearts and a filter.
    Catalog,
}

/// What a provider can do beyond listing and playing, so a control it has no answer for is
/// never put in front of the listener.
///
/// This is about the service, not the account: something a provider simply does not have, like
/// a station Apple Music will not list or a play count Deezer does not keep. A capability that
/// is off hides its button, its menu item and its column, rather than showing one that fails
/// when pressed. A provider that gains one flips a flag here and the UI follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The listener can follow and unfollow an artist.
    pub follow_artists: bool,
    /// A track can seed a station, which is what fills the queue behind it.
    pub radio: bool,
    /// Tracks carry a play count worth a column of its own.
    pub playcounts: bool,
    /// The listener has a library apart from their favorites, which a track or an album can be
    /// put into and taken out of through `set_in_library`. Off where the library is the
    /// favorites, as on Spotify, and where it is fixed, as on a self-hosted server.
    pub library: bool,
    /// The provider keeps sidebar pins of its own, listed by `pin_targets` and changed
    /// through `set_pinned`. Off, a pin lives in Sonora's settings alone.
    pub pins: bool,
}

impl Capabilities {
    /// What a full streaming service offers. A library apart from favorites is not among
    /// them: on most services the two are one thing. We love Apple Music. Pins of the
    /// provider's own are not either, since only Spotify and Apple Music keep any.
    pub const ALL: Self = Self {
        follow_artists: true,
        radio: true,
        playcounts: true,
        library: false,
        pins: false,
    };

    /// Nothing beyond listing and playing.
    pub const NONE: Self = Self {
        follow_artists: false,
        radio: false,
        playcounts: false,
        library: false,
        pins: false,
    };
}

pub struct ProviderSession {
    pub profile: UserProfile,
    pub api: Arc<dyn MusicApi>,
    pub playback: Arc<dyn PlaybackFactory>,
    pub shape: Shape,
    pub authenticated: bool,
    pub capabilities: Capabilities,
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
            SignInProblem::Network => "the provider could not be reached",
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

/// One page of a listing that arrives in pieces. `total` is how long the whole listing will
/// be, on the pages of a provider that knows before the last one; a page that does not know
/// carries `None`, and the count so far stands in.
pub struct Page<T> {
    pub total: Option<usize>,
    pub items: Vec<T>,
}

/// A listing arriving a page at a time, in order. The channel closes after the last page, or
/// carries the error a page broke on, after which nothing more comes. Dropping it stops the
/// provider fetching.
pub type Pages<T> = tokio::sync::mpsc::Receiver<Result<Page<T>>>;

/// The home feed arriving a lot at a time: every message is the whole feed so far, arranged
/// the way the provider wants it drawn, so each one can replace the last on the page. The
/// channel closes after the last lot, or carries the error one broke on, after which nothing
/// more comes. Dropping it stops the provider fetching.
pub type Feed = tokio::sync::mpsc::Receiver<Result<HomeFeed>>;

/// A home feed that arrived whole, as its one and only message.
pub fn at_once(feed: HomeFeed) -> Feed {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    // Room for one message was made above, so this never waits and never fails.
    sender.try_send(Ok(feed)).ok();
    receiver
}

/// A listing that arrived whole, as its one and only page. What a provider that lists in one
/// go answers the paged calls with.
pub fn whole<T: Send + 'static>(items: Vec<T>) -> Pages<T> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let total = Some(items.len());
    // Room for one page was made above, so this never waits and never fails.
    sender.try_send(Ok(Page { total, items })).ok();
    receiver
}

/// A cookie sign-in the app runs in its own browser window. `url` opens first and `landing` scopes
/// URL-based cookie reads. The user is through once the cookies for `domain` carry one of the
/// `proof` names. The header those cookies make is what `SignInPrompt::Secret` then receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebSignIn {
    pub url: &'static str,
    pub landing: &'static str,
    pub domain: &'static str,
    pub proof: &'static [&'static str],
    /// A user agent the window presents instead of its default. Some providers' anti-bot
    /// checks reject an agent that does not match the engine behind it (a Firefox string on a
    /// WebKit window); the string must match what the backend's engine would say. Backends
    /// whose default is already engine-consistent may ignore it.
    pub agent: Option<&'static str>,
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn slug(&self) -> &'static str;

    /// Forgets whatever the provider remembers about its last scan, so the next one reads
    /// everything again. Only a provider that scans files has anything to forget, and only a
    /// rescan the user asked for should ask it to.
    fn forget_scan(&self) {}
    fn sign_in_options(&self) -> Vec<SignIn>;
    fn stored(&self) -> bool;
    /// Whether what is stored is an anonymous session rather than an account, so a caller
    /// can tell the two apart. A provider without an anonymous sign-in never says yes.
    fn stored_guest(&self) -> bool {
        false
    }
    fn location(&self) -> Option<String> {
        None
    }
    /// The host to open a connection to when checking whether the network is back. `None`
    /// where the provider needs no network, which is what keeps a local library from ever
    /// looking for one.
    fn reach(&self) -> Option<String> {
        None
    }
    /// What a status calls this provider after "listening to". A service answers with its own
    /// name; one that is only the user's own files says what the files are instead.
    fn listening_to(&self) -> &'static str {
        self.name()
    }
    /// Whether this provider's tracks need the Widevine module to play. The app fetches the
    /// module once such a provider has an account, and does not go near it otherwise.
    fn protected(&self) -> bool {
        false
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
