mod aac;
pub mod apple;
pub mod artwork;
mod audio;
pub mod binimum;
mod calendar;
pub mod connect;
pub mod cover;
pub mod credentials;
pub mod deezer;
pub mod drm;
pub mod engine;
pub mod equalizer;
pub mod escape;
pub mod kugou;
pub mod link;
#[cfg(test)]
mod live_tests;
pub mod local;
pub mod lrclib;
pub mod lyrics;
mod models;
pub mod musixmatch;
pub mod netease;
pub mod output;
pub mod potoken;
pub mod progress;
pub mod scrobble;
mod sink;
mod spectrum;
pub mod spotify;
mod stream;
pub mod subsonic;
pub mod tidal;
mod trim;
pub mod trouble;
pub mod vault;
pub mod youtube;

use std::collections::HashMap;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use async_trait::async_trait;
use time::format_description::well_known::Iso8601;
use time::parsing::Parsed;
use time::{Month, OffsetDateTime};

pub use audio::Meter;
pub use equalizer::Equalizer;
pub use models::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, ArtistRef,
    Contributor, CoverSize, Credit, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed,
    ImportedPlaylist, Lyrics, LyricsContributor, LyricsHit, LyricsLane, LyricsLine, LyricsQuery,
    LyricsRole, LyricsWord, PinOutcome, PinTarget, PinTargetKind, Playlist, PlaylistDetail,
    ReleaseType, RomanizedText, SavedArtist, Track, TrackKey, TrackTags, UserDetail, UserProfile,
    Voice, WritingSystem,
};
pub use output::{Server as SoundServer, missing_bridge};
pub use spectrum::{Analyzer, Level, Listening, Spectrum, Watch};

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

    /// The provider's device network, for a provider that has one. Spotify Connect is the
    /// only one so far.
    fn connect(&self) -> Option<Arc<connect::Connect>> {
        None
    }

    /// A cover url this provider handed out, at `size`. It is worked out from the url alone,
    /// so `None` means the provider cannot tell and the caller loads the album instead.
    fn cover_at(&self, _cover: &str, _size: CoverSize) -> Option<String> {
        None
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

    /// Tells the provider a track has started playing, so the play counts on the provider's
    /// own side. A provider that keeps no history keeps the default and makes no request.
    async fn report_play(&self, _track_id: &str) -> Result<()> {
        Ok(())
    }

    /// The tracks the account has recently played, newest first, across every device. A provider
    /// that keeps no cross-device history keeps the default and answers with nothing.
    async fn recently_played(&self) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn playlists(&self) -> Result<Vec<Playlist>>;
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
    /// whose library is apart from its favorites. Only a `Catalog::Editable` provider
    /// answers. An artist is never added: a library artist is one whose music is there.
    async fn set_in_library(&self, _kind: MediaKind, _id: &str, _present: bool) -> Result<()> {
        anyhow::bail!("this provider has no library apart from its favorites")
    }
    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()>;

    /// Replaces a playlist's cover with `image`, the bytes of a picture in any format
    /// [`cover::square_jpeg`] reads, or puts the provider's own cover back when it is `None`.
    /// Only a provider whose capabilities list `playlist_covers` answers.
    async fn set_playlist_cover(&self, _playlist_id: &str, _image: Option<&[u8]>) -> Result<()> {
        anyhow::bail!("this provider cannot change a playlist's cover")
    }
    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;

    /// Adds tracks to the end of a playlist in order, and a track named twice goes in twice. A
    /// provider that takes many at once does it in one write. The default adds them one by one
    /// and stops at the first that fails, leaving the ones before it in.
    async fn add_tracks_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<()> {
        for track_id in track_ids {
            self.add_track_to_playlist(playlist_id, track_id).await?;
        }
        Ok(())
    }
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;

    /// Reads the M3U or M3U8 playlist at `path` into a new playlist named after the file, for a
    /// provider whose tracks are files. Entries naming a file outside the library are left out
    /// and counted.
    async fn import_playlist(&self, _path: &Path) -> Result<ImportedPlaylist> {
        anyhow::bail!("this provider cannot import playlist files")
    }

    /// Writes a playlist to `path` as M3U8, for a provider whose tracks are files.
    async fn export_playlist(&self, _playlist_id: &str, _path: &Path) -> Result<()> {
        anyhow::bail!("this provider cannot export playlist files")
    }
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

    /// A picture for a genre that `genres` listed without one, found with as few requests as
    /// the provider allows. Defaults to none, for a provider whose genres come with covers.
    async fn genre_cover(&self, _genre_id: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// Hands over the key the user entered for this source, `None` once it is cleared. Only a
    /// source that needs a key of the user's own keeps it.
    fn set_key(&self, _key: Option<&str>) {}
    /// Whether the source can be asked now. One that needs a key is left out until it has one,
    /// so it is never recorded as having had nothing.
    fn ready(&self) -> bool {
        true
    }
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
    pub transcode: Transcode,
    pub quality: Quality,
    /// Shared like `equalizer`: every output the engine opens reports its loudest sample here.
    pub meter: Meter,
    /// Shared like `equalizer`: every output the engine opens lets it hear the music ahead of the
    /// equalizer while it is watched.
    pub analyzer: Analyzer,
    /// Where tracks are kept, the downloads and, while caching is on, the ones that played.
    pub vault: Option<Arc<vault::Vault>>,
}

/// How good a stream to ask a streaming service for. Each provider lists the levels it serves in
/// [`Capabilities::qualities`], plays any other level as the nearest of those through
/// [`Quality::within`], and steps further down by itself when the account or the track has
/// nothing at that level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quality {
    Low,
    Normal,
    High,
    Lossless,
    #[default]
    HiRes,
}

