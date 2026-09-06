use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use librespot_core::Session;
use serde::Deserialize;

use super::query;
use crate::{Album, AlbumDetail, ArtistRef, ReleaseType, Track};

const PAGE_LIMIT: usize = 50;
const ALBUM_PREFIX: &str = "spotify:album:";
const ARTIST_PREFIX: &str = "spotify:artist:";
const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "albumUnion")]
    album: Option<PathAlbum>,
}

#[derive(Deserialize)]
struct PathAlbum {
    uri: String,
    name: String,
    #[serde(default)]
    label: String,
    #[serde(rename = "type")]
    kind: String,
    date: Option<PathDate>,
    #[serde(default)]
    artists: Artists,
    #[serde(rename = "coverArt", default)]
    cover: Cover,
    #[serde(default)]
    copyright: Copyrights,
    #[serde(rename = "tracksV2")]
    tracks: AlbumTracks,
}

#[derive(Deserialize)]
struct PathDate {
    #[serde(rename = "isoString")]
    iso: String,
    precision: String,
}

#[derive(Default, Deserialize)]
struct Artists {
    #[serde(default)]
    items: Vec<PathArtist>,
}

#[derive(Deserialize)]
struct PathArtist {
    uri: String,
    profile: Profile,
}

#[derive(Deserialize)]
struct Profile {
    name: String,
}

#[derive(Default, Deserialize)]
struct Cover {
    #[serde(default)]
    sources: Vec<Image>,
}

#[derive(Deserialize)]
struct Image {
    height: u32,
    url: String,
}

#[derive(Default, Deserialize)]
struct Copyrights {
    #[serde(default)]
    items: Vec<Copyright>,
}

#[derive(Deserialize)]
struct Copyright {
    text: String,
}

#[derive(Deserialize)]
struct AlbumTracks {
    #[serde(default)]
    items: Vec<AlbumItem>,
    #[serde(rename = "totalCount")]
    total_count: usize,
}

#[derive(Deserialize)]
struct AlbumItem {
    track: PathTrack,
}

#[derive(Deserialize)]
struct PathTrack {
    uri: String,
    name: String,
    #[serde(default)]
    artists: Artists,
    #[serde(default)]
    playability: Playability,
    #[serde(rename = "contentRating", default)]
    content_rating: ContentRating,
    #[serde(rename = "discNumber", default)]
    disc_number: u32,
    #[serde(rename = "trackNumber", default)]
    track_number: u32,
    #[serde(default)]
    duration: PathDuration,
    playcount: Option<String>,
}

#[derive(Default, Deserialize)]
struct Playability {
    #[serde(default)]
    playable: bool,
}

#[derive(Default, Deserialize)]
struct ContentRating {
    #[serde(default)]
    label: String,
}

#[derive(Default, Deserialize)]
struct PathDuration {
    #[serde(rename = "totalMilliseconds", default)]
    milliseconds: u64,
}

struct Page {
    album: Album,
    tracks: Vec<Track>,
    items: usize,
    total: usize,
}

pub(crate) async fn album(session: &Session, album_id: &str) -> Result<AlbumDetail> {
    let mut album = None;
    let mut tracks = Vec::new();
    let mut offset = 0;

    loop {
        let variables = serde_json::json!({
            "uri": format!("spotify:album:{album_id}"),
            "locale": "",
            "offset": offset,
            "limit": PAGE_LIMIT,
        });
        let data = query::<Data>(session, "getAlbum", variables).await?;
        let page = page(data)?;
        album.get_or_insert(page.album);
        tracks.extend(page.tracks);

        let Some(next) = next_offset(offset, page.items, page.total) else {
            return Ok(AlbumDetail {
                album: album.context("album Pathfinder response has no album")?,
                tracks,
            });
        };
        offset = next;
    }
}

fn page(data: Data) -> Result<Page> {
    let Some(album) = data.album else {
        return Err(anyhow!("album Pathfinder response has no album"));
    };
    let items = album.tracks.items.len();
    let total = album.tracks.total_count;
    let header = album_from(&album);
    let tracks = album
        .tracks
        .items
        .into_iter()
        .map(|item| track_from(item.track, &header))
        .collect::<Result<_>>()?;
    Ok(Page {
        album: header,
        tracks,
        items,
        total,
    })
}

fn album_from(album: &PathAlbum) -> Album {
    let (artists, artist_refs) = artists(&album.artists);
    let release_date = album.date.as_ref().map(date).unwrap_or_default();
    let year = release_date
        .get(..4)
        .and_then(|year| year.parse().ok())
        .unwrap_or_default();

    Album {
        id: album
            .uri
            .strip_prefix(ALBUM_PREFIX)
            .unwrap_or(&album.uri)
            .to_owned(),
        name: non_empty(&album.name).unwrap_or(UNKNOWN).to_owned(),
        artists,
        artist_refs,
        cover: cover(&album.cover.sources, false),
        cover_large: cover(&album.cover.sources, true),
        release_type: release_type(&album.kind),
        year,
        track_count: album.tracks.total_count as u32,
        release_date,
        label: album.label.clone(),
        copyrights: album
            .copyright
            .items
            .iter()
            .filter_map(|copyright| non_empty(&copyright.text).map(str::to_owned))
            .collect(),
        added_at: None,
    }
}

