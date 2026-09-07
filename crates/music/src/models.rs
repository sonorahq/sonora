use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserProfile {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserDetail {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub followers: Option<u64>,
    pub following: Option<u64>,
    pub playlists: Vec<Playlist>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
}

impl Contributor {
    pub fn unnamed(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
            avatar: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub name: String,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    pub name: String,
    pub role: String,
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: Option<String>,
    pub name: String,
    pub playable: bool,
    pub artists: String,
    pub artist_refs: Vec<ArtistRef>,
    pub album: String,
    pub album_id: Option<String>,
    pub cover: Option<String>,
    pub duration: Duration,
    pub added_at: Option<i64>,
    pub added_by: Option<Arc<Contributor>>,
    pub playcount: Option<u64>,
    pub popularity: u32,
    pub explicit: bool,
    pub track_number: u32,
    pub disc_number: u32,
    pub tags: Vec<String>,
    pub languages: Vec<String>,
    pub credits: Vec<Credit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub owner_id: String,
    pub owned: bool,
    pub collaborative: bool,
    pub blend: bool,
    pub public: bool,
    pub cover: Option<String>,
    pub track_count: u32,
    pub modified_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReleaseType {
    Album,
    Single,
    Compilation,
    Ep,
    Audiobook,
    Podcast,
}

impl ReleaseType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Album => "Album",
            Self::Single => "Single",
            Self::Compilation => "Compilation",
            Self::Ep => "EP",
            Self::Audiobook => "Audiobook",
            Self::Podcast => "Podcast",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artists: String,
    pub artist_refs: Vec<ArtistRef>,
    pub cover: Option<String>,
    pub cover_large: Option<String>,
    pub release_type: ReleaseType,
    pub year: i32,
    pub track_count: u32,
    pub release_date: String,
    pub label: String,
    pub copyrights: Vec<String>,
    pub added_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumDetail {
    pub album: Album,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genre {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenreItem {
    Playlist(Playlist),
    Album(Album),
    Genre(Genre),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreSection {
    pub title: String,
    pub items: Vec<GenreItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HomeFeed {
    pub listen_again: Vec<Track>,
    pub quick_picks: Option<Vec<Track>>,
    pub sections: Vec<GenreSection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenreDetail {
    pub name: String,
    pub sections: Vec<GenreSection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: String,
    pub track_total: String,
    pub disc_number: String,
    pub disc_total: String,
    pub year: String,
    pub genre: String,
    pub composer: String,
    pub publisher: String,
    pub isrc: String,
    pub comment: String,
    pub lyrics: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistDetail {
    pub playlist: Playlist,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistProfile {
    pub name: String,
    pub cover_large: Option<String>,
    pub biography: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedArtist {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub added_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub cover_large: Option<String>,
    pub biography: Option<String>,
    pub monthly_listeners: Option<u64>,
    pub top_tracks: Vec<Track>,
    pub albums: Vec<Album>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lyrics {
    Plain {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        romanized: Option<RomanizedText>,
    },
    Synced {
        lines: Arc<[LyricsLine]>,
    },
}

impl Lyrics {
    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        let romanized = crate::lyrics::romanize::plain(&text);
        Self::Plain { text, romanized }
    }

    pub fn synced(&self) -> bool {
        matches!(self, Self::Synced { .. })
    }

    pub fn worded(&self) -> bool {
        match self {
            Self::Plain { .. } => false,
            Self::Synced { lines } => lines.iter().any(LyricsLine::worded),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Plain { text, .. } => text.trim().is_empty(),
            Self::Synced { lines } => lines.is_empty(),
        }
    }

    pub fn span(&self) -> Option<Duration> {
        let Self::Synced { lines } = self else {
            return None;
        };
        lines
            .iter()
            .map(|line| line.end.unwrap_or(line.start))
            .max()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsLine {
    pub start: Duration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Duration>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanized: Option<RomanizedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LyricsWord>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<LyricsLane>,
    #[serde(default, skip_serializing_if = "Voice::lead")]
    pub voice: Voice,
}

impl LyricsLine {
    pub fn worded(&self) -> bool {
        self.words.as_ref().is_some_and(|words| !words.is_empty())
            || self.secondary.iter().any(LyricsLane::worded)
    }

    pub fn sung_end(&self) -> Option<Duration> {
        let primary = self
            .words
            .as_ref()
            .and_then(|words| words.iter().rev().find(|word| !word.text.trim().is_empty()))
            .map(|word| word.end.max(word.start).max(self.start))
            .or(self.end);
        self.secondary
            .iter()
            .filter_map(LyricsLane::sung_end)
            .chain(primary)
            .max()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsLane {
    pub start: Duration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Duration>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanized: Option<RomanizedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LyricsWord>>,
}

impl LyricsLane {
    pub fn worded(&self) -> bool {
        self.words.as_ref().is_some_and(|words| !words.is_empty())
    }

    pub fn sung_end(&self) -> Option<Duration> {
        self.words
            .as_ref()
            .and_then(|words| words.iter().rev().find(|word| !word.text.trim().is_empty()))
            .map(|word| word.end.max(word.start).max(self.start))
            .or(self.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomanizedText {
    pub text: String,
    pub writing_system: WritingSystem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WritingSystem {
    Japanese,
    Chinese,
    Korean,
    Cyrillic,
    Greek,
    Arabic,
    Other,
}

impl WritingSystem {
    pub const ALL: [Self; 7] = [
        Self::Japanese,
        Self::Chinese,
        Self::Korean,
        Self::Cyrillic,
        Self::Greek,
        Self::Arabic,
        Self::Other,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Voice {
    #[default]
    Lead,
    Counter,
}

impl Voice {
    pub fn lead(&self) -> bool {
        matches!(self, Self::Lead)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsWord {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackKey {
    pub provider: &'static str,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Duration,
    pub track: Option<TrackKey>,
    pub language: Option<String>,
}

impl LyricsQuery {
    pub fn id_for(&self, provider: &str) -> Option<&str> {
        self.track
            .as_ref()
            .filter(|track| track.provider == provider)
            .map(|track| track.id.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsHit {
    pub source: &'static str,
    pub trust: u32,
    pub lyrics: Lyrics,
    pub instrumental: bool,
    pub fallback: bool,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub writers: Vec<String>,
}
