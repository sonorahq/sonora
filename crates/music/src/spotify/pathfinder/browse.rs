use anyhow::{Context as _, Result, bail};
use librespot_core::Session;
use serde::{Deserialize, Deserializer};

use super::query;
use crate::{
    Album, ArtistRef, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, Playlist, ReleaseType,
    SavedArtist,
};

const PAGE_PREFIX: &str = "spotify:page:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const ALBUM_PREFIX: &str = "spotify:album:";
const ARTIST_PREFIX: &str = "spotify:artist:";
const SHOW_ART: &str = "ab676563";
const INTEGRATION: &str = "INTEGRATION_WEB_PLAYER";
const SECTIONS: u32 = 20;
const ITEMS: u32 = 10;
const CARDS: u32 = 99;
/// The untitled grid at the top of Spotify's home, holding what the user played lately.
const RECENTS: &str = "HomeShortsSectionData";
/// A titled home shelf. The single-card feed sections and the Recents list are other kinds.
const SHELF: &str = "HomeGenericSectionData";
const ZONE: &str = "UTC";

#[derive(Deserialize)]
struct Start {
    #[serde(rename = "browseStart")]
    start: Option<Container>,
}

#[derive(Deserialize)]
struct Page {
    browse: Option<Container>,
}

#[derive(Deserialize)]
struct Home {
    home: Option<HomeData>,
}

#[derive(Deserialize)]
struct HomeData {
    #[serde(rename = "sectionContainer")]
    container: Option<Container>,
}

#[derive(Deserialize)]
struct Container {
    header: Option<Header>,
    #[serde(default, deserialize_with = "nullable")]
    sections: Sections,
}

#[derive(Deserialize)]
struct Header {
    title: Option<Label>,
}

#[derive(Default, Deserialize)]
struct Sections {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Section>,
}

#[derive(Deserialize)]
struct Section {
    #[serde(default, deserialize_with = "nullable")]
    data: SectionData,
    #[serde(rename = "sectionItems", default, deserialize_with = "nullable")]
    items: Items,
}

#[derive(Default, Deserialize)]
struct SectionData {
    #[serde(rename = "__typename", default, deserialize_with = "nullable")]
    kind: String,
    title: Option<Label>,
}

#[derive(Default, Deserialize)]
struct Items {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default, deserialize_with = "nullable")]
    uri: String,
    content: Option<Content>,
}

