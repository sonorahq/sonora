use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use http::{HeaderValue, header};
use prost::Message as _;

use super::proto::{
    feeds::{Browse, Heading, ResolvedHome, SearchGenreMetadata, Shelf},
    metadata::{Album as AlbumMessage, Artist as ArtistMessage, ExtensionKind},
};
use super::{albums, artists, collection, playlists, session::Session, transport, wire};
use crate::{BrowseShelf, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed};

const MADE_FOR_YOU: &str = "0JQ5DAt0tbjZptfcdMSKl3";
const ITEMS_PER_SHELF: usize = 10;
const LIKED_SONGS_ART: &str = "https://misc.scdn.co/liked-songs/liked-songs-";

/// Reads the landing shelves and resolves their category cards in one metadata batch.
pub(super) async fn landing(session: &Session) -> Result<Vec<BrowseShelf>> {
    let headers = transport::proto_headers("application/protobuf", None);
    let response: Browse = transport::get(session, "/browsita/v1/browse", headers).await?;
    let structure = response
        .structure
        .context("Spotify Browse has no structure")?;
    let shelves: Vec<_> = structure
        .sections
        .into_iter()
        .filter_map(|section| Some((section.metadata?.uri, section.shelf?)))
        .collect();
    let uris: Vec<_> = shelves
        .iter()
        .flat_map(|(_, shelf)| &shelf.items)
        .filter(|item| {
            item.entity_uri.starts_with("spotify:page:")
                && item.target_uri.starts_with("spotify:genre:")
        })
        .map(|item| item.target_uri.clone())
        .collect();
    let cards = collection::project(
        session,
        &uris,
        ExtensionKind::SearchGenreMetadata,
        collection::BATCH,
        genre_card,
    )
    .await?;

    let shelves: Vec<_> = shelves
        .into_iter()
        .filter_map(|(id, shelf)| {
            let heading = shelf.heading?.text?;
            let genres: Vec<_> = shelf
                .items
                .into_iter()
                .filter_map(|item| cards.get(&item.target_uri).cloned())
                .collect();
            let see_all = heading
                .action
                .and_then(|action| action.uri.strip_prefix("spotify:genre:").map(str::to_owned));
            (!genres.is_empty() || see_all.is_some()).then_some(BrowseShelf {
                id,
                title: heading.title,
                genres,
                see_all,
            })
        })
        .collect();
    ensure!(
        !shelves.is_empty(),
        "Spotify Browse has no readable shelves"
    );
    Ok(shelves)
}

/// Reads a genre page, then loads its ordered sections in one batch.
pub(super) async fn page(session: &Session, id: &str) -> Result<GenreDetail> {
    let (name, sections) = page_sections(session, id).await?;
    let page_uri = format!("spotify:genre:{id}");
    let mut uris: HashSet<_> = sections
        .iter()
        .flat_map(|(_, items)| items.iter().map(String::as_str))
        .collect();
    if name.is_empty() {
        uris.insert(&page_uri);
    }
    let cards = cards(session, uris, true).await?;
    let name = match cards.get(&page_uri) {
        Some(GenreItem::Genre(genre)) if name.is_empty() => genre.name.clone(),
        _ => name,
    };
    Ok(GenreDetail {
        name,
        sections: shelf_cards(sections, &cards),
    })
}

/// Reads the ordered section references and their cards before metadata is resolved.
async fn page_sections(session: &Session, id: &str) -> Result<(String, Vec<SectionUris>)> {
    let root = playlists::browse_sections(session, vec![format!("spotify:genre:{id}")])
        .await
        .context("cannot read Spotify Browse page")?
        .pop()
        .context("Spotify omitted the Browse page")?;
    let section_uris = root
        .items
        .into_iter()
        .map(|(uri, _)| uri)
        .filter(|uri| uri.starts_with("spotify:section:"))
        .collect();
    let sections: Vec<_> = playlists::browse_sections(session, section_uris)
        .await?
        .into_iter()
        .map(section)
        .collect();

    Ok((root.title, sections))
}

/// Reads the personalized shelf URIs that supplement Home.
async fn made_for_you(session: &Session) -> Result<Vec<SectionUris>> {
    Ok(page_sections(session, MADE_FOR_YOU).await?.1)
}

/// A titled shelf of catalogue references in Spotify's order.
type SectionUris = (String, Vec<String>);

/// Builds shelves in their original order and omits those without readable cards.
fn shelf_cards(
    sections: Vec<SectionUris>,
    cards: &HashMap<String, GenreItem>,
) -> Vec<GenreSection> {
    sections
        .into_iter()
        .filter_map(|(title, uris)| {
            let items = resolve_cards(uris, cards);
            (!items.is_empty()).then_some(GenreSection { title, items })
        })
        .collect()
}

