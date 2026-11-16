//! What Jellyfin answers with, and how it becomes Sonora's models.
//!
//! Every field is optional or defaulted, so an item the server describes in less detail than
//! expected still reads, with the missing parts left empty, rather than failing the whole page.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::escape;
use crate::{
    Album, ArtistRef, CoverSize, Credit, Encoding, Genre, LyricsLine, LyricsWord, Playlist,
    ReleaseType, SavedArtist, Track, UserProfile, Voice, iso_8601_to_epoch,
};

/// Jellyfin counts time in ticks of 100 nanoseconds.
pub const TICKS_PER_MICRO: u64 = 10;
/// The edge of a list cover, in pixels.
pub const COVER: u32 = 300;
/// The edge of a large cover, wide enough for the fullscreen player on a 2x display.
pub const COVER_LARGE: u32 = 1200;
/// The album artist Jellyfin and most taggers credit a compilation to.
const VARIOUS: &str = "Various Artists";

/// The answer to `POST /Users/AuthenticateByName` and `POST /Users/AuthenticateWithQuickConnect`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Authentication {
    pub user: Option<User>,
    pub access_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct User {
    pub id: Option<String>,
    pub name: Option<String>,
    pub primary_image_tag: Option<String>,
}

/// One page of `GET /Items` and the listings shaped like it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Items {
    pub items: Vec<Item>,
    pub total_record_count: Option<usize>,
}

/// The parts of a `BaseItemDto` that Sonora reads, for tracks, albums, artists, playlists and
/// genres alike.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artist: Option<String>,
    pub album_artists: Vec<NameId>,
    pub artist_items: Vec<NameId>,
    pub artists: Vec<String>,
    pub run_time_ticks: Option<u64>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub production_year: Option<i32>,
    pub premiere_date: Option<String>,
    pub date_created: Option<String>,
    pub child_count: Option<u32>,
    pub song_count: Option<u32>,
    pub album_count: Option<u32>,
    pub image_tags: HashMap<String, String>,
    pub album_primary_image_tag: Option<String>,
    pub normalization_gain: Option<f32>,
    pub overview: Option<String>,
    pub user_data: Option<UserData>,
    /// The entry's own id inside a playlist, which removing it from the playlist names.
    pub playlist_item_id: Option<String>,
    pub genres: Vec<String>,
    pub studios: Vec<NameId>,
    pub people: Vec<Person>,
    pub media_streams: Vec<MediaStream>,
    pub container: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NameId {
    pub name: Option<String>,
    pub id: Option<String>,
}

/// What the server keeps about one item for the signed-in user.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserData {
    pub play_count: Option<u64>,
    pub last_played_date: Option<String>,
}

/// Someone credited on an item, such as its composer.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Person {
    pub name: Option<String>,
    pub id: Option<String>,
    #[serde(rename = "Type")]
    pub kind: Option<String>,
}

/// One stream of a file, of which Sonora reads the audio one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub codec: Option<String>,
    pub bit_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
}

/// The answer to `GET /Playlists/{playlistId}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaylistInfo {
    pub open_access: bool,
    pub shares: Vec<Share>,
}

/// A user a playlist is shared with, and whether they may change it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Share {
    pub user_id: Option<String>,
    pub can_edit: bool,
}

/// The answer to `GET /Playlists/{playlistId}/Users/{userId}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Permission {
    pub can_edit: bool,
}

/// The answer to `POST /Playlists`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Created {
    pub id: String,
}

/// The answer to `GET /Audio/{itemId}/Lyrics`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricsAnswer {
    pub metadata: LyricsMetadata,
    pub lyrics: Vec<LyricLine>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricsMetadata {
    /// Ticks to move every line by, from the file's `[offset:]` tag.
    pub offset: Option<i64>,
}

/// One line of lyrics. A line without a start belongs to unsynced lyrics.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LyricLine {
    pub text: String,
    pub start: Option<u64>,
    pub cues: Vec<Cue>,
}

/// One timed word of a line. The positions count UTF-16 units into the line's text, as .NET
/// strings do.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Cue {
    pub position: usize,
    pub end_position: usize,
    pub start: u64,
    pub end: Option<u64>,
}

