use std::collections::HashSet;

use anyhow::{Context as _, Result};
use librespot_core::Session;
use serde::Deserialize;

use super::query;
use crate::{Album, ArtistRef, ReleaseType};

pub(crate) struct Overview {
    pub(crate) name: String,
    pub(crate) cover_large: Option<String>,
    pub(crate) biography: Option<String>,
    pub(crate) monthly_listeners: Option<u64>,
    pub(crate) tracks: Vec<(String, Option<u64>)>,
    pub(crate) albums: Vec<Album>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "artistUnion")]
    artist: Option<PathArtist>,
}

#[derive(Deserialize)]
struct PathArtist {
    profile: Profile,
    #[serde(default)]
    visuals: Visuals,
    discography: Option<Discography>,
    stats: Option<Stats>,
}

#[derive(Deserialize)]
struct Profile {
    name: String,
    biography: Option<Biography>,
}

#[derive(Deserialize)]
struct Biography {
    text: Option<String>,
}

#[derive(Default, Deserialize)]
struct Visuals {
    #[serde(rename = "avatarImage", default)]
    avatar: Cover,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discography {
    top_tracks: Option<Tracks>,
    #[serde(default)]
    albums: ReleaseGroups,
    #[serde(default)]
    singles: ReleaseGroups,
    #[serde(default)]
    compilations: ReleaseGroups,
}

#[derive(Deserialize)]
struct Tracks {
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    track: PathTrack,
}

#[derive(Deserialize)]
struct PathTrack {
    uri: String,
    playcount: Option<String>,
}

#[derive(Default, Deserialize)]
struct ReleaseGroups {
    #[serde(default)]
    items: Vec<ReleaseGroup>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    releases: Releases,
}

#[derive(Deserialize)]
struct Releases {
    #[serde(default)]
    items: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    date: Option<PathDate>,
    #[serde(rename = "coverArt", default)]
    cover: Cover,
    #[serde(default)]
    label: String,
    #[serde(default)]
    copyright: Copyrights,
    #[serde(default)]
    tracks: ReleaseTracks,
}

#[derive(Deserialize)]
struct PathDate {
    year: i32,
    month: Option<u32>,
    day: Option<u32>,
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

#[derive(Default, Deserialize)]
struct ReleaseTracks {
    #[serde(rename = "totalCount", default)]
    total_count: u32,
}

#[derive(Deserialize)]
struct Stats {
    #[serde(rename = "monthlyListeners")]
    monthly_listeners: Option<u64>,
}

pub(crate) async fn artist(session: &Session, artist_id: &str) -> Result<Overview> {
    let variables = variables(artist_id);
    let data = query::<Data>(session, "queryArtistOverview", variables).await?;
    overview(data, artist_id)
}

fn variables(artist_id: &str) -> serde_json::Value {
    serde_json::json!({
        "uri": format!("spotify:artist:{artist_id}"),
        "locale": "",
        "preReleaseV2": true,
    })
}

fn overview(data: Data, artist_id: &str) -> Result<Overview> {
    let artist = data
        .artist
        .context("artist Pathfinder response has no artist")?;
    let discography = artist
        .discography
        .context("artist Pathfinder response has no discography")?;
    let Discography {
        top_tracks,
        albums,
        singles,
        compilations,
    } = discography;
    let tracks = top_tracks
        .into_iter()
        .flat_map(|tracks| tracks.items)
        .map(|item| {
            let count = item
                .track
                .playcount
                .and_then(|count| count.parse().ok())
                .and_then(super::reported);
            (item.track.uri, count)
        })
        .collect();
    let artist_ref = ArtistRef {
        name: artist.profile.name.clone(),
        id: Some(artist_id.to_owned()),
    };
    let albums = releases(albums, singles, compilations, &artist_ref);

    Ok(Overview {
        name: artist.profile.name,
        cover_large: artist
            .visuals
            .avatar
            .sources
            .iter()
            .max_by_key(|image| image.height)
            .map(|image| image.url.clone()),
        biography: artist
            .profile
            .biography
            .and_then(|biography| biography.text),
        monthly_listeners: artist.stats.and_then(|stats| stats.monthly_listeners),
        tracks,
        albums,
    })
}

fn releases(
    albums: ReleaseGroups,
    singles: ReleaseGroups,
    compilations: ReleaseGroups,
    artist: &ArtistRef,
) -> Vec<Album> {
    let mut seen = HashSet::new();

    albums
        .items
        .into_iter()
        .chain(singles.items)
        .chain(compilations.items)
        .flat_map(|group| group.releases.items)
        .filter(|release| seen.insert(release.id.clone()))
        .map(|release| album(release, artist))
        .collect()
}

fn album(release: Release, artist: &ArtistRef) -> Album {
    let release_date = release.date.as_ref().map(date).unwrap_or_default();
    let year = release
        .date
        .as_ref()
        .map(|date| date.year)
        .unwrap_or_default();

    Album {
        id: release.id,
        name: release.name,
        artists: artist.name.clone(),
        artist_refs: vec![artist.clone()],
        cover: cover(&release.cover.sources, false),
        cover_large: cover(&release.cover.sources, true),
        release_type: release_type(&release.kind),
        year,
        track_count: release.tracks.total_count,
        release_date,
        label: release.label,
        copyrights: release
            .copyright
            .items
            .into_iter()
            .map(|copyright| copyright.text)
            .collect(),
        added_at: None,
    }
}

fn date(date: &PathDate) -> String {
    match (date.month, date.day) {
        (Some(month), Some(day)) => format!("{:04}-{month:02}-{day:02}", date.year),
        (Some(month), None) => format!("{:04}-{month:02}", date.year),
        _ => format!("{:04}", date.year),
    }
}

fn cover(sources: &[Image], large: bool) -> Option<String> {
    let picked = match large {
        true => sources
            .iter()
            .filter(|image| image.height >= 300)
            .min_by_key(|image| image.height)
            .or_else(|| sources.iter().max_by_key(|image| image.height)),
        false => sources.iter().min_by_key(|image| image.height),
    }?;

    Some(picked.url.clone())
}

fn release_type(kind: &str) -> ReleaseType {
    match kind {
        "ALBUM" => ReleaseType::Album,
        "COMPILATION" => ReleaseType::Compilation,
        "EP" => ReleaseType::Ep,
        "AUDIOBOOK" => ReleaseType::Audiobook,
        "PODCAST" => ReleaseType::Podcast,
        _ => ReleaseType::Single,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_overview() {
        let data: Data = serde_json::from_slice(
            br#"{"artistUnion":{"profile":{"name":"Artist","biography":{"text":"About"}},"visuals":{"avatarImage":{"sources":[{"height":640,"url":"large"},{"height":160,"url":"small"},{"height":320,"url":"header"}]}},"discography":{"topTracks":{"items":[{"track":{"uri":"spotify:track:abc","playcount":"57545277"}},{"track":{"uri":"spotify:track:def","playcount":null}}]},"albums":{"items":[{"releases":{"items":[{"id":"album","name":"Release","type":"ALBUM","date":{"day":2,"month":3,"year":2024},"coverArt":{"sources":[{"height":300,"url":"album-large"},{"height":64,"url":"album-small"}]},"label":"Label","copyright":{"items":[{"text":"Copyright"}]},"tracks":{"totalCount":12}}]}}]},"singles":{"items":[]},"compilations":{"items":[]}},"stats":{"monthlyListeners":1900430}}}"#,
        )
        .unwrap();
        let overview = overview(data, "artist").unwrap();

        assert_eq!(overview.name, "Artist");
        assert_eq!(overview.cover_large.as_deref(), Some("large"));
        assert_eq!(overview.biography.as_deref(), Some("About"));
        assert_eq!(overview.monthly_listeners, Some(1_900_430));
        assert_eq!(
            overview.tracks,
            vec![
                ("spotify:track:abc".to_owned(), Some(57_545_277)),
                ("spotify:track:def".to_owned(), None),
            ]
        );
        assert_eq!(overview.albums.len(), 1);
        assert_eq!(overview.albums[0].name, "Release");
        assert_eq!(overview.albums[0].cover.as_deref(), Some("album-small"));
        assert_eq!(
            overview.albums[0].cover_large.as_deref(),
            Some("album-large")
        );
        assert_eq!(overview.albums[0].release_date, "2024-03-02");
        assert_eq!(overview.albums[0].track_count, 12);
        assert_eq!(
            overview.albums[0].artist_refs[0].id.as_deref(),
            Some("artist")
        );
    }

    #[test]
    fn accepts_null_biography() {
        let data: Data = serde_json::from_slice(
            br#"{"artistUnion":{"profile":{"name":"Artist","biography":{"text":null}},"discography":{"topTracks":{"items":[{"track":{"uri":"spotify:track:abc","playcount":"42"}},{"track":{"uri":"spotify:track:def","playcount":"0"}}]}}}}"#,
        )
        .unwrap();
        let overview = overview(data, "artist").unwrap();

        assert_eq!(overview.biography, None);
        assert_eq!(
            overview.tracks,
            vec![
                ("spotify:track:abc".to_owned(), Some(42)),
                ("spotify:track:def".to_owned(), None),
            ]
        );
    }

    #[test]
    fn sends_current_overview_variables() {
        assert_eq!(
            variables("artist1"),
            serde_json::json!({
                "uri": "spotify:artist:artist1",
                "locale": "",
                "preReleaseV2": true,
            })
        );
    }
}
