use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{SpotifyLyrics, search};
use crate::lyrics::{catalog, lrc};
use crate::{
    Lyrics, LyricsContributor, LyricsHit, LyricsLane, LyricsLine, LyricsProvider, LyricsQuery,
    LyricsRole, LyricsWord, Voice, escape,
};

const SOURCE: &str = "Spicy Lyrics";
const ENDPOINT: &str = "https://api.spicylyrics.org/v1/lyrics";
/// The environment variable holding the developer key, `sl_sk_…` or `sl_pk_…`.
const KEY_ENV: &str = "SPICY_LYRICS_KEY";
/// The source the catalogue names for its own community sheets.
const COMMUNITY: &str = "spicy_lyrics";
/// Hand-timed sheets, so they rank like the Apple catalogue and are never reshaped.
const TRUST: u32 = 180;
const TIMEOUT: Duration = Duration::from_secs(15);
/// The terms let a response be kept for 30 days. A day less leaves room for a clock that is off.
const SHELF_LIFE: Duration = Duration::from_secs(29 * 24 * 60 * 60);
/// How much of a refused answer's body to keep in the error, enough to name the reason.
const BODY: usize = 300;
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sonorahq/sonora)"
);

/// The Spicy Lyrics developer API, which answers with the best sync it has for a Spotify track
/// id: its own community sheets, or one it relays from another catalogue. It needs a key of the
/// user's own in `SPICY_LYRICS_KEY`, and a Spotify sign-in only to find the id of a song played
/// from another service.
pub struct SpicyLyrics {
    spotify: SpotifyLyrics,
    http: reqwest::Client,
    key: Option<String>,
}

impl SpicyLyrics {
    pub fn new(spotify: SpotifyLyrics, key: Option<String>) -> Self {
        Self {
            spotify,
            http: reqwest::Client::new(),
            key: key.filter(|key| !key.trim().is_empty()),
        }
    }

    pub fn from_env() -> Self {
        Self::new(SpotifyLyrics::from_env(), std::env::var(KEY_ENV).ok())
    }

    /// The answer for one Spotify track, or `None` when the catalogue has nothing for it.
    async fn answer(&self, key: &str, track_id: &str) -> Result<Option<Body>> {
        let response = self
            .http
            .get(format!("{ENDPOINT}/{}", escape::component(track_id)))
            .timeout(TIMEOUT)
            .header("User-Agent", AGENT)
            .bearer_auth(key.trim())
            .send()
            .await
            .context("cannot reach the spicy lyrics catalogue")?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let reason: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let reason: String = reason.chars().take(BODY).collect();
            anyhow::bail!("the spicy lyrics catalogue answered with status {status}: {reason}");
        }
        let envelope: Envelope = response
            .json()
            .await
            .context("cannot read the spicy lyrics response")?;
        let about: About = serde_json::from_value(envelope.body.clone())
            .context("cannot decode the spicy lyrics attribution")?;
        let sheet = serde_json::from_value(envelope.body)
            .context("cannot decode the spicy lyrics sheet")?;
        Ok(Some(Body { about, sheet }))
    }

    async fn hit(
        &self,
        key: &str,
        track_id: &str,
        mut hit: LyricsHit,
    ) -> Result<Option<LyricsHit>> {
        let Some(Body { about, sheet }) = self.answer(key, track_id).await? else {
            return Ok(None);
        };
        let (lyrics, writers) = sheet.lyrics();
        if lyrics.is_empty() {
            return Ok(None);
        }
        hit.trust = TRUST;
        hit.lyrics = lyrics;
        hit.writers = writers;
        hit.origin = about.origin();
        hit.contributors = about.contributors();
        hit.expires = Some(SystemTime::now() + SHELF_LIFE);
        Ok(Some(hit))
    }
}

impl Default for SpicyLyrics {
    fn default() -> Self {
        Self::from_env()
    }
}

#[async_trait]
impl LyricsProvider for SpicyLyrics {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(key) = self.key.as_deref() else {
            log::debug!("lyrics: spicy lyrics has no key, set {KEY_ENV}");
            return Ok(Vec::new());
        };
        if let Some(id) = query.id_for("spotify") {
            let hit = catalog::hit(SOURCE, query, Lyrics::plain(String::new()));
            return Ok(self.hit(key, id, hit).await?.into_iter().collect());
        }
        let Some(session) = self.spotify.session().await? else {
            return Ok(Vec::new());
        };
        let tracks = search::search(&session, &format!("{} {}", query.title, query.artist)).await?;
        for (id, hit) in catalog::candidates(SOURCE, query, tracks) {
            if let Some(hit) = self.hit(key, &id, hit).await? {
                return Ok(vec![hit]);
            }
        }
        Ok(Vec::new())
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "Body")]
    body: Value,
}

struct Body {
    about: About,
    sheet: Sheet,
}

/// Who to credit for an answer: the catalogue it came from, and for a community sheet the people
/// who made and uploaded it.
#[derive(Deserialize)]
struct About {
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "UploadAttribution", default)]
    attribution: Option<Attribution>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Attribution {
    #[serde(default)]
    uploader: Option<Person>,
    #[serde(default)]
    maker: Option<Person>,
}

#[derive(Deserialize)]
struct Person {
    #[serde(default)]
    username: String,
    #[serde(default)]
    url: Option<String>,
}

