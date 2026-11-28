//! Finding a listen in MusicBrainz, for the services whose records link a play to its recording,
//! release and artist there.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::Play;

const SEARCH: &str = "https://musicbrainz.org/ws/2/recording";
const RELEASES: &str = "https://musicbrainz.org/ws/2/release";
const RESULTS: &str = "10";
/// How many releases of one recording are weighed for the one a play links to.
const RELEASE_RESULTS: &str = "50";
/// MusicBrainz refuses a client that does not say who it is.
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sonorahq/sonora)"
);
/// MusicBrainz allows one request a second from a client, and answers 503 to one that sends more.
const SPACING: Duration = Duration::from_secs(1);
/// How long to wait before asking again after MusicBrainz said it was busy, which is what its
/// `Retry-After` asks for, and how many times to ask before giving up. Under load it sheds about
/// half of all searches.
const BUSY_WAIT: Duration = Duration::from_secs(1);
const ATTEMPTS: usize = 5;
const REQUEST_WAIT: Duration = Duration::from_secs(10);
/// How far the length of a recording may stray from the play's before it counts as another take.
const LENGTH_SLACK: Duration = Duration::from_secs(10);
/// How many recent answers are kept, so a now-playing report and the listen that follows it ask
/// once between them.
const REMEMBERED: usize = 32;

/// What MusicBrainz knows a listen as. The ids are bare UUIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub recording: String,
    pub release: Option<String>,
    /// The id of the credited artist whose name is the play's artist.
    pub artist: Option<String>,
    pub isrc: Option<String>,
}

/// The recording `play` is, found by its ISRC when the provider gave one, and otherwise by a search
/// on its title and artist that only takes a recording whose title, artist and length agree with
/// the play. `None` when nothing matches or MusicBrainz cannot be asked, since the listen is sent
/// either way.
pub async fn identify(play: &Play) -> Option<Identity> {
    let key = Key::of(play);
    if let Some(known) = recall(&key) {
        return known;
    }
    match lookup(play).await {
        Ok(found) => {
            remember(key, found.clone());
            found
        }
        Err(error) => {
            log::info!("scrobble: cannot find the listen in musicbrainz: {error:#}");
            None
        }
    }
}

/// Which play an answer belongs to.
#[derive(Clone, PartialEq, Eq)]
struct Key {
    artist: String,
    title: String,
    album: Option<String>,
    isrc: Option<String>,
    seconds: u64,
}

impl Key {
    fn of(play: &Play) -> Self {
        Self {
            artist: play.artist.clone(),
            title: play.title.clone(),
            album: play.album.clone(),
            isrc: play.isrc.clone(),
            seconds: play.duration.as_secs(),
        }
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize)]
struct Recording {
    id: String,
    title: String,
    /// The length in milliseconds, absent when nobody entered one.
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    credit: Vec<Credit>,
    #[serde(default)]
    releases: Vec<Release>,
    #[serde(default)]
    isrcs: Vec<String>,
}

#[derive(Deserialize)]
struct Credit {
    /// The name as this recording credits it, which may differ from the artist's own.
    name: String,
    artist: CreditedArtist,
}

#[derive(Deserialize)]
struct CreditedArtist {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Releases {
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    id: String,
    title: String,
    status: Option<String>,
    /// What the Cover Art Archive holds for the release. Only a release lookup or browse says, a
    /// search leaves it out.
    #[serde(rename = "cover-art-archive")]
    art: Option<Art>,
}

#[derive(Deserialize)]
struct Art {
    #[serde(default)]
    front: bool,
}

/// The answer kept for `key`, `Some(None)` when MusicBrainz was asked and knew nothing.
fn recall(key: &Key) -> Option<Option<Identity>> {
    let remembered = answers().lock().ok()?;
    remembered
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, found)| found.clone())
}