impl Quality {
    /// Every level, best first.
    pub const ALL: [Self; 5] = [
        Self::HiRes,
        Self::Lossless,
        Self::High,
        Self::Normal,
        Self::Low,
    ];

    /// The name settings store the level by.
    pub fn id(self) -> &'static str {
        match self {
            Self::HiRes => "hi-res",
            Self::Lossless => "lossless",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|quality| quality.id() == id)
    }

    /// The level out of `offers` that stands in for this one: the best at or below it, or the
    /// lowest offered when every one is above it. It is this level itself when `offers` is
    /// empty.
    pub fn within(self, offers: &[Offer]) -> Self {
        let levels = || offers.iter().map(|offer| offer.quality);
        let below = levels().filter(|level| *level <= self).max();
        below.or_else(|| levels().min()).unwrap_or(self)
    }
}

/// One level a service serves and what it streams at that level, so the listener can choose by
/// what they will hear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub quality: Quality,
    pub encoding: Encoding,
    /// The figures are the most a track comes at. Many come lower, such as an album that was
    /// never mastered in hi-res, or a plan that does not reach that far.
    pub up_to: bool,
}

/// What a stream or a file is encoded as: a lossless encoding with its bit depth, a lossy one
/// with its bit rate in kilobits a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// FLAC at this bit depth and sample rate in hertz.
    Flac {
        depth: u8,
        rate: u32,
    },
    /// Apple Lossless at this bit depth.
    Alac {
        depth: u8,
    },
    /// Uncompressed, as in WAV or AIFF, at this bit depth.
    Pcm {
        depth: u8,
    },
    Aac {
        kbps: u32,
    },
    Vorbis {
        kbps: u32,
    },
    Mp3 {
        kbps: u32,
    },
    Opus {
        kbps: u32,
    },
    /// A codec Sonora has no name for.
    Other,
}

/// What an engine knows about one track, reported as `PlaybackEvent::Signal` as it starts
/// decoding it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSignal {
    /// How the provider encodes it, when it says.
    pub encoding: Option<Encoding>,
    /// The rate and channel count it decodes to, before Sonora resamples it to the device's.
    pub rate: u32,
    pub channels: u16,
    pub normalisation: Normalisation,
}

/// What loudness normalisation does to a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Normalisation {
    Off,
    /// A fixed gain in decibels, zero when nothing says how loud the track is.
    Gain(f32),
    /// librespot normalises inside its own player with a limiter, so the gain is not known
    /// and changes with the music.
    Limited,
}

/// What a server that converts audio on request is asked to send. Only Subsonic honours it, and
/// the default leaves both choices to the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transcode {
    pub codec: Option<Codec>,
    /// The highest bitrate to send, in kilobits per second.
    pub bitrate: Option<u32>,
}

impl Encoding {
    /// The bits per sample of a lossless encoding, which a lossy one has none of.
    pub fn depth(self) -> Option<u8> {
        match self {
            Self::Flac { depth, .. } | Self::Alac { depth } | Self::Pcm { depth } => Some(depth),
            _ => None,
        }
    }
}