/// Looks up ordered card references in a shared set of resolved cards.
fn resolve_cards(uris: Vec<String>, cards: &HashMap<String, GenreItem>) -> Vec<GenreItem> {
    uris.into_iter()
        .filter_map(|uri| cards.get(&uri).cloned())
        .collect()
}

/// Keeps every directory entry and up to ten media cards per shelf.
fn section(section: playlists::BrowseSection) -> (String, Vec<String>) {
    let mut media = 0;
    let items = section
        .items
        .into_iter()
        .filter_map(section_item)
        .filter(|uri| {
            if uri.starts_with("spotify:genre:") {
                return true;
            }
            media += 1;
            media <= ITEMS_PER_SHELF
        })
        .collect();
    (section.title, items)
}

fn section_item((uri, target): (String, Option<String>)) -> Option<String> {
    match uri.split(':').nth(1)? {
        "page" => {
            let id = uri.strip_prefix("spotify:page:")?;
            Some(
                target
                    .filter(|target| target.starts_with("spotify:genre:"))
                    .unwrap_or_else(|| format!("spotify:genre:{id}")),
            )
        }
        "album" | "artist" | "playlist" | "track" => Some(uri),
        _ => None,
    }
}

fn genre_card(uri: &str, data: SearchGenreMetadata) -> Option<Genre> {
    let id = uri.strip_prefix("spotify:genre:")?;
    (!data.name.trim().is_empty()).then(|| Genre {
        id: id.to_owned(),
        name: data.name,
        cover: wire::playlist_image(&data.image_url),
    })
}

/// Resolves playlist cards and one mixed catalogue batch in parallel.
pub(super) async fn cards(
    session: &Session,
    uris: HashSet<&str>,
    own_display_name: bool,
) -> Result<HashMap<String, GenreItem>> {
    let mut playlist_ids = Vec::new();
    let mut catalogue: Vec<(&str, &[ExtensionKind])> = Vec::new();
    for uri in uris {
        match uri
            .strip_prefix("spotify:")
            .and_then(|uri| uri.split_once(':'))
        {
            Some(("playlist", id)) => playlist_ids.push(id.to_owned()),
            Some(("album", _)) => catalogue.push((uri, &[ExtensionKind::AlbumV4])),
            Some(("artist", _)) => catalogue.push((uri, &[ExtensionKind::ArtistV4])),
            Some(("genre", _)) => catalogue.push((uri, &[ExtensionKind::SearchGenreMetadata])),
            Some(("track", _)) => catalogue.push((uri, &[ExtensionKind::TrackV4])),
            _ => {}
        }
    }
    // If sign-in could not load the name, try it in the existing catalogue batch.
    let own = format!("spotify:user:{}", session.username());
    if own_display_name && session.display_name().is_none() && !playlist_ids.is_empty() {
        catalogue.push((&own, &[ExtensionKind::UserProfile]));
    }
    let (playlists, metadata) = tokio::join!(
        playlists::cards(session, &playlist_ids),
        collection::extended_batch(session, &catalogue),
    );
    let mut metadata = metadata?;
    if let Some(data) = collection::take(&mut metadata, ExtensionKind::UserProfile, &own) {
        let profile = super::profiles::contributor(&own, data);
        session.remember_display_name(&profile.name);
    }
    let own_name = session.display_name().unwrap_or_else(|| session.username());
    let mut cards: HashMap<_, _> = playlists
        .into_iter()
        .map(|(uri, mut playlist)| {
            // Spotify publishes this personal collection under its own account.
            let liked_songs = playlist.owner_id == "spotify"
                && playlist
                    .cover
                    .as_deref()
                    .is_some_and(|url| url.starts_with(LIKED_SONGS_ART));
            if own_display_name && (playlist.owner_id == session.username() || liked_songs) {
                own_name.clone_into(&mut playlist.owner);
            }
            (uri, GenreItem::Playlist(playlist))
        })
        .collect();
    for ((kind, uri), raw) in metadata {
        let item = match kind {
            ExtensionKind::AlbumV4 => AlbumMessage::decode(raw)
                .ok()
                .map(|album| GenreItem::Album(albums::album_from(&uri, &album))),
            ExtensionKind::ArtistV4 => ArtistMessage::decode(raw)
                .ok()
                .and_then(|artist| artists::artist_card(&uri, &artist))
                .map(GenreItem::Artist),
            ExtensionKind::SearchGenreMetadata => SearchGenreMetadata::decode(raw)
                .ok()
                .and_then(|data| genre_card(&uri, data))
                .map(GenreItem::Genre),
            ExtensionKind::TrackV4 => super::proto::metadata::Track::decode(raw)
                .ok()
                .map(|track| GenreItem::Track(collection::track_from(&uri, &track))),
            _ => None,
        };
        if let Some(item) = item {
            cards.insert(uri, item);
        }
    }
    Ok(cards)
}

