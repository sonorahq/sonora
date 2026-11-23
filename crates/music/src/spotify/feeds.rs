use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};

use super::{albums, artists, auth::sign_in_error, collection, playlists, session::Session, wire};
use crate::{Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, Playlist};
use spotifyapis::proto::{
    casita::{Heading, Shelf},
    extended_metadata::{
        Album as AlbumMessage, Artist as ArtistMessage, ExtensionKind, SearchGenreMetadata,
    },
    list_read_service::Attributes,
};

const SPOKEN_WORD_SHELVES: [&str; 2] = [
    "spotify:section:0JQ5DAwD41iU1g70AywV44", // Podcasts
    "spotify:section:0JQ5DAwD41iU1g70AywW88", // Audiobooks
];
const SPOKEN_WORD_DIRECTORIES: [&str; 2] = [
    "spotify:genre:0JQ5DArNBzkmxXHCqFLx3j", // Podcasts
    "spotify:genre:0JQ5DArNBzkmxXHCqFLx3q", // Audiobooks
];
const MADE_FOR_YOU: &str = "0JQ5DAt0tbjZptfcdMSKl3";
const ITEMS_PER_SHELF: usize = 10;
const LIKED_SONGS_ART: &str = "https://misc.scdn.co/liked-songs/liked-songs-";

/// Flattens music shelves and their directory links into cards in feed order.
pub(super) async fn landing(session: &Session) -> Result<Vec<Genre>> {
    let response = session.api().browse().await.map_err(sign_in_error)?;
    let structure = response
        .structure
        .context("Spotify Browse has no structure")?;
    let mut targets = Vec::new();
    let mut directories = HashMap::new();
    for section in structure.sections {
        if section
            .metadata
            .is_some_and(|section| SPOKEN_WORD_SHELVES.contains(&section.uri.as_str()))
        {
            continue;
        }
        let Some(shelf) = section.shelf else {
            continue;
        };
        let directory = shelf
            .heading
            .and_then(|heading| heading.text)
            .and_then(|heading| Some((heading.action?.uri, heading.title)));
        if directory
            .as_ref()
            .is_some_and(|(uri, _)| SPOKEN_WORD_DIRECTORIES.contains(&uri.as_str()))
        {
            continue;
        }
        targets.extend(shelf.items.into_iter().filter_map(|item| {
            (item.entity_uri.starts_with("spotify:page:") && music_browse_target(&item.target_uri))
                .then_some(item.target_uri)
        }));
        if let Some((uri, name)) = directory
            && music_browse_target(&uri)
        {
            targets.push(uri.clone());
            directories.insert(uri, name);
        }
    }
    let mut cards = collection::project(
        session,
        &targets,
        ExtensionKind::SearchGenreMetadata,
        collection::BATCH,
        genre_card,
    )
    .await?;
    let genres: Vec<_> = targets
        .into_iter()
        .filter_map(|uri| {
            let directory = directories.remove(&uri);
            cards.remove(&uri).or_else(|| {
                Some(Genre {
                    id: uri.strip_prefix("spotify:genre:")?.to_owned(),
                    name: directory?,
                    cover: None,
                })
            })
        })
        .collect();
    ensure!(
        !genres.is_empty(),
        "Spotify Browse has no readable music categories"
    );
    Ok(genres)
}

fn music_browse_target(uri: &str) -> bool {
    uri.starts_with("spotify:genre:") && !SPOKEN_WORD_DIRECTORIES.contains(&uri)
}

