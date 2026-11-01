//! What an artist or an album page fills in behind its own rows, from the views the
//! web player reads its own rails off: every album of an artist with its appears-on view, and the related-albums view of an album with more from the same artist and its
//! neighbours beside it. A view that comes back empty leaves its rail off the page rather
//! than failing it.

use std::collections::HashSet;

use anyhow::{Context as _, Result};

use crate::apple::client::{ALBUM_ARTISTS, AppleClient, Missing};
use crate::apple::wire;
use crate::escape;
use crate::{Album, AlbumCatalogue, ArtistCatalogue, SUGGESTIONS, SavedArtist};

/// How many similar artists lend their releases to a thin rail, and how many releases each
/// lends.
const SIMILAR_ARTISTS: usize = 6;
const SIMILAR_RELEASES: usize = 2;

/// The most albums one request may ask for, which is all Apple allows, and how many such pages
/// a discography may take.
const ALBUM_PAGE: usize = 100;
const ALBUM_PAGES: usize = 10;

/// Every album of one artist, newest first, and its appears-on view, once a library id has been
/// turned into the catalog's. Uploaded albums from the user's library are merged into the
/// discography. The artist page itself holds only the first ten releases of each view and one
/// edition of each album. A discography that cannot be read leaves those on the page.
pub(crate) async fn artist_catalogue(
    client: &AppleClient,
    artist_id: &str,
) -> Result<ArtistCatalogue> {
    let catalog_id = match AppleClient::is_mine(artist_id) {
        true => {
            let Ok(catalog_id) = client.catalog_artist(artist_id).await else {
                return Ok(ArtistCatalogue::default());
            };
            catalog_id
        }
        false => client.catalog_artist(artist_id).await?,
    };
    let path = client.catalog(&format!("/artists/{}", escape::component(&catalog_id)));
    let (answered, albums) = tokio::join!(
        client.get(&path, &[("views", "appears-on-albums"), ALBUM_ARTISTS]),
        every_album(client, &catalog_id),
    );
    let body = answered?;
    let answered: wire::Answer = wire::answer(&body);
    let found = answered
        .data
        .first()
        .context("apple music has no such artist")?;
    let artist_name = found
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_str())
        .unwrap_or_default();
    let albums = match albums {
        Ok(albums) => albums,
        Err(error) => {
            log::warn!("apple: cannot read the discography of {catalog_id}: {error:#}");
            Vec::new()
        }
    };
    let library_albums = client.artist_library_albums(&catalog_id, artist_name).await;
    let albums = wire::merge_albums(albums, library_albums);
    Ok(ArtistCatalogue {
        albums,
        appears_on: wire::view(found, "appears-on-albums")
            .iter()
            .filter_map(wire::album)
            .take(SUGGESTIONS)
            .collect(),
        ..Default::default()
    })
}

/// The related-albums view of one album, without the album itself. A library album that is
/// not in the catalog has no related albums to read.
async fn related_albums(client: &AppleClient, album_id: &str) -> Result<Vec<Album>> {
    if AppleClient::is_mine(album_id) {
        return Ok(Vec::new());
    }
    let body = client
        .get(
            &client.catalog(&format!("/albums/{}", escape::component(album_id))),
            &[("views", "related-albums"), ALBUM_ARTISTS],
        )
        .await
        .context("cannot read the related albums")?;
    let answered: wire::Answer = wire::answer(&body);
    let found = answered
        .data
        .first()
        .context("apple music has no such album")?;
    Ok(wire::view(found, "related-albums")
        .iter()
        .filter_map(wire::album)
        .filter(|album| album.id != album_id)
        .collect())
}

/// The artist's own releases without the album the page is already showing, with the
/// artists Apple lists as similar beside them, read in a single request.
async fn more_from_artist(
    client: &AppleClient,
    album_id: &str,
    artist_id: &str,
) -> Result<(Vec<Album>, Vec<SavedArtist>)> {
    let catalog_id = match client.catalog_artist(artist_id).await {
        Ok(id) => id,
        Err(_) if AppleClient::is_mine(artist_id) => {
            return library_more_from_artist(client, album_id, artist_id).await;
        }
        Err(error) => return Err(error),
    };
    let body = client
        .get(
            &client.catalog(&format!("/artists/{}", escape::component(&catalog_id))),
            &[
                ("views", "full-albums,singles,similar-artists"),
                ALBUM_ARTISTS,
            ],
        )
        .await
        .context("cannot read more from this artist")?;
    let answered: wire::Answer = wire::answer(&body);
    let found = answered
        .data
        .first()
        .context("apple music has no such artist")?;
    let more = wire::view(found, "full-albums")
        .iter()
        .chain(wire::view(found, "singles").iter())
        .filter_map(wire::album)
        .filter(|album| album.id != album_id)
        .collect();
    let similar = wire::view(found, "similar-artists")
        .iter()
        .filter_map(wire::similar_artist)
        .collect();
    Ok((more, similar))
}

