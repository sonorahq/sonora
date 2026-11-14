use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Track;
use crate::cover::{CoverLimits, square_jpeg};

const ALBUMS: &str = "https://api.deezer.com/search/album";
const TRACKS: &str = "https://api.deezer.com/search/track";
const RESULTS: &str = "10";
const ITUNES: &str = "https://itunes.apple.com/search";
/// The size iTunes names in its thumbnail urls, and the size asked for in its place.
const ITUNES_THUMB: &str = "100x100bb";
const ITUNES_LARGE: &str = "600x600bb";
const MUSICBRAINZ: &str = "https://musicbrainz.org/ws/2/release-group";
const MUSICBRAINZ_RESULTS: &str = "5";
/// The lowest score MusicBrainz may give a release group for its art to be trusted.
const MUSICBRAINZ_SCORE: u32 = 90;
const COVER_ART: &str = "https://coverartarchive.org/release-group";
/// Uguu, a temporary host: anonymous uploads served as plain images, gone after three hours.
const UGUU: &str = "https://uguu.se/upload";
/// The scheme the local provider gives its covers, folder art and pictures extracted from tags.
const FILE_URL: &str = "file://";
/// How long the fetch of a provider's cover, or its upload, may take before it counts as failed
/// and the search answers instead.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// What an uploaded cover is shrunk to before it leaves the machine.
const UPLOAD_LIMITS: CoverLimits = CoverLimits {
    side: 512,
    bytes: 1024 * 1024,
};
/// MusicBrainz refuses a client that does not say who it is.
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sonorahq/sonora)"
);

/// Finds cover art by name in public catalogues that need no account and serve every image from
/// a public https url: Deezer first, then iTunes, then MusicBrainz with the Cover Art Archive. A
/// catalogue that lacks the album, or refuses or cannot be reached, hands over to the next. It is
/// for a track whose own art cannot be handed to someone else, so a miss is `None` rather than an
/// error.
#[derive(Clone, Default)]
pub struct ArtworkSearch {
    http: reqwest::Client,
}

/// What a lookup searches for. A track with an album is looked up by the album, so every track
/// of one album shares a query and a result. A track without one is looked up by its title.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtworkQuery {
    artist: String,
    album: String,
    title: String,
    /// The album or title as the track names it, brackets and all, so an edition the track does
    /// not name is never taken for it.
    wanted: String,
}

#[derive(Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct FoundAlbum {
    title: String,
    artist: Named,
    cover_xl: Option<String>,
}

#[derive(Deserialize)]
struct FoundTrack {
    title: String,
    artist: Named,
    album: Covered,
}

/// A catalogue `ArtworkSearch` asks, in the order it asks them.
#[derive(Clone, Copy, Debug)]
enum Source {
    Deezer,
    Itunes,
    MusicBrainz,
}

impl Source {
    const ALL: [Self; 3] = [Self::Deezer, Self::Itunes, Self::MusicBrainz];
}

/// Uguu's answer to an upload.
#[derive(Deserialize)]
struct Uploaded {
    success: bool,
    #[serde(default)]
    files: Vec<UploadedFile>,
}

#[derive(Deserialize)]
struct UploadedFile {
    url: String,
}

#[derive(Deserialize)]
struct ItunesPage {
    #[serde(default)]
    results: Vec<ItunesFound>,
}