/// The answers of the Quick Connect sign-in, `POST /QuickConnect/Initiate` and
/// `GET /QuickConnect/Connect`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QuickConnect {
    pub secret: String,
    pub code: String,
    pub authenticated: bool,
}

impl Item {
    /// Whether the server lists this item as an audio track, the only kind Sonora plays.
    pub fn is_audio(&self) -> bool {
        self.kind.as_deref() == Some("Audio")
    }

    pub fn duration(&self) -> Duration {
        ticks(self.run_time_ticks.unwrap_or(0))
    }

    /// The audio stream's codec and its figures, as the signal path shows them.
    pub fn encoding(&self) -> Option<Encoding> {
        let stream = self
            .media_streams
            .iter()
            .find(|stream| stream.kind.as_deref() == Some("Audio"))?;
        let kbps = stream.bit_rate.map_or(0, |rate| rate / 1000);
        let depth = stream.bit_depth.unwrap_or(16);
        Some(
            match stream.codec.as_deref()?.to_ascii_lowercase().as_str() {
                "flac" => Encoding::Flac {
                    depth,
                    rate: stream.sample_rate.unwrap_or(44_100),
                },
                "alac" => Encoding::Alac { depth },
                codec if codec.starts_with("pcm") => Encoding::Pcm { depth },
                "aac" => Encoding::Aac { kbps },
                "vorbis" => Encoding::Vorbis { kbps },
                "mp3" => Encoding::Mp3 { kbps },
                "opus" => Encoding::Opus { kbps },
                _ => Encoding::Other,
            },
        )
    }

    fn primary_tag(&self) -> Option<&str> {
        self.image_tags
            .get("Primary")
            .map(String::as_str)
            .filter(|tag| !tag.is_empty())
    }
}

/// Builds the picture urls of one server. Jellyfin serves item images without a token, so a
/// cover url carries nothing secret and stays the same across launches, which lets every image
/// cache keep it.
#[derive(Clone, Debug)]
pub struct Covers {
    server: String,
}

impl Covers {
    pub fn new(server: &str) -> Self {
        Self {
            server: server.trim_end_matches('/').to_owned(),
        }
    }

    /// `GET /Items/{itemId}/Images/Primary`, scaled down to `size` when there is one. The tag
    /// names the image's version, so a replaced cover gets a new url. It comes first, so the
    /// size can be taken off and put back by [`Self::resized`].
    pub fn url(&self, id: &str, tag: &str, size: Option<u32>) -> String {
        let mut url = format!(
            "{}/Items/{}/Images/Primary?tag={}",
            self.server,
            escape::component(id),
            escape::component(tag),
        );
        if let Some(size) = size {
            url.push_str(&format!("&maxWidth={size}&maxHeight={size}"));
        }
        url
    }

    /// One of this server's picture urls at another size, or `None` for a url it never made.
    pub fn resized(&self, cover: &str, size: CoverSize) -> Option<String> {
        let rest = cover.strip_prefix(&self.server)?.strip_prefix("/Items/")?;
        let (_, query) = rest.split_once("/Images/Primary?tag=")?;
        let base = &cover[..cover.len() - query.len()];
        let tag = query.split('&').next()?;
        Some(match size {
            CoverSize::Large => {
                format!("{base}{tag}&maxWidth={COVER_LARGE}&maxHeight={COVER_LARGE}")
            }
            CoverSize::Full => format!("{base}{tag}"),
        })
    }

    /// The item's own picture, or for a track without one, its album's.
    pub fn of(&self, item: &Item, size: u32) -> Option<String> {
        if let Some(tag) = item.primary_tag() {
            return Some(self.url(&item.id, tag, Some(size)));
        }
        let album = item.album_id.as_deref().filter(|id| !id.is_empty())?;
        let tag = item
            .album_primary_image_tag
            .as_deref()
            .filter(|tag| !tag.is_empty())?;
        Some(self.url(album, tag, Some(size)))
    }