fn remember(key: Key, found: Option<Identity>) {
    let Ok(mut remembered) = answers().lock() else {
        return;
    };
    remembered.retain(|(known, _)| *known != key);
    if remembered.len() >= REMEMBERED {
        remembered.pop_front();
    }
    remembered.push_back((key, found));
}

fn answers() -> &'static Mutex<VecDeque<(Key, Option<Identity>)>> {
    static ANSWERS: Mutex<VecDeque<(Key, Option<Identity>)>> = Mutex::new(VecDeque::new());
    &ANSWERS
}

/// Asks by ISRC first, then by title and artist. An ISRC MusicBrainz does not know falls through
/// to the search, since many recordings there carry none.
async fn lookup(play: &Play) -> Result<Option<Identity>> {
    if let Some(isrc) = play.isrc.as_deref().filter(|isrc| !isrc.is_empty()) {
        let found = recordings(&format!("isrc:\"{}\"", quoted(isrc))).await?;
        let titles = titles(&play.title);
        let chosen = found
            .iter()
            .find(|recording| titles.contains(&fold(&recording.title)))
            .or_else(|| found.first());
        if let Some(recording) = chosen {
            return Ok(Some(identity(recording, play).await));
        }
    }

    let titles = titles(&play.title);
    if titles.is_empty() {
        return Ok(None);
    }
    let terms = titles
        .iter()
        .map(|title| format!("recording:\"{}\"", quoted(title)))
        .collect::<Vec<_>>()
        .join(" OR ");
    let query = format!("({terms}) AND artist:\"{}\"", quoted(&play.artist));
    let found = recordings(&query).await?;
    let Some(recording) = found
        .iter()
        .find(|recording| matches(recording, play, &titles))
    else {
        return Ok(None);
    };
    Ok(Some(identity(recording, play).await))
}

/// The recordings a search finds, best scored first.
async fn recordings(query: &str) -> Result<Vec<Recording>> {
    let found: Found = ask(
        SEARCH,
        &[("query", query), ("fmt", "json"), ("limit", RESULTS)],
    )
    .await
    .context("cannot search musicbrainz")?;
    Ok(found.recordings)
}

/// Every release a recording appears on, with whether each has front art.
async fn releases(recording: &str) -> Result<Vec<Release>> {
    let found: Releases = ask(
        RELEASES,
        &[
            ("recording", recording),
            ("fmt", "json"),
            ("limit", RELEASE_RESULTS),
        ],
    )
    .await
    .context("cannot list the releases of a musicbrainz recording")?;
    Ok(found.releases)
}

/// Sends one request, waiting its turn under MusicBrainz's rate limit and asking again while the
/// server says it is busy.
async fn ask<T: DeserializeOwned>(url: &str, query: &[(&str, &str)]) -> Result<T> {
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(BUSY_WAIT).await;
        }
        turn().await;
        let answer = super::http()
            .get(url)
            .header(reqwest::header::USER_AGENT, AGENT)
            .query(query)
            .timeout(REQUEST_WAIT)
            .send()
            .await
            .context("cannot reach musicbrainz")?;
        if answer.status() == StatusCode::SERVICE_UNAVAILABLE {
            continue;
        }
        return answer
            .error_for_status()
            .context("musicbrainz refused the request")?
            .json()
            .await
            .context("cannot read the musicbrainz answer");
    }
    anyhow::bail!("musicbrainz stayed busy")
}

/// Waits until a second has passed since the last request any lookup sent.
async fn turn() {
    static LAST: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
    let mut last = LAST.lock().await;
    if let Some(sent) = *last {
        let due = sent + SPACING;
        let now = Instant::now();
        if due > now {
            tokio::time::sleep(due - now).await;
        }
    }
    *last = Some(Instant::now());
}

/// Whether a search result is the play rather than a namesake, a live take or another edit.
fn matches(recording: &Recording, play: &Play, titles: &[String]) -> bool {
    let artist = fold(&play.artist);
    let credited = recording
        .credit
        .iter()
        .any(|credit| fold(&credit.name) == artist || fold(&credit.artist.name) == artist);
    let length = match (recording.length, play.duration.is_zero()) {
        (Some(millis), false) => {
            Duration::from_millis(millis).abs_diff(play.duration) <= LENGTH_SLACK
        }
        _ => true,
    };
    credited && length && titles.contains(&fold(&recording.title))
}