/// Reads Casita Home and adds the distinct Made For You shelves.
pub(super) async fn home(session: &Session) -> Result<HomeFeed> {
    let (casita, made) = tokio::join!(casita(session), made_for_you(session));
    let (shortcuts, sections) = casita?;
    let made = made.unwrap_or_default();
    let unique: HashSet<_> = shortcuts
        .iter()
        .chain(sections.iter().chain(&made).flat_map(|(_, uris)| uris))
        .map(String::as_str)
        .collect();
    let cards = cards(session, unique, true).await?;
    let listen_again = resolve_cards(shortcuts, &cards);
    let sections = shelf_cards(sections, &cards);
    ensure!(
        !listen_again.is_empty() || !sections.is_empty(),
        "Spotify Home has no readable cards"
    );
    let mut feed = HomeFeed {
        listen_again,
        sections,
        quick_picks: None,
    };
    appended(&mut feed, shelf_cards(made, &cards));
    Ok(feed)
}

/// Reads Casita shelf and shortcut references in Spotify's order.
async fn casita(session: &Session) -> Result<(Vec<String>, Vec<SectionUris>)> {
    let mut headers = transport::proto_headers("application/protobuf", None);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/protobuf"),
    );
    let zone = jiff::tz::TimeZone::system();
    let timezone = crate::escape::component(zone.iana_name().unwrap_or("UTC"));
    let path = format!("/casita/v1/home/default?timezone={timezone}");
    let response: ResolvedHome = transport::get(session, &path, headers).await?;
    let feed = response.structure.context("Spotify Home has no sections")?;

    let mut shortcuts = Vec::new();
    let mut sections = Vec::new();
    for section in feed.sections {
        if let Some(shelf) = section.shortcuts {
            shortcuts.extend(uris(shelf));
        }
        if let Some(mut shelf) = section.shelf {
            let title = shelf.heading.take().map(title).unwrap_or_default();
            sections.push((title, uris(shelf)));
        }
        if let Some(single) = section.single
            && let Some(item) = single.item
        {
            let title = single.heading.map(title).unwrap_or_default();
            sections.push((title, vec![item.uri]));
        }
    }

    Ok((shortcuts, sections))
}

fn uris(shelf: Shelf) -> Vec<String> {
    shelf
        .provided
        .and_then(|provided| provided.items)
        .into_iter()
        .flat_map(|items| items.entries)
        .map(|item| item.uri)
        .filter(|uri| uri.starts_with("spotify:"))
        .collect()
}

fn title(heading: Heading) -> String {
    if let Some(text) = heading.text {
        return text.text;
    }
    let Some(enriched) = heading.enriched else {
        return heading
            .with_description
            .map(|heading| heading.title)
            .or_else(|| heading.static_enriched.map(|heading| heading.text))
            .unwrap_or_default();
    };
    enriched
        .entities
        .iter()
        .enumerate()
        .fold(enriched.template, |text, (index, entity)| {
            text.replace(&format!("{{{index}}}"), &entity.name)
        })
}

/// Puts `sections` after the feed's own shelves, leaving out every item a shelf already shows
/// and every shelf whose title is already on the page or that ends up empty.
fn appended(feed: &mut HomeFeed, sections: Vec<GenreSection>) {
    let mut titles: HashSet<String> = feed
        .sections
        .iter()
        .map(|section| section.title.clone())
        .collect();
    let mut shown: HashSet<String> = feed
        .sections
        .iter()
        .flat_map(|section| &section.items)
        .filter_map(shelf_key)
        .collect();
    for mut section in sections {
        if titles.contains(&section.title) {
            continue;
        }
        section
            .items
            .retain(|item| shelf_key(item).is_none_or(|key| shown.insert(key)));
        if !section.items.is_empty() {
            titles.insert(section.title.clone());
            feed.sections.push(section);
        }
    }
}

/// What tells two shelf items apart, unique across kinds.
fn shelf_key(item: &GenreItem) -> Option<String> {
    match item {
        GenreItem::Playlist(playlist) => Some(format!("playlist:{}", playlist.id)),
        GenreItem::Album(album) => Some(format!("album:{}", album.id)),
        GenreItem::Genre(genre) => Some(format!("genre:{}", genre.id)),
        GenreItem::Track(track) => track.id.as_ref().map(|id| format!("track:{id}")),
        GenreItem::Artist(artist) => Some(format!("artist:{}", artist.id)),
    }
}