#[derive(Deserialize)]
struct ItunesFound {
    #[serde(rename = "artistName")]
    artist: String,
    #[serde(rename = "collectionName")]
    album: Option<String>,
    #[serde(rename = "trackName")]
    title: Option<String>,
    #[serde(rename = "artworkUrl100")]
    artwork: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseGroups {
    #[serde(rename = "release-groups", default)]
    groups: Vec<ReleaseGroup>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    id: String,
    title: String,
    #[serde(default)]
    score: u32,
    #[serde(rename = "artist-credit", default)]
    credit: Vec<Named>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Covered {
    cover_xl: Option<String>,
}

impl ArtworkQuery {
    /// The query for `track`, or `None` when it has no artist or nothing else to search by.
    pub fn for_track(track: &Track) -> Option<Self> {
        let artist = track
            .artist_refs
            .first()
            .map(|artist| artist.name.as_str())
            .or_else(|| track.artists.split(", ").next())
            .map(searchable)
            .unwrap_or_default();
        let album = searchable(&track.album);
        let (title, wanted) = match album.is_empty() {
            true => (searchable(&track.name), track.name.trim().to_owned()),
            false => (String::new(), track.album.trim().to_owned()),
        };
        if artist.is_empty() || album.is_empty() && title.is_empty() {
            return None;
        }
        Some(Self {
            artist,
            album,
            title,
            wanted,
        })
    }
}

impl ArtworkSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The url of a cover for `query`, or `None` when no catalogue has a result that names the
    /// same artist and a close enough album or title to trust. A wrong cover is worse than none.
    /// It fails only when every catalogue failed, so an outage is never taken for a miss.
    pub async fn find(&self, query: &ArtworkQuery) -> Result<Option<String>> {
        let mut answered = false;
        let mut failure = None;
        for source in Source::ALL {
            let found = match source {
                Source::Deezer => self.deezer(query).await,
                Source::Itunes => self.itunes(query).await,
                // MusicBrainz knows albums only, so a query by title never asks it
                Source::MusicBrainz if query.album.is_empty() => continue,
                Source::MusicBrainz => self.musicbrainz(query).await,
            };
            match found {
                Ok(Some(cover)) => return Ok(Some(cover)),
                Ok(None) => answered = true,
                Err(error) => {
                    log::debug!("artwork: {source:?} failed: {error:#}");
                    failure.get_or_insert(error);
                }
            }
        }
        match (answered, failure) {
            (false, Some(error)) => Err(error),
            _ => Ok(None),
        }
    }

    /// The cover Deezer has for `query`.
    async fn deezer(&self, query: &ArtworkQuery) -> Result<Option<String>> {
        match query.album.is_empty() {
            false => {
                let found: Vec<FoundAlbum> = self.search(ALBUMS, &query.album, query).await?;
                let candidates = found
                    .into_iter()
                    .filter(|album| same_artist(&album.artist.name, &query.artist))
                    .map(|album| (album.title, album.cover_xl));
                Ok(closest(candidates, &query.wanted, true))
            }
            true => {
                let found: Vec<FoundTrack> = self.search(TRACKS, &query.title, query).await?;
                let candidates = found
                    .into_iter()
                    .filter(|track| same_artist(&track.artist.name, &query.artist))
                    .map(|track| (track.title, track.album.cover_xl));
                Ok(closest(candidates, &query.wanted, false))
            }
        }
    }

    /// The cover iTunes has for `query`, asked for at a larger size than its search answers with.
    async fn itunes(&self, query: &ArtworkQuery) -> Result<Option<String>> {
        let (entity, name) = match query.album.is_empty() {
            false => ("album", &query.album),
            true => ("song", &query.title),
        };
        let terms = format!("{} {name}", query.artist);
        let page: ItunesPage = self
            .http
            .get(ITUNES)
            .query(&[
                ("term", terms.as_str()),
                ("entity", entity),
                ("limit", RESULTS),
            ])
            .send()
            .await
            .context("cannot reach itunes")?
            .error_for_status()
            .context("cannot search itunes")?
            .json()
            .await
            .context("cannot read the itunes search")?;
        let candidates = page
            .results
            .into_iter()
            .filter(|found| same_artist(&found.artist, &query.artist))
            .map(|found| {
                let title = match query.album.is_empty() {
                    false => found.album,
                    true => found.title,
                };
                let cover = found
                    .artwork
                    .map(|artwork| artwork.replace(ITUNES_THUMB, ITUNES_LARGE));
                (title.unwrap_or_default(), cover)
            });
        Ok(closest(candidates, &query.wanted, !query.album.is_empty()))
    }

    /// The front cover the Cover Art Archive holds for the release group MusicBrainz matches to
    /// the album of `query`.
    async fn musicbrainz(&self, query: &ArtworkQuery) -> Result<Option<String>> {
        let terms = format!(
            "releasegroup:\"{}\" AND artist:\"{}\"",
            quoted(&query.album),
            quoted(&query.artist)
        );
        let found: ReleaseGroups = self
            .http
            .get(MUSICBRAINZ)
            .header(reqwest::header::USER_AGENT, AGENT)
            .query(&[
                ("query", terms.as_str()),
                ("fmt", "json"),
                ("limit", MUSICBRAINZ_RESULTS),
            ])
            .send()
            .await
            .context("cannot reach musicbrainz")?
            .error_for_status()
            .context("cannot search musicbrainz")?
            .json()
            .await
            .context("cannot read the musicbrainz search")?;
        let candidates = found
            .groups
            .into_iter()
            .filter(|group| group.score >= MUSICBRAINZ_SCORE)
            .filter(|group| {
                group
                    .credit
                    .first()
                    .is_some_and(|artist| same_artist(&artist.name, &query.artist))
            })
            .map(|group| {
                (
                    group.title,
                    Some(format!("{COVER_ART}/{}/front-500", group.id)),
                )
            });
        let Some(cover) = closest(candidates, &query.wanted, true) else {
            return Ok(None);
        };

        // the archive answers a group without art with 404, and a found one with a redirect to
        // the image, whose final address is the one that never moves again
        let response = self
            .http
            .head(&cover)
            .header(reqwest::header::USER_AGENT, AGENT)
            .send()
            .await
            .context("cannot reach the cover art archive")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .context("cannot read the cover art archive")?;
        let url = response.url().as_str();
        Ok(url.starts_with("https://").then(|| url.to_owned()))
    }

    /// Reads the picture at `source`, a provider's own cover url or a local file's `file://` one,
    /// and uploads it to Uguu for three hours, answering the public url there. Only the
    /// re-encoded picture leaves the machine, never `source`, which for a self-hosted server
    /// carries the credentials that fetch it.
    pub async fn upload(&self, source: &str) -> Result<String> {
        let picture = match source.strip_prefix(FILE_URL) {
            Some(path) => {
                let path = path.to_owned();
                tokio::task::spawn_blocking(move || std::fs::read(path))
                    .await
                    .context("cannot read the cover file")?
                    .context("cannot read the cover file")?
            }
            None => self.fetch(source).await?,
        };
        let jpeg = square_jpeg(&picture, UPLOAD_LIMITS)?;

        let boundary = format!("sonora{:016x}", fastrand::u64(..));
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"files[]\"; filename=\"cover.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(&jpeg);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let answer: Uploaded = self
            .http
            .post(UGUU)
            .timeout(UPLOAD_TIMEOUT)
            .header(reqwest::header::USER_AGENT, AGENT)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .context("cannot reach uguu")?
            .error_for_status()
            .context("cannot upload the cover to uguu")?
            .json()
            .await
            .context("cannot read the uguu answer")?;
        let url = answer
            .files
            .into_iter()
            .next()
            .map(|file| file.url)
            .filter(|url| answer.success && url.starts_with("https://"));
        url.context("uguu answered without a url")
    }

    /// The bytes behind a provider's cover url. Every error drops the url, so the credentials in
    /// it never reach a log.
    async fn fetch(&self, source: &str) -> Result<Vec<u8>> {
        let picture = self
            .http
            .get(source)
            .timeout(UPLOAD_TIMEOUT)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("cannot reach the cover")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)
            .context("cannot fetch the cover")?
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)
            .context("cannot read the cover")?;
        Ok(picture.to_vec())
    }

    /// One page of results for the artist and `name`. Deezer reports a refused search, such as
    /// one over its rate limit, in the body of a successful response.
    async fn search<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        name: &str,
        query: &ArtworkQuery,
    ) -> Result<Vec<T>> {
        let terms = format!("{} {name}", query.artist);
        let response = self
            .http
            .get(endpoint)
            .query(&[("q", terms.as_str()), ("limit", RESULTS)])
            .send()
            .await
            .context("cannot reach deezer")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("deezer answered with status {status}");
        }
        let page: Page<T> = response
            .json()
            .await
            .context("cannot read the deezer search")?;
        if let Some(error) = page.error {
            anyhow::bail!("deezer refused the search: {error}");
        }
        Ok(page.data)
    }
}