    /// The account's picture, `GET /Users/{userId}/Images/Primary`, when it has one.
    pub fn avatar(&self, user_id: &str, tag: Option<&str>) -> Option<String> {
        let tag = tag.filter(|tag| !tag.is_empty())?;
        Some(format!(
            "{}/Users/{}/Images/Primary?tag={}&maxWidth={COVER}&maxHeight={COVER}",
            self.server,
            escape::component(user_id),
            escape::component(tag),
        ))
    }

    pub fn track(&self, item: Item) -> Track {
        let cover = self.of(&item, COVER);
        let (artists, artist_refs) = artists_of(&item.artist_items, &item.artists);
        let credits = credits(&item, &artist_refs);
        Track {
            id: Some(item.id.clone()),
            name: item.name.clone().unwrap_or_default(),
            playable: true,
            artists,
            artist_refs,
            album: item.album.clone().unwrap_or_default(),
            album_artist: item.album_artist.clone().unwrap_or_default(),
            album_id: item.album_id.clone().filter(|id| !id.is_empty()),
            cover,
            duration: item.duration(),
            added_at: iso_8601_to_epoch(item.date_created.as_deref()),
            added_by: None,
            playcount: item.user_data.as_ref().and_then(|data| data.play_count),
            popularity: 0,
            explicit: false,
            track_number: item.index_number.unwrap_or(0),
            disc_number: item.parent_index_number.unwrap_or(1).max(1),
            tags: item.genres,
            languages: Vec::new(),
            credits,
        }
    }

    /// An album as Sonora shows it. A tag that holds only a year reaches Jellyfin as the first
    /// of January, so a release dated then shows as its year alone.
    pub fn album(&self, item: Item) -> Album {
        let cover = self.of(&item, COVER);
        let cover_large = self.of(&item, COVER_LARGE);
        let names = item.album_artist.iter().cloned().collect::<Vec<_>>();
        let (artists, artist_refs) = match item.album_artists.is_empty() {
            true => artists_of(&item.artist_items, &names),
            false => artists_of(&item.album_artists, &names),
        };
        let year = item.production_year.unwrap_or(0);
        let release_type = match artists == VARIOUS {
            true => ReleaseType::Compilation,
            false => ReleaseType::Album,
        };
        Album {
            id: item.id,
            name: item.name.unwrap_or_default(),
            artists,
            artist_refs,
            cover,
            cover_large,
            release_type,
            year,
            track_count: item.song_count.or(item.child_count).unwrap_or(0),
            release_date: item
                .premiere_date
                .as_deref()
                .and_then(|date| date.get(..10))
                .filter(|date| !date.ends_with("-01-01"))
                .map(str::to_owned)
                .unwrap_or_else(|| match year {
                    0 => String::new(),
                    _ => year.to_string(),
                }),
            label: item
                .studios
                .iter()
                .find_map(|studio| studio.name.clone())
                .unwrap_or_default(),
            copyrights: Vec::new(),
            added_at: iso_8601_to_epoch(item.date_created.as_deref()),
        }
    }

    pub fn artist(&self, item: Item) -> SavedArtist {
        SavedArtist {
            cover: self.of(&item, COVER),
            id: item.id,
            name: item.name.unwrap_or_default(),
            added_at: None,
        }
    }

    pub fn artist_large(&self, item: &Item) -> Option<String> {
        self.of(item, COVER_LARGE)
    }

    /// A playlist as a listing shows it. Who may change it comes from the playlist's own
    /// permissions, which a listing does not carry, so the caller fills those in.
    pub fn playlist(&self, item: Item) -> Playlist {
        Playlist {
            cover: self.of(&item, COVER),
            id: item.id,
            name: item.name.unwrap_or_default(),
            owner: String::new(),
            owner_id: String::new(),
            owned: false,
            collaborative: false,
            blend: false,
            public: false,
            track_count: item.child_count.or(item.song_count).unwrap_or(0),
            modified_at: None,
        }
    }

    pub fn genre(&self, item: Item) -> Genre {
        Genre {
            cover: self.of(&item, COVER),
            id: item.id,
            name: item.name.unwrap_or_default(),
        }
    }
}