/// Reads a genre page, then loads its ordered sections in one batch.
pub(super) async fn page(
    session: &Session,
    id: &str,
    title: Option<&str>,
    preview: Option<tokio::sync::mpsc::Sender<GenreDetail>>,
) -> Result<GenreDetail> {
    let BrowsePage {
        mut name,
        sections,
        playlists,
    } = page_sections(session, id, None).await?;
    if name.is_empty()
        && let Some(title) = title
    {
        title.clone_into(&mut name);
    }
    if let Some(preview) = preview {
        let ready: Vec<_> = sections
            .iter()
            .take_while(|(_, uris)| uris.iter().all(|uri| playlists.contains_key(uri)))
            .filter(|(_, uris)| !uris.is_empty())
            .map(|(title, uris)| GenreSection {
                title: title.clone(),
                items: uris
                    .iter()
                    .map(|uri| GenreItem::Playlist(playlists[uri].clone()))
                    .collect(),
            })
            .collect();
        if !ready.is_empty() {
            let _ = preview.try_send(GenreDetail {
                name: name.clone(),
                sections: ready,
            });
        }
    }
    let page_uri = format!("spotify:genre:{id}");
    let mut uris: HashSet<_> = sections
        .iter()
        .flat_map(|(_, items)| items.iter().map(String::as_str))
        .collect();
    if name.is_empty() {
        uris.insert(&page_uri);
    }
    let cards = cards(session, uris, playlists, true).await?;
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
async fn page_sections(
    session: &Session,
    id: &str,
    media_limit: Option<usize>,
) -> Result<BrowsePage> {
    let root = playlists::browse_sections(session, vec![format!("spotify:genre:{id}")], false)
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
    let mut page = BrowsePage {
        name: root.title,
        ..Default::default()
    };
    for mut shelf in playlists::browse_sections(session, section_uris, true).await? {
        page.playlists.extend(std::mem::take(&mut shelf.playlists));
        page.sections.push(section(shelf, media_limit));
    }
    Ok(page)
}

#[derive(Default)]
struct BrowsePage {
    name: String,
    sections: Vec<SectionUris>,
    playlists: HashMap<String, Playlist>,
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

/// Keeps directory links and optionally limits the media on Home shelves.
fn section(section: playlists::BrowseSection, media_limit: Option<usize>) -> (String, Vec<String>) {
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
            media_limit.is_none_or(|limit| media <= limit)
        })
        .collect();
    (section.title, items)
}