/// The library-only releases of an artist that has no catalog equivalent.
async fn library_more_from_artist(
    client: &AppleClient,
    album_id: &str,
    artist_id: &str,
) -> Result<(Vec<Album>, Vec<SavedArtist>)> {
    let body = match client
        .get(
            &format!(
                "/me/library/artists/{}/albums",
                escape::component(artist_id)
            ),
            &[("include", "catalog,artists"), ALBUM_ARTISTS],
        )
        .await
    {
        Ok(body) => body,
        Err(error) if error.is::<Missing>() => return Ok((Vec::new(), Vec::new())),
        Err(error) => {
            return Err(error).context("cannot read library albums from this artist");
        }
    };
    let outline = wire::outline(&body);
    let more = outline
        .read(&wire::library_album)
        .into_iter()
        .filter(|album| album.id != album_id)
        .collect();
    Ok((more, Vec::new()))
}

/// A few releases by one similar artist, skipping the page's own album.
async fn releases_by(client: &AppleClient, album_id: &str, artist_id: &str) -> Result<Vec<Album>> {
    let body = client
        .get(
            &client.catalog(&format!("/artists/{}", escape::component(artist_id))),
            &[("views", "full-albums,singles"), ALBUM_ARTISTS],
        )
        .await
        .with_context(|| format!("cannot read releases by similar artist {artist_id}"))?;
    let answered: wire::Answer = wire::answer(&body);
    let found = answered
        .data
        .first()
        .with_context(|| format!("apple music has no such artist {artist_id}"))?;
    Ok(wire::view(found, "full-albums")
        .iter()
        .chain(wire::view(found, "singles").iter())
        .filter_map(wire::album)
        .filter(|album| album.id != album_id)
        .take(SIMILAR_RELEASES)
        .collect())
}

/// A few releases each from the first similar artists: the cross-artist half of the rail
/// when the related view comes back thin. One artist failing only shortens the rail.
async fn similar_releases(client: &AppleClient, album_id: &str, similar: &[String]) -> Vec<Album> {
    let reads = similar
        .iter()
        .take(SIMILAR_ARTISTS)
        .map(|artist_id| releases_by(client, album_id, artist_id));
    let mut releases = Vec::new();
    for read in futures::future::join_all(reads).await {
        match read {
            Ok(read) => releases.extend(read),
            Err(error) => log::warn!("apple: {error:#}"),
        }
    }
    releases
}

/// The related-albums view of one album with the artist's own releases first, topped up
/// from similar artists while the rail is thin, read together and deduplicated by id.
/// Either half failing only empties its own half; the page keeps the other.
pub(crate) async fn album_catalogue(
    client: &AppleClient,
    album_id: &str,
    artist_id: Option<&str>,
) -> Result<AlbumCatalogue> {
    let related = related_albums(client, album_id);
    let more = async {
        match artist_id {
            Some(artist_id) => more_from_artist(client, album_id, artist_id).await,
            None => Ok((Vec::new(), Vec::new())),
        }
    };
    let (also_like, more) = tokio::join!(related, more);
    // Nothing read at all is an error rather than an empty rail, so the catalog does not keep
    // the empty answer for the rest of the session.
    let (also_like, more) = match (also_like, more) {
        (Err(error), Err(_)) => return Err(error.context("cannot read any recommendations")),
        (Err(error), _) if artist_id.is_none() => return Err(error),
        pair => pair,
    };
    if let Err(error) = &also_like {
        log::warn!("apple: cannot read the related albums: {error:#}");
    }
    let (more_by, similar) = match more {
        Ok(more) => more,
        Err(error) => {
            log::warn!("apple: cannot read more from this artist: {error:#}");
            (Vec::new(), Vec::new())
        }
    };
    let mut seen = HashSet::new();
    let mut liked: Vec<Album> = more_by
        .into_iter()
        .chain(also_like.unwrap_or_default())
        .filter(|album| seen.insert(album.id.clone()))
        .take(SUGGESTIONS)
        .collect();
    let similar_ids: Vec<String> = similar.iter().map(|artist| artist.id.clone()).collect();
    if liked.len() < SUGGESTIONS && !similar_ids.is_empty() {
        for album in similar_releases(client, album_id, &similar_ids).await {
            if liked.len() >= SUGGESTIONS {
                break;
            }
            if seen.insert(album.id.clone()) {
                liked.push(album);
            }
        }
    }
    Ok(AlbumCatalogue {
        also_like: liked,
        similar: similar.into_iter().take(SUGGESTIONS).collect(),
    })
}

/// Every album credited to the artist with every edition of it, read `ALBUM_PAGE` at a time until
/// Apple names no next page. The artist views keep one edition of each album, this list keeps
/// them all. Apple answers 404 for an artist with no albums, which reads as none.
async fn every_album(client: &AppleClient, artist_id: &str) -> Result<Vec<Album>> {
    let path = client.catalog(&format!("/artists/{}/albums", escape::component(artist_id)));
    let limit = ALBUM_PAGE.to_string();
    let mut albums = Vec::new();
    for page in 0..ALBUM_PAGES {
        let offset = (page * ALBUM_PAGE).to_string();
        let answered = client
            .get(
                &path,
                &[("limit", &limit), ("offset", &offset), ALBUM_ARTISTS],
            )
            .await;
        let body = match answered {
            Err(error) if error.is::<Missing>() => break,
            answered => {
                answered.with_context(|| format!("cannot read the albums of artist {artist_id}"))?
            }
        };
        let outline = wire::outline(&body);
        albums.extend(outline.read(&wire::album));
        if !outline.next.is_present() {
            break;
        }
    }
    Ok(albums)
}