impl Transcode {
    /// The bitrates offered as a ceiling, highest first.
    pub const BITRATES: [u32; 7] = [320, 256, 192, 160, 128, 96, 64];
}

/// A lossy format a streaming server can convert a track into before sending it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Mp3,
    Opus,
    Aac,
}

impl Codec {
    pub const ALL: [Self; 3] = [Self::Mp3, Self::Opus, Self::Aac];

    /// The name Subsonic servers know the format by, which is also how settings store it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
            Self::Aac => "aac",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|codec| codec.id() == id)
    }
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
    /// The whole of the current track has arrived, so fetching the next one takes nothing from
    /// it. Engines that cannot tell never send this.
    Downloaded {
        id: Option<String>,
    },
    Ended {
        id: Option<String>,
    },
    Unavailable {
        id: Option<String>,
    },
    /// The provider turned the load down for now, as it does under a rate limit, so the same
    /// track may play after a wait.
    Throttled {
        id: Option<String>,
    },
    Refused,
    Gated,
    OutputChanged,
    /// What the engine knows about the sound of track `id`, sent as it starts decoding it. A
    /// track queued behind the current one reports before it is heard.
    Signal {
        id: String,
        track: TrackSignal,
    },
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
            | Self::Downloaded { id }
            | Self::Ended { id, .. }
            | Self::Unavailable { id, .. }
            | Self::Throttled { id } => id.as_deref(),
            Self::Signal { id, .. } => Some(id),
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

    /// Fetches all of a track into the downloads without playing it. The future answers once it
    /// is kept, or why it could not be. A player that cannot download says so at once.
    /// `progress` follows how much of it has arrived.
    fn download(
        &self,
        _track_id: &str,
        _progress: Fraction,
    ) -> futures::future::BoxFuture<'static, Result<()>> {
        Box::pin(async { Err(anyhow::anyhow!("this player cannot download")) })
    }

    /// Whether a track has been downloaded.
    fn downloaded(&self, _track_id: &str) -> bool {
        false
    }

    /// Whether a track plays with no network, which only a download can. While the vault is
    /// still unlocking, a player whose downloads play offline answers yes for every track, since
    /// it cannot tell yet which are downloaded, so the caller checks its own list then.
    fn plays_offline(&self, track_id: &str) -> bool {
        self.downloaded(track_id)
    }

    /// Removes the download of a track.
    fn discard(&self, _track_id: &str) {}
}

#[async_trait]
pub trait PlaybackEvents: Send {
    async fn next(&mut self) -> Option<PlaybackEvent>;
}

/// How far a download has got, from 0 to 1, shared between the player that fetches it and the
/// app that shows it. It reads `None` until the player can tell.
#[derive(Clone, Debug)]
pub struct Fraction(Arc<std::sync::atomic::AtomicU32>);

impl Default for Fraction {
    fn default() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU32::new(
            f32::NAN.to_bits(),
        )))
    }
}

impl Fraction {
    pub fn get(&self) -> Option<f32> {
        let value = f32::from_bits(self.0.load(std::sync::atomic::Ordering::Relaxed));
        (!value.is_nan()).then_some(value)
    }