fn section_item((uri, target): (String, Option<String>)) -> Option<String> {
    match uri.split(':').nth(1)? {
        "page" => {
            let id = uri.strip_prefix("spotify:page:")?;
            if target
                .as_deref()
                .is_some_and(|uri| SPOKEN_WORD_DIRECTORIES.contains(&uri))
            {
                return None;
            }
            Some(
                target
                    .filter(|target| music_browse_target(target))
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

/// Reuses projected playlist cards, then resolves the rest alongside a mixed catalogue batch.
pub(super) async fn cards(
    session: &Session,
    uris: HashSet<&str>,
    mut playlists: HashMap<String, Playlist>,
    own_display_name: bool,
) -> Result<HashMap<String, GenreItem>> {
    playlists.retain(|uri, _| uris.contains(uri.as_str()));
    let mut catalogue: Vec<(&str, &[ExtensionKind])> = Vec::new();
    for &uri in &uris {
        match uri
            .strip_prefix("spotify:")
            .and_then(|uri| uri.split_once(':'))
        {
            Some(("playlist", _)) if !playlists.contains_key(uri) => {
                catalogue.push((uri, &[ExtensionKind::ListMetadataV2]))
            }
            Some(("album", _)) => catalogue.push((uri, &[ExtensionKind::AlbumV4])),
            Some(("artist", _)) => catalogue.push((uri, &[ExtensionKind::ArtistV4])),
            Some(("genre", _)) => catalogue.push((uri, &[ExtensionKind::SearchGenreMetadata])),
            Some(("track", _)) => catalogue.push((uri, &[ExtensionKind::TrackV4])),
            _ => {}
        }
    }
    // If sign-in could not load the name, try it in the existing catalogue batch.
    let own = format!("spotify:user:{}", session.username());
    if own_display_name
        && session.display_name().is_none()
        && uris.iter().any(|uri| uri.starts_with("spotify:playlist:"))
    {
        catalogue.push((&own, &[ExtensionKind::UserProfile]));
    }
    let mut metadata = collection::extended_batch(session, &catalogue).await?;
    if let Some(data) = collection::take(&mut metadata, ExtensionKind::UserProfile, &own) {
        let profile = super::profiles::contributor(&own, data);
        session.remember_display_name(&profile.name);
    }
    let mut cards: HashMap<_, _> = playlists
        .into_iter()
        .map(|(uri, playlist)| (uri, GenreItem::Playlist(playlist)))
        .collect();
    for ((kind, uri), raw) in metadata {
        let item = match kind {
            ExtensionKind::AlbumV4 => raw
                .decode::<AlbumMessage>()
                .ok()
                .map(|album| GenreItem::Album(albums::album_from(&uri, &album))),
            ExtensionKind::ArtistV4 => raw
                .decode::<ArtistMessage>()
                .ok()
                .and_then(|artist| artists::artist_card(&uri, &artist))
                .map(GenreItem::Artist),
            ExtensionKind::SearchGenreMetadata => raw
                .decode::<SearchGenreMetadata>()
                .ok()
                .and_then(|data| genre_card(&uri, data))
                .map(GenreItem::Genre),
            ExtensionKind::TrackV4 => raw
                .decode::<spotifyapis::proto::extended_metadata::Track>()
                .ok()
                .map(|track| GenreItem::Track(collection::track_from(&uri, &track))),
            ExtensionKind::ListMetadataV2 => uri
                .strip_prefix("spotify:playlist:")
                .and_then(|id| playlists::playlist_card(id, raw.decode::<Attributes>().ok()?))
                .map(GenreItem::Playlist),
            _ => None,
        };
        if let Some(item) = item {
            cards.insert(uri, item);
        }
    }
    if own_display_name {
        let own_name = session.display_name().unwrap_or_else(|| session.username());
        for item in cards.values_mut() {
            let GenreItem::Playlist(playlist) = item else {
                continue;
            };
            // Spotify publishes Liked Songs under its own account.
            let liked_songs = playlist.owner_id == "spotify"
                && playlist
                    .cover
                    .as_deref()
                    .is_some_and(|url| url.starts_with(LIKED_SONGS_ART));
            if playlist.owner_id == session.username() || liked_songs {
                own_name.clone_into(&mut playlist.owner);
            }
        }
    }
    Ok(cards)
}

/// Emits Casita shelves as soon as their cards are ready, then appends Made For You.
pub(super) fn home_paged(session: Session) -> crate::Feed {
    let (send, feed) = tokio::sync::mpsc::channel(2);
    tokio::spawn(async move {
        let result = tokio::select! {
            () = send.closed() => return,
            result = home(&session, |feed| { let _ = send.try_send(Ok(feed.clone())); }) => result,
        };
        if let Some(lot) = result.transpose() {
            let _ = send.send(lot).await;
        }
    });
    feed
}

/// Resolves the main feed while the supplemental page and its sections are fetched.
async fn home(session: &Session, publish: impl FnOnce(&HomeFeed)) -> Result<Option<HomeFeed>> {
    let main = async {
        let (shortcuts, sections) = casita(session).await?;
        let uris = shortcuts
            .iter()
            .chain(sections.iter().flat_map(|(_, uris)| uris))
            .map(String::as_str)
            .collect();
        let cards = cards(session, uris, HashMap::new(), true).await?;
        let feed = HomeFeed {
            listen_again: resolve_cards(shortcuts, &cards),
            sections: shelf_cards(sections, &cards),
            quick_picks: None,
        };
        ensure!(
            !feed.listen_again.is_empty() || !feed.sections.is_empty(),
            "Spotify Home has no readable cards"
        );
        publish(&feed);
        Ok::<_, anyhow::Error>((feed, cards))
    };
    let ((mut feed, mut known), made) = tokio::try_join!(main, async {
        Ok(page_sections(session, MADE_FOR_YOU, Some(ITEMS_PER_SHELF)).await)
    })?;
    let Ok(BrowsePage {
        sections,
        playlists,
        ..
    }) = made
    else {
        return Ok(None);
    };
    let missing = sections
        .iter()
        .flat_map(|(_, uris)| uris)
        .map(String::as_str)
        .filter(|uri| !known.contains_key(*uri))
        .collect();
    known.extend(cards(session, missing, playlists, true).await?);
    let count = feed.sections.len();
    appended(&mut feed, shelf_cards(sections, &known));
    Ok((feed.sections.len() != count).then_some(feed))
}

/// Reads Casita shelf and shortcut references in Spotify's order.
async fn casita(session: &Session) -> Result<(Vec<String>, Vec<SectionUris>)> {
    let zone = jiff::tz::TimeZone::system();
    let response = session
        .api()
        .home(zone.iana_name().unwrap_or("UTC"))
        .await
        .map_err(sign_in_error)?;
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