pub fn profile(user_id: &str, username: &str, avatar: Option<String>) -> UserProfile {
    UserProfile {
        id: user_id.to_owned(),
        display_name: username.to_owned(),
        avatar,
    }
}

/// Whether `name` is the stand-in compilations are credited to, which is nobody to recommend.
pub fn is_various(name: &str) -> bool {
    name.eq_ignore_ascii_case(VARIOUS)
}

/// A span of ticks as a duration.
pub fn ticks(ticks: u64) -> Duration {
    Duration::from_micros(ticks / TICKS_PER_MICRO)
}

/// A duration as ticks, the unit every position sent to the server is in.
pub fn to_ticks(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros())
        .unwrap_or(u64::MAX)
        .saturating_mul(TICKS_PER_MICRO)
}

/// The synced lines of an answer, each with its words when the file timed them. `None` when
/// the lyrics carry no times, which the caller reads as plain text.
pub fn synced(answer: &LyricsAnswer) -> Option<Vec<LyricsLine>> {
    let offset = answer.metadata.offset.unwrap_or(0);
    let shift = |at: u64| ticks(at.saturating_add_signed(-offset));
    if answer.lyrics.iter().any(|line| line.start.is_none()) {
        return None;
    }
    Some(
        answer
            .lyrics
            .iter()
            .map(|line| {
                let units: Vec<u16> = line.text.encode_utf16().collect();
                let words: Vec<LyricsWord> = line
                    .cues
                    .iter()
                    .filter_map(|cue| {
                        let text = units.get(cue.position..cue.end_position.min(units.len()))?;
                        Some(LyricsWord {
                            start: shift(cue.start),
                            end: shift(cue.end.unwrap_or(cue.start)),
                            text: String::from_utf16_lossy(text),
                        })
                    })
                    .collect();
                LyricsLine {
                    start: shift(line.start.unwrap_or(0)),
                    end: None,
                    text: line.text.trim().to_owned(),
                    romanized: None,
                    words: (!words.is_empty()).then_some(words),
                    secondary: Vec::new(),
                    voice: Voice::Lead,
                }
            })
            .collect(),
    )
}

/// The display line and the followable artists of an item. Jellyfin lists artists with ids
/// where it has matched them to an artist item, and as bare names otherwise.
fn artists_of(items: &[NameId], names: &[String]) -> (String, Vec<ArtistRef>) {
    let refs: Vec<ArtistRef> = match items.is_empty() {
        false => items
            .iter()
            .filter_map(|artist| {
                let name = artist.name.clone().filter(|name| !name.is_empty())?;
                Some(ArtistRef {
                    name,
                    id: artist.id.clone().filter(|id| !id.is_empty()),
                    channel: false,
                })
            })
            .collect(),
        true => names
            .iter()
            .filter(|name| !name.is_empty())
            .map(|name| ArtistRef {
                name: name.clone(),
                id: None,
                channel: false,
            })
            .collect(),
    };
    let line = refs
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (line, refs)
}

/// Who wrote and made a track, read from the people the server lists on it. The song page
/// credits the track's artists by itself while there are no credits, so the artists lead the
/// list only once someone else is named.
fn credits(item: &Item, artists: &[ArtistRef]) -> Vec<Credit> {
    let others: Vec<Credit> = item
        .people
        .iter()
        .filter_map(|person| {
            let role = match person.kind.as_deref()? {
                "Composer" => "Composer",
                "Lyricist" => "Lyricist",
                "Arranger" => "Arranger",
                "Conductor" => "Conductor",
                "Producer" => "Producer",
                "Mixer" => "Mixer",
                "Engineer" => "Engineer",
                "Remixer" => "Remixer",
                _ => return None,
            };
            Some(Credit {
                name: person.name.clone().filter(|name| !name.is_empty())?,
                role: role.to_owned(),
                id: None,
            })
        })
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    artists
        .iter()
        .map(|artist| Credit {
            name: artist.name.clone(),
            role: "Main artist".to_owned(),
            id: artist.id.clone(),
        })
        .chain(others)
        .collect()
}
