use anyhow::{Context as _, Result};
use librespot_core::Session;
use serde::Deserialize;

use super::album::release_type;
use super::query;
use crate::{Album, ArtistRef, Playlist, SavedArtist};

const ALBUMS: &str = "searchAlbums";
/// The search page's own query, which answers artists among every other kind. The shared hash
/// service does not list `searchArtists`, so artists come from here.
const DESKTOP: &str = "searchDesktop";
const PLAYLISTS: &str = "searchPlaylists";
const ALBUM_PREFIX: &str = "spotify:album:";
const ARTIST_PREFIX: &str = "spotify:artist:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const HEADER: u32 = 300;
const LIMIT: u32 = 20;

#[derive(Deserialize)]
struct AlbumSearch {
    #[serde(rename = "searchV2")]
    search: Option<AlbumResults>,
}

#[derive(Deserialize)]
struct AlbumResults {
    #[serde(rename = "albumsV2", default)]
    albums: AlbumHits,
}

#[derive(Default, Deserialize)]
struct AlbumHits {
    #[serde(default)]
    items: Vec<AlbumHit>,
}

#[derive(Deserialize)]
struct AlbumHit {
    data: Option<AlbumEntity>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum AlbumEntity {
    Album(WireAlbum),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct WireAlbum {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    date: Option<Year>,
    #[serde(default)]
    artists: Artists,
    #[serde(rename = "coverArt", default)]
    cover: Artwork,
}

#[derive(Deserialize)]
struct ArtistSearch {
    #[serde(rename = "searchV2")]
    search: Option<ArtistResults>,
}

#[derive(Deserialize)]
struct ArtistResults {
    #[serde(default)]
    artists: ArtistHits,
}

#[derive(Default, Deserialize)]
struct ArtistHits {
    #[serde(default)]
    items: Vec<ArtistHit>,
}

#[derive(Deserialize)]
struct ArtistHit {
    data: Option<ArtistEntity>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum ArtistEntity {
    Artist(WireArtist),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct WireArtist {
    #[serde(default)]
    uri: String,
    profile: Option<Profile>,
    visuals: Option<Visuals>,
}

#[derive(Deserialize)]
struct Visuals {
    #[serde(rename = "avatarImage")]
    avatar: Option<Artwork>,
}

#[derive(Deserialize)]
struct PlaylistSearch {
    #[serde(rename = "searchV2")]
    search: Option<PlaylistResults>,
}

#[derive(Deserialize)]
struct PlaylistResults {
    #[serde(default)]
    playlists: PlaylistHits,
}

#[derive(Default, Deserialize)]
struct PlaylistHits {
    #[serde(default)]
    items: Vec<PlaylistHit>,
}

#[derive(Deserialize)]
struct PlaylistHit {
    data: Option<PlaylistEntity>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum PlaylistEntity {
    Playlist(WirePlaylist),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct WirePlaylist {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    images: Images,
    #[serde(rename = "ownerV2")]
    owner: Option<Owner>,
}

#[derive(Deserialize)]
struct Year {
    #[serde(default)]
    year: i32,
}

#[derive(Default, Deserialize)]
struct Artists {
    #[serde(default)]
    items: Vec<Artist>,
}

#[derive(Deserialize)]
struct Artist {
    #[serde(default)]
    uri: String,
    profile: Option<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    #[serde(default)]
    name: String,
}

#[derive(Default, Deserialize)]
struct Images {
    #[serde(default)]
    items: Vec<Artwork>,
}

#[derive(Default, Deserialize)]
struct Artwork {
    #[serde(default)]
    sources: Vec<Image>,
}

#[derive(Deserialize)]
struct Image {
    height: Option<u32>,
    url: String,
}

#[derive(Deserialize)]
struct Owner {
    data: Option<Account>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(default)]
    name: String,
    #[serde(default)]
    username: String,
}

pub(crate) async fn albums(session: &Session, term: &str) -> Result<Vec<Album>> {
    let Some(term) = wanted(term) else {
        return Ok(Vec::new());
    };
    let data = query::<AlbumSearch>(session, ALBUMS, variables(term)).await?;
    let results = data
        .search
        .context("searchAlbums Pathfinder response has no search")?;

    Ok(results.albums.items.into_iter().filter_map(album).collect())
}

pub(crate) async fn playlists(session: &Session, term: &str) -> Result<Vec<Playlist>> {
    let Some(term) = wanted(term) else {
        return Ok(Vec::new());
    };
    let data = query::<PlaylistSearch>(session, PLAYLISTS, variables(term)).await?;
    let results = data
        .search
        .context("searchPlaylists Pathfinder response has no search")?;
    let username = session.username();

    Ok(results
        .playlists
        .items
        .into_iter()
        .filter_map(|hit| playlist(hit, &username))
        .collect())
}

/// The artists Spotify finds for `term`, read from the search page's whole answer with the
/// optional sections turned off.
pub(crate) async fn artists(session: &Session, term: &str) -> Result<Vec<SavedArtist>> {
    let Some(term) = wanted(term) else {
        return Ok(Vec::new());
    };
    let mut asked = variables(term);
    asked["numberOfTopResults"] = 1.into();
    asked["includeAudiobooks"] = false.into();
    asked["includeArtistHasConcertsField"] = false.into();
    asked["includePreReleases"] = false.into();
    asked["includeAlbumPreReleases"] = false.into();
    asked["includeAuthors"] = false.into();
    asked["includeEpisodeContentRatingsV2"] = false.into();
    let data = query::<ArtistSearch>(session, DESKTOP, asked).await?;
    let results = data
        .search
        .context("searchDesktop Pathfinder response has no search")?;

    Ok(results
        .artists
        .items
        .into_iter()
        .filter_map(artist)
        .collect())
}

fn artist(hit: ArtistHit) -> Option<SavedArtist> {
    let ArtistEntity::Artist(artist) = hit.data? else {
        return None;
    };
    let face = artist.visuals.and_then(|visuals| visuals.avatar);

    Some(SavedArtist {
        id: trimmed(&artist.uri, ARTIST_PREFIX)?,
        name: artist.profile?.name,
        cover: face.and_then(|face| cover(&face.sources, false)),
        added_at: None,
    })
}

fn album(hit: AlbumHit) -> Option<Album> {
    let AlbumEntity::Album(album) = hit.data? else {
        return None;
    };
    let (names, refs) = credits(album.artists);
    let year = album.date.map(|date| date.year).unwrap_or_default();

    Some(Album {
        id: trimmed(&album.uri, ALBUM_PREFIX)?,
        name: album.name,
        artists: names,
        artist_refs: refs,
        cover: cover(&album.cover.sources, false),
        cover_large: cover(&album.cover.sources, true),
        release_type: release_type(&album.kind),
        year,
        track_count: 0,
        release_date: match year {
            0 => String::new(),
            year => year.to_string(),
        },
        label: String::new(),
        copyrights: Vec::new(),
        added_at: None,
    })
}

fn playlist(hit: PlaylistHit, username: &str) -> Option<Playlist> {
    let PlaylistEntity::Playlist(playlist) = hit.data? else {
        return None;
    };
    let owner = playlist.owner.and_then(|owner| owner.data);

    Some(Playlist {
        id: trimmed(&playlist.uri, PLAYLIST_PREFIX)?,
        name: playlist.name,
        owned: owner
            .as_ref()
            .is_some_and(|account| account.username == username),
        owner: owner.map(|account| account.name).unwrap_or_default(),
        owner_id: String::new(),
        collaborative: false,
        blend: false,
        public: true,
        cover: playlist
            .images
            .items
            .first()
            .and_then(|artwork| cover(&artwork.sources, false)),
        track_count: 0,
        modified_at: None,
    })
}

fn credits(artists: Artists) -> (String, Vec<ArtistRef>) {
    let refs: Vec<_> = artists
        .items
        .into_iter()
        .filter_map(|artist| {
            Some(ArtistRef {
                name: artist.profile.map(|profile| profile.name)?,
                id: trimmed(&artist.uri, ARTIST_PREFIX),
                channel: false,
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
    let source = match large {
        true => sources
            .iter()
            .filter(|source| height(source) >= HEADER)
            .min_by_key(height)
            .or_else(|| sources.iter().max_by_key(height)),
        false => sources.iter().min_by_key(height),
    }?;
    (!source.url.is_empty()).then(|| source.url.clone())
}

fn height(source: &&Image) -> u32 {
    source.height.unwrap_or_default()
}

fn variables(term: &str) -> serde_json::Value {
    serde_json::json!({
        "searchTerm": term,
        "offset": 0,
        "limit": LIMIT,
    })
}

fn wanted(term: &str) -> Option<&str> {
    Some(term.trim()).filter(|term| !term.is_empty())
}

fn trimmed(uri: &str, prefix: &str) -> Option<String> {
    uri.strip_prefix(prefix)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}