fn track_from(track: PathTrack, album: &Album) -> Result<Track> {
    let (artists, artist_refs) = artists(&track.artists);
    let playcount = track
        .playcount
        .map(|count| {
            count.parse().with_context(|| {
                let id = track.uri.strip_prefix(TRACK_PREFIX).unwrap_or(&track.uri);
                format!("invalid album play count for track {id}")
            })
        })
        .transpose()?
        .and_then(super::reported);

    Ok(Track {
        id: track.uri.strip_prefix(TRACK_PREFIX).map(str::to_owned),
        name: non_empty(&track.name).unwrap_or(UNKNOWN).to_owned(),
        playable: track.playability.playable,
        artists,
        artist_refs,
        album: album.name.clone(),
        album_id: Some(album.id.clone()),
        cover: album.cover.clone(),
        cover_large: album.cover_large.clone(),
        duration: Duration::from_millis(track.duration.milliseconds),
        added_at: None,
        added_by: None,
        playcount,
        popularity: 0,
        explicit: track.content_rating.label == "EXPLICIT",
        track_number: track.track_number,
        disc_number: track.disc_number,
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

fn artists(artists: &Artists) -> (String, Vec<ArtistRef>) {
    let refs: Vec<_> = artists
        .items
        .iter()
        .filter_map(|artist| {
            let name = non_empty(&artist.profile.name)?.to_owned();
            Some(ArtistRef {
                name,
                id: artist.uri.strip_prefix(ARTIST_PREFIX).map(str::to_owned),
            })
        })
        .collect();
    let names = refs
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (names, refs)
}

fn cover(sources: &[Image], large: bool) -> Option<String> {
    const HEADER: u32 = 300;

    let source = match large {
        true => sources
            .iter()
            .filter(|source| source.height >= HEADER)
            .min_by_key(|source| source.height)
            .or_else(|| sources.iter().max_by_key(|source| source.height)),
        false => sources.iter().min_by_key(|source| source.height),
    }?;
    non_empty(&source.url).map(str::to_owned)
}

fn date(date: &PathDate) -> String {
    let length = match date.precision.as_str() {
        "YEAR" => 4,
        "MONTH" => 7,
        _ => 10,
    };
    date.iso.get(..length).unwrap_or(&date.iso).to_owned()
}

pub(super) fn release_type(kind: &str) -> ReleaseType {
    match kind {
        "SINGLE" => ReleaseType::Single,
        "COMPILATION" => ReleaseType::Compilation,
        "EP" => ReleaseType::Ep,
        "AUDIOBOOK" => ReleaseType::Audiobook,
        "PODCAST" => ReleaseType::Podcast,
        _ => ReleaseType::Album,
    }
}

fn next_offset(offset: usize, items: usize, total: usize) -> Option<usize> {
    let loaded = offset.saturating_add(items);
    (items > 0 && loaded < total).then_some(loaded)
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_album_page() {
        let body = br#"{"albumUnion":{"uri":"spotify:album:album1","name":"The Poison","label":"Label","type":"ALBUM","date":{"isoString":"2005-01-01T00:00:00Z","precision":"YEAR"},"artists":{"items":[{"profile":{"name":"Artist"},"uri":"spotify:artist:artist1"}]},"coverArt":{"sources":[{"height":64,"url":"small"},{"height":640,"url":"large"}]},"copyright":{"items":[{"text":"Copyright"}]},"tracksV2":{"items":[{"track":{"uri":"spotify:track:abc123","name":"Song","artists":{"items":[{"profile":{"name":"Artist"},"uri":"spotify:artist:artist1"}]},"playability":{"playable":true},"contentRating":{"label":"EXPLICIT"},"discNumber":1,"trackNumber":2,"duration":{"totalMilliseconds":141453},"playcount":"462537503"}}],"totalCount":2}}}"#;
        let data: Data = serde_json::from_slice(body).unwrap();
        let page = page(data).unwrap();
        assert_eq!(page.album.id, "album1");
        assert_eq!(page.album.release_date, "2005");
        assert_eq!(page.album.cover.as_deref(), Some("small"));
        assert_eq!(page.album.cover_large.as_deref(), Some("large"));
        assert_eq!(page.items, 1);
        assert_eq!(page.total, 2);
        assert_eq!(page.tracks[0].id.as_deref(), Some("abc123"));
        assert_eq!(page.tracks[0].playcount, Some(462_537_503));
        assert!(page.tracks[0].playable);
        assert!(page.tracks[0].explicit);
    }

    #[test]
    fn maps_track_uri_to_playcount() {
        let track = PathTrack {
            uri: "spotify:track:base62id".to_owned(),
            name: "Song".to_owned(),
            artists: Artists::default(),
            playability: Playability::default(),
            content_rating: ContentRating::default(),
            disc_number: 1,
            track_number: 1,
            duration: PathDuration::default(),
            playcount: Some("1234".to_owned()),
        };
        let album = Album {
            id: "album".to_owned(),
            name: "Album".to_owned(),
            artists: String::new(),
            artist_refs: Vec::new(),
            cover: None,
            cover_large: None,
            release_type: ReleaseType::Album,
            year: 0,
            track_count: 1,
            release_date: String::new(),
            label: String::new(),
            copyrights: Vec::new(),
            added_at: None,
        };
        let track = track_from(track, &album).unwrap();
        assert_eq!(track.id.as_deref(), Some("base62id"));
        assert_eq!(track.playcount, Some(1_234));
    }

    #[test]
    fn advances_page_offset() {
        assert_eq!(next_offset(0, 50, 101), Some(50));
        assert_eq!(next_offset(50, 50, 101), Some(100));
        assert_eq!(next_offset(100, 1, 101), None);
        assert_eq!(next_offset(50, 0, 101), None);
    }
}