#[derive(Deserialize)]
struct Content {
    data: Option<Entity>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum Entity {
    Playlist(WirePlaylist),
    Album(WireAlbum),
    Artist(WireArtist),
    BrowseSectionContainer(WireCard),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct WireArtist {
    profile: Option<Profile>,
    visuals: Option<Visuals>,
}

#[derive(Deserialize)]
struct Visuals {
    #[serde(rename = "avatarImage", default, deserialize_with = "nullable")]
    avatar: Artwork,
}

#[derive(Deserialize)]
struct WirePlaylist {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    images: Images,
    #[serde(rename = "ownerV2")]
    owner: Option<Owner>,
}

#[derive(Deserialize)]
struct WireAlbum {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(rename = "coverArt", default, deserialize_with = "nullable")]
    cover: Artwork,
    #[serde(default, deserialize_with = "nullable")]
    artists: Artists,
}

#[derive(Deserialize)]
struct WireCard {
    data: Option<CardData>,
}

#[derive(Deserialize)]
struct CardData {
    #[serde(rename = "cardRepresentation")]
    card: Option<Representation>,
}

#[derive(Deserialize)]
struct Representation {
    title: Option<Label>,
    #[serde(default, deserialize_with = "nullable")]
    artwork: Artwork,
}

#[derive(Deserialize)]
struct Owner {
    data: Option<OwnerData>,
}

#[derive(Deserialize)]
struct OwnerData {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
}

#[derive(Default, Deserialize)]
struct Artists {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Artist>,
}

#[derive(Deserialize)]
struct Artist {
    #[serde(default, deserialize_with = "nullable")]
    uri: String,
    profile: Option<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
}

#[derive(Default, Deserialize)]
struct Images {
    #[serde(default, deserialize_with = "nullable")]
    items: Vec<Artwork>,
}

#[derive(Default, Deserialize)]
struct Artwork {
    #[serde(default, deserialize_with = "nullable")]
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    url: String,
}

#[derive(Deserialize)]
struct Label {
    #[serde(rename = "transformedLabel", default, deserialize_with = "nullable")]
    label: String,
}

fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

pub(crate) async fn all(session: &Session) -> Result<Vec<Genre>> {
    let variables = serde_json::json!({
        "pagePagination": { "offset": 0, "limit": ITEMS },
        "sectionPagination": { "offset": 0, "limit": CARDS },
        "browseEndUserIntegration": INTEGRATION,
    });
    cards(query::<Start>(session, "browseAll", variables).await?)
}

pub(crate) async fn page(session: &Session, genre_id: &str) -> Result<GenreDetail> {
    let variables = serde_json::json!({
        "uri": format!("{PAGE_PREFIX}{genre_id}"),
        "pagePagination": { "offset": 0, "limit": SECTIONS },
        "sectionPagination": { "offset": 0, "limit": ITEMS },
        "browseEndUserIntegration": INTEGRATION,
        "includeEpisodeContentRatingsV2": false,
    });
    detail(query::<Page>(session, "browsePage", variables).await?)
}

/// Spotify's own home feed, the one the web player opens on. Fails when the feed holds
/// neither recents nor a shelf this module can read.
pub(crate) async fn home(session: &Session) -> Result<HomeFeed> {
    let zone = jiff::tz::TimeZone::system();
    let variables = serde_json::json!({
        "homeEndUserIntegration": INTEGRATION,
        "timeZone": zone.iana_name().unwrap_or(ZONE),
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": ITEMS,
        "includeEpisodeContentRatingsV2": false,
    });
    feed(query::<Home>(session, "home", variables).await?)
}

fn feed(data: Home) -> Result<HomeFeed> {
    let container = data
        .home
        .and_then(|home| home.container)
        .context("home Pathfinder response has no sections")?;

    let mut feed = HomeFeed::default();
    for shelf in container.sections.items {
        match shelf.data.kind.as_str() {
            RECENTS if feed.listen_again.is_empty() => {
                feed.listen_again = shelf.items.items.into_iter().filter_map(item).collect();
            }
            SHELF => feed.sections.extend(section(shelf)),
            _ => {}
        }
    }
    if feed.listen_again.is_empty() && feed.sections.is_empty() {
        bail!("home Pathfinder response has no readable sections");
    }
    Ok(feed)
}

fn cards(data: Start) -> Result<Vec<Genre>> {
    let start = data
        .start
        .context("browseAll Pathfinder response has no browse")?;

    Ok(start
        .sections
        .items
        .into_iter()
        .flat_map(|section| section.items.items)
        .filter_map(|item| match item.content?.data? {
            Entity::BrowseSectionContainer(card) => genre(&item.uri, card),
            _ => None,
        })
        .collect())
}

fn detail(data: Page) -> Result<GenreDetail> {
    let browse = data
        .browse
        .context("browsePage Pathfinder response has no browse")?;
    let header = browse.header;

    Ok(GenreDetail {
        name: header
            .and_then(|header| header.title)
            .map(|title| title.label)
            .unwrap_or_default(),
        sections: browse
            .sections
            .items
            .into_iter()
            .filter_map(section)
            .collect(),
    })
}

fn section(section: Section) -> Option<GenreSection> {
    let title = section
        .data
        .title
        .map(|title| title.label)
        .unwrap_or_default();
    let items: Vec<GenreItem> = section.items.items.into_iter().filter_map(item).collect();

    (!items.is_empty()).then_some(GenreSection { title, items })
}

fn item(item: Item) -> Option<GenreItem> {
    let Item { uri, content } = item;
    match content?.data? {
        Entity::Playlist(playlist) => Some(GenreItem::Playlist(Playlist {
            id: trimmed(&uri, PLAYLIST_PREFIX)?,
            name: playlist.name,
            owner: playlist
                .owner
                .and_then(|owner| owner.data)
                .map(|data| data.name)
                .unwrap_or_default(),
            owner_id: String::new(),
            owned: false,
            collaborative: false,
            blend: false,
            public: true,
            cover: playlist.images.items.first().and_then(image),
            track_count: 0,
            modified_at: None,
        })),
        Entity::Album(album) => {
            let artists: Vec<ArtistRef> = album
                .artists
                .items
                .into_iter()
                .filter_map(|artist| {
                    Some(ArtistRef {
                        name: artist.profile?.name,
                        id: trimmed(&artist.uri, ARTIST_PREFIX),
                        channel: false,
                    })
                })
                .collect();
            Some(GenreItem::Album(Album {
                id: trimmed(&uri, ALBUM_PREFIX)?,
                name: album.name,
                artists: joined(&artists),
                artist_refs: artists,
                cover: image(&album.cover),
                cover_large: None,
                release_type: ReleaseType::Album,
                year: 0,
                track_count: 0,
                release_date: String::new(),
                label: String::new(),
                copyrights: Vec::new(),
                added_at: None,
            }))
        }
        Entity::Artist(artist) => Some(GenreItem::Artist(SavedArtist {
            id: trimmed(&uri, ARTIST_PREFIX)?,
            name: artist.profile?.name,
            cover: artist.visuals.and_then(|visuals| image(&visuals.avatar)),
            added_at: None,
        })),
        Entity::BrowseSectionContainer(card) => genre(&uri, card).map(GenreItem::Genre),
        Entity::Unknown => None,
    }
}

fn genre(uri: &str, container: WireCard) -> Option<Genre> {
    let id = trimmed(uri, PAGE_PREFIX)?;
    let card = container.data?.card?;

    let cover = image(&card.artwork).filter(|url| !shows(url))?;

    Some(Genre {
        id,
        name: card.title.map(|title| title.label).unwrap_or_default(),
        cover: Some(cover),
    })
}

fn shows(cover: &str) -> bool {
    cover
        .rsplit('/')
        .next()
        .is_some_and(|file| file.starts_with(SHOW_ART))
}

fn image(artwork: &Artwork) -> Option<String> {
    artwork
        .sources
        .first()
        .map(|source| source.url.clone())
        .filter(|url| !url.is_empty())
}

fn joined(artists: &[ArtistRef]) -> String {
    artists
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn trimmed(uri: &str, prefix: &str) -> Option<String> {
    uri.strip_prefix(prefix)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}