impl About {
    /// The relayed catalogue's name, `None` for a community sheet and an empty name when the
    /// answer does not say, which the credit shows as unknown.
    fn origin(&self) -> Option<String> {
        let source = self.source.as_deref().map(str::trim).unwrap_or_default();
        let name = match source {
            COMMUNITY => return None,
            "apple_music" => "Apple Music",
            "spotify" => "Spotify",
            "musixmatch" => "Musixmatch",
            "youtube_music" => "YouTube Music",
            "deezer" => "Deezer",
            "tidal" => "Tidal",
            other => other,
        };
        Some(name.to_owned())
    }

    fn contributors(&self) -> Vec<LyricsContributor> {
        if self.source.as_deref() != Some(COMMUNITY) {
            return Vec::new();
        }
        let Some(attribution) = &self.attribution else {
            return Vec::new();
        };
        [
            (LyricsRole::Maker, &attribution.maker),
            (LyricsRole::Uploader, &attribution.uploader),
        ]
        .into_iter()
        .filter_map(|(role, person)| {
            let person = person.as_ref()?;
            let name = person.username.trim();
            (!name.is_empty()).then(|| LyricsContributor {
                role,
                name: name.to_owned(),
                url: person.url.clone().filter(|url| url.starts_with("https://")),
            })
        })
        .collect()
    }
}

/// A sheet as Spicy Lyrics stores it: times in seconds, one of three timing kinds.
#[derive(Deserialize)]
#[serde(tag = "Type")]
enum Sheet {
    Syllable {
        #[serde(rename = "Content", default)]
        content: Vec<Verse>,
        #[serde(rename = "SongWriters", default)]
        writers: Vec<String>,
    },
    Line {
        #[serde(rename = "Content", default)]
        content: Vec<Row>,
        #[serde(rename = "SongWriters", default)]
        writers: Vec<String>,
    },
    Static {
        #[serde(rename = "Lines", default)]
        lines: Vec<Plain>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Verse {
    #[serde(default)]
    lead: Option<Group>,
    #[serde(default)]
    background: Vec<Group>,
    #[serde(default)]
    opposite_aligned: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Group {
    start_time: f64,
    end_time: f64,
    #[serde(default)]
    syllables: Vec<Syllable>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Syllable {
    #[serde(default)]
    text: String,
    start_time: f64,
    end_time: f64,
    #[serde(default)]
    is_part_of_word: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Row {
    #[serde(default)]
    text: String,
    start_time: f64,
    end_time: f64,
    #[serde(default)]
    opposite_aligned: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Plain {
    #[serde(default)]
    text: String,
}

impl Sheet {
    fn lyrics(self) -> (Lyrics, Vec<String>) {
        let (mut lines, writers) = match self {
            Sheet::Static { lines } => {
                let text = lines
                    .iter()
                    .map(|line| line.text.trim())
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                return (Lyrics::plain(text), Vec::new());
            }
            Sheet::Syllable { content, writers } => (
                content.iter().filter_map(verse).collect::<Vec<_>>(),
                writers,
            ),
            Sheet::Line { content, writers } => (content.iter().filter_map(row).collect(), writers),
        };
        lrc::normalize(&mut lines);
        let lyrics = match lines.is_empty() {
            true => Lyrics::plain(String::new()),
            false => Lyrics::Synced {
                lines: lines.into(),
            },
        };
        (lyrics, writers)
    }
}

fn verse(verse: &Verse) -> Option<LyricsLine> {
    let lead = verse.lead.as_ref()?;
    let words = words(&lead.syllables);
    let text = joined(&words);
    if text.is_empty() {
        return None;
    }
    let start = stamp(lead.start_time)?;
    let secondary = verse.background.iter().filter_map(lane).collect();
    Some(LyricsLine {
        start,
        end: stamp(lead.end_time).map(|end| end.max(start)),
        text,
        romanized: None,
        words: Some(words),
        secondary,
        voice: voice(verse.opposite_aligned),
    })
}

fn lane(group: &Group) -> Option<LyricsLane> {
    let words = words(&group.syllables);
    let text = joined(&words);
    if text.is_empty() {
        return None;
    }
    let start = stamp(group.start_time)?;
    Some(LyricsLane {
        start,
        end: stamp(group.end_time).map(|end| end.max(start)),
        text,
        romanized: None,
        words: Some(words),
    })
}

fn row(row: &Row) -> Option<LyricsLine> {
    let text = row.text.trim();
    if text.is_empty() || text == "♪" {
        return None;
    }
    let start = stamp(row.start_time)?;
    Some(LyricsLine {
        start,
        end: stamp(row.end_time).filter(|end| *end > start),
        text: text.to_owned(),
        romanized: None,
        words: None,
        secondary: Vec::new(),
        voice: voice(row.opposite_aligned),
    })
}

/// Syllables become words. A syllable that is not part of a longer word carries the space that
/// separates it from the next, the way TTML sheets keep it.
fn words(syllables: &[Syllable]) -> Vec<LyricsWord> {
    let last = syllables.len().saturating_sub(1);
    syllables
        .iter()
        .enumerate()
        .filter_map(|(index, syllable)| {
            let start = stamp(syllable.start_time)?;
            let end = stamp(syllable.end_time).unwrap_or(start).max(start);
            let mut text = syllable.text.clone();
            if !syllable.is_part_of_word && index < last && !text.ends_with(' ') {
                text.push(' ');
            }
            (!text.trim().is_empty()).then_some(LyricsWord { start, end, text })
        })
        .collect()
}

fn joined(words: &[LyricsWord]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn voice(opposite: bool) -> Voice {
    match opposite {
        true => Voice::Counter,
        false => Voice::Lead,
    }
}

fn stamp(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(seconds.max(0.)).ok()
}