/// The cover of the candidate whose name is closest to `wanted`, the album or title as the track
/// names it. Names are compared without their brackets, and a candidate that matches exactly
/// beats one that only adds words, such as "- EP". A candidate whose brackets name an edition
/// `wanted` does not, such as "(blind)", is never taken, since it may wear another cover. When
/// `album` is set the same holds the other way, so "Album" never stands in for "Album (Deluxe)".
fn closest(
    candidates: impl Iterator<Item = (String, Option<String>)>,
    wanted: &str,
    album: bool,
) -> Option<String> {
    let words = compared(wanted);
    let words: Vec<&str> = words.split(' ').collect();
    let edition = match album {
        true => bracketed(wanted),
        false => Vec::new(),
    };
    let wanted = compared(&searchable(wanted));
    let mut near = None;
    for (name, cover) in candidates {
        let Some(cover) = cover.filter(|cover| cover.starts_with("https://")) else {
            continue;
        };
        let named = compared(&name);
        let named: Vec<&str> = named.split(' ').collect();
        let other_edition = bracketed(&name)
            .iter()
            .any(|word| !words.contains(&word.as_str()))
            || edition.iter().any(|word| !named.contains(&word.as_str()));
        if other_edition {
            continue;
        }
        let core = compared(&searchable(&name));
        if core == wanted {
            return Some(cover);
        }
        if near.is_none() && extends(&core, &wanted) {
            near = Some(cover);
        }
    }
    near
}

fn same_artist(found: &str, wanted: &str) -> bool {
    let found = compared(found);
    let wanted = compared(wanted);
    found == wanted || extends(&found, &wanted)
}

/// Whether one of two compared names is the other with whole words added after it.
fn extends(one: &str, other: &str) -> bool {
    let starts = |long: &str, short: &str| {
        !short.is_empty()
            && long
                .strip_prefix(short)
                .is_some_and(|rest| rest.starts_with(' '))
    };
    starts(one, other) || starts(other, one)
}

/// `text` without anything in brackets, since Deezer wants every word of a search to match and
/// "(Official Video)" or "[Remastered]" would rule out the release itself.
fn searchable(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut depth = 0usize;
    for character in text.chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(character),
            _ => {}
        }
    }
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` made safe inside a quoted Lucene term, which MusicBrainz searches are.
fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The words inside the brackets of `text`, compared the way names are.
fn bracketed(text: &str) -> Vec<String> {
    let mut inside = String::new();
    let mut depth = 0usize;
    for character in text.chars() {
        match character {
            '(' | '[' => {
                depth += 1;
                inside.push(' ');
            }
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => inside.push(character),
            _ => {}
        }
    }
    compared(&inside)
        .split(' ')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `text` lowercased, with every run of punctuation and spaces turned into one space.
fn compared(text: &str) -> String {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}