/// The ids a matched recording gives the play. The release is looked up among all of the
/// recording's releases, falling back to the few the search listed when that lookup fails.
async fn identity(recording: &Recording, play: &Play) -> Identity {
    let artist = fold(&play.artist);
    let listed = releases(&recording.id).await.unwrap_or_else(|error| {
        log::info!("scrobble: {error:#}");
        Vec::new()
    });
    let release = match listed.is_empty() {
        true => best_release(&recording.releases, play),
        false => best_release(&listed, play),
    };
    Identity {
        recording: recording.id.clone(),
        release,
        artist: recording
            .credit
            .iter()
            .find(|credit| fold(&credit.name) == artist || fold(&credit.artist.name) == artist)
            .map(|credit| credit.artist.id.clone()),
        isrc: play
            .isrc
            .clone()
            .filter(|isrc| !isrc.is_empty())
            .or_else(|| recording.isrcs.first().cloned()),
    }
}

/// The release a play links to, which is where teal.fm takes its cover from. A pseudo-release,
/// which only lists another release under a translated title and seldom has art, comes last.
/// Otherwise the play's own album comes first, then a release with front art, then an official
/// one over a bootleg, and among equals the first MusicBrainz listed.
fn best_release(releases: &[Release], play: &Play) -> Option<String> {
    let album = play.release().map(fold);
    let rank = |release: &Release| {
        (
            release.status.as_deref() != Some("Pseudo-Release"),
            album.as_deref() == Some(fold(&release.title).as_str()),
            release.art.as_ref().is_some_and(|art| art.front),
            release.status.as_deref() == Some("Official"),
        )
    };
    releases
        .iter()
        .rev()
        .max_by_key(|release| rank(release))
        .map(|release| release.id.clone())
}

/// The titles a recording of the play may carry in MusicBrainz, folded: the title as the provider
/// gave it, without a bracketed featured-artist note, and without a ` - ` translation after it,
/// since MusicBrainz credits featured artists apart and keeps a title in its own script.
fn titles(title: &str) -> Vec<String> {
    let bare = unfeatured(title);
    let mut titles = vec![fold(title), fold(&bare)];
    if let Some((before, _)) = bare.split_once(" - ") {
        titles.push(fold(before));
    }
    titles.retain(|title| !title.is_empty());
    titles.dedup();
    titles
}

/// `title` without the bracketed notes that name a featured artist, such as `(feat. X)`.
fn unfeatured(title: &str) -> String {
    let mut kept = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close = match rest.as_bytes()[open] {
            b'(' => ')',
            _ => ']',
        };
        let Some(length) = rest[open..].find(close) else {
            break;
        };
        let inside = rest[open + 1..open + length].trim_start().to_lowercase();
        let featuring = ["feat.", "feat ", "featuring ", "ft.", "ft ", "with "]
            .iter()
            .any(|word| inside.starts_with(word));
        kept.push_str(&rest[..open]);
        if !featuring {
            kept.push_str(&rest[open..=open + length]);
        }
        rest = &rest[open + length + 1..];
    }
    kept.push_str(rest);
    kept
}

/// A name the way two catalogues are compared: lowercase with its spaces collapsed, and with the
/// typographic apostrophes, quotes, dashes and ellipses MusicBrainz prefers spelled the plain way
/// most providers write them.
fn fold(text: &str) -> String {
    let plain = text
        .replace(['’', '‘', 'ʼ', '＇'], "'")
        .replace(['“', '”'], "\"")
        .replace(['‐', '‑', '–', '—'], "-")
        .replace('…', "...");
    plain
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `text` made safe inside a quoted Lucene term, which MusicBrainz searches are.
fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}