    pub fn set(&self, value: f32) {
        self.0.store(
            value.clamp(0.0, 1.0).to_bits(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
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
    /// The library is a catalog read through the `all_*` methods, with the `saved_*` set drawn
    /// on top as hearts and a filter.
    Catalog(Catalog),
}

impl Shape {
    pub fn is_catalog(self) -> bool {
        matches!(self, Self::Catalog(_))
    }

    /// Whether the listener can put things into the library and take them out, apart from
    /// favoriting them.
    pub fn editable(self) -> bool {
        self == Self::Catalog(Catalog::Editable)
    }
}

/// Whether the listener can change what a catalog holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Catalog {
    /// Everything the provider has, which changes only with the files behind it, as with local
    /// files and a self-hosted server.
    Fixed,
    /// A set the listener picked out of a bigger catalog, which tracks and albums are put into
    /// and taken out of through `set_in_library`, as on Apple Music.
    Editable,
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
    /// The provider keeps sidebar pins of its own, listed by `pin_targets` and changed
    /// through `set_pinned`. Off, a pin lives in Sonora's settings alone.
    pub pins: bool,
    /// The server converts tracks on request, as [`Transcode`] asks.
    pub transcode: bool,
    /// The listener can give an owned playlist a cover of their own through
    /// `set_playlist_cover`, and take it off again.
    pub playlist_covers: bool,
    /// Played tracks can be kept in the [`vault`], so playing one again downloads nothing.
    pub keeps: bool,
    /// Tracks can be downloaded. Whether a download then plays with no network is up to the
    /// player, through [`Player::plays_offline`].
    pub downloads: bool,
    /// The stream levels the service serves, best first. Empty when there is nothing to choose
    /// between, so the setting stays hidden.
    pub qualities: &'static [Offer],
    /// For a service that refuses a burst of downloads, the pause between them once it has refused
    /// one. Such a service downloads one track at a time, with no pause until it first refuses.
    /// Zero lets downloads run side by side.
    pub download_gap: Duration,
}

impl Capabilities {
    /// What a full streaming service offers. Pins of the provider's own are not among them,
    /// since only Spotify and Apple Music keep any. Converting on request is not either, since
    /// only Subsonic does it. Playlist covers are left for each provider to turn on.
    pub const ALL: Self = Self {
        follow_artists: true,
        radio: true,
        playcounts: true,
        pins: false,
        transcode: false,
        playlist_covers: false,
        keeps: true,
        downloads: true,
        qualities: &[],
        download_gap: Duration::ZERO,
    };

    /// Nothing beyond listing and playing.
    pub const NONE: Self = Self {
        follow_artists: false,
        radio: false,
        playcounts: false,
        pins: false,
        transcode: false,
        playlist_covers: false,
        keeps: false,
        downloads: false,
        qualities: &[],
        download_gap: Duration::ZERO,
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

/// A provider refusing for now because too much was asked of it too quickly. Whoever asked can
/// try again later, and finds this in the error's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throttled;

impl std::fmt::Display for Throttled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the provider is refusing for now")
    }
}

impl std::error::Error for Throttled {}

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
    /// A factory for the provider's playback engine that can work without sign-in and scan,
    /// so playback can start before the library has loaded. Only for a provider whose tracks
    /// are files.
    fn playback_factory(&self) -> Option<Arc<dyn PlaybackFactory>> {
        None
    }
    /// Reads an arbitrary file on disk as a track, for a provider whose tracks are files. Used
    /// by file-association opens, which may point outside any scanned folder.
    fn track_from_path(&self, _path: &Path) -> Option<Track> {
        None
    }
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
    /// A playback factory built from the stored sign-in alone, with no network, for a provider
    /// whose downloads play offline. The session starts it when the account cannot be reached,
    /// inside the app's tokio runtime.
    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        None
    }
    /// What `link` points at when it is one of this provider's own, on its website or in its
    /// app's scheme. It reads the text alone and fetches nothing, so a short link comes back as
    /// `Linked::Short` for the caller to follow and ask again.
    fn link(&self, _link: &str) -> Option<link::Linked> {
        None
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

/// Leniently convert an ISO8601 timestamp to unix epoch seconds. Accepts only
/// the date portion, date and time portions, or full date and time with offset.
pub fn iso_8601_to_epoch(value: Option<&str>) -> Option<i64> {
    let time_str = value?.as_bytes();
    let defaults = Parsed::new()
        .with_month(Month::January)
        .and_then(|d| d.with_day(NonZero::<u8>::new(1)?))
        .and_then(|d| d.with_hour_24(0))
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_subsecond(0))
        .and_then(|d| d.with_offset_hour(0))
        .and_then(|d| d.with_offset_minute_signed(0))
        .and_then(|d| d.with_offset_second_signed(0))?;
    let timestamp = OffsetDateTime::parse_with_defaults(time_str, &Iso8601::PARSING, defaults)
        .ok()?
        .unix_timestamp();
    Some(timestamp)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn iso_8601_to_epoch_parses_correctly() {
        // None and malformed input
        assert_eq!(iso_8601_to_epoch(None), None);
        assert_eq!(iso_8601_to_epoch(Some("")), None);
        assert_eq!(iso_8601_to_epoch(Some("malformed")), None);

        // Basic epoch format
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01")), Some(0));
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01T00:00:00")), Some(0));
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01T00:00:00Z")), Some(0));
        assert_eq!(
            iso_8601_to_epoch(Some("1970-01-01T00:00:00+00:00")),
            Some(0)
        );

        // A specific date with/without time/offset
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20")),
            Some(datetime!(2021-03-20 00:00:00 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07.123456")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07Z")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07+00:00")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T10:45:07-03:00")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
    }
}
