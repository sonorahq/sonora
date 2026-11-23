use std::collections::{HashMap, HashSet};

use crate::spotify::session::Session;
use anyhow::{Context as _, Result};
use futures::{StreamExt as _, stream};
use spotifyapis::proto::metadata::{
    AlbumGroup, Artist as ArtistMessage, Image, RelatedCreators, RelatedEntities, Reputation,
    VisualIdentity,
};

use crate::spotify::{albums, collection, collection::take, wire};
use crate::{
    Album, AlbumCatalogue, Artist, ArtistCatalogue, ArtistProfile, ReleaseType, SUGGESTIONS,
    SavedArtist, Track,
};
use spotifyapis::proto::metadata::ExtensionKind;

const ARTIST_PREFIX: &str = "spotify:artist:";
const ALBUM_PREFIX: &str = "spotify:album:";
const UNKNOWN: &str = "Unknown";
const OVERVIEW_RELEASES: usize = 24;
const ARTIST_OVERVIEW: &[ExtensionKind] = &[
    ExtensionKind::ArtistV4,
    ExtensionKind::VisualIdentity,
    ExtensionKind::OnPlatformReputation,
];

/// Reads the profile and official ranking before hydrating tracks and releases together.
pub async fn artist(session: &Session, artist_id: &str) -> Result<Artist> {
    let uri = format!("{ARTIST_PREFIX}{artist_id}");
    let requests = [(uri.as_str(), ARTIST_OVERVIEW)];
    let mut data = collection::extended_batch(session, &requests).await?;
    let artist: ArtistMessage = take(&mut data, ExtensionKind::ArtistV4, &uri)
        .context("Spotify omitted the artist metadata")?;
    let audience: Option<Reputation> = take(&mut data, ExtensionKind::OnPlatformReputation, &uri);
    let banner = take::<VisualIdentity>(&mut data, ExtensionKind::VisualIdentity, &uri)
        .and_then(|visual| visual.wide_full_bleed_image)
        .and_then(|image| {
            image
                .instances
                .into_iter()
                .filter_map(|instance| Some((instance.size, instance.flat_file?.cdn_url)))
                .filter(|(_, url)| wire::fetchable(url))
                .max_by_key(|(size, _)| *size)
                .map(|(_, url)| url)
        });
    let profile = profile_from(&artist);
    let releases = release_uris(&artist)
        .into_iter()
        .take(OVERVIEW_RELEASES)
        .collect::<Vec<_>>();
    let ranked = ranked_tracks(&artist);
    let (top_tracks, albums) = overview_cards(session, &ranked, &releases).await?;
    Ok(Artist {
        name: profile.name,
        cover_large: profile.cover_large,
        banner,
        biography: profile.biography,
        monthly_listeners: audience
            .as_ref()
            .and_then(|stats| audience_count(stats.monthly_listeners)),
        followers: audience
            .and_then(|stats| audience_count(stats.total_established_user_relationships)),
        top_tracks,
        albums,
    })
}

/// Loads the remaining releases after the artist overview appears.
pub async fn catalogue(
    session: &Session,
    artist_id: &str,
    top_tracks: Vec<Track>,
    mut preview: Option<tokio::sync::mpsc::Sender<ArtistCatalogue>>,
) -> Result<ArtistCatalogue> {
    let artist = metadata(session, artist_id).await?;
    let releases = release_uris(&artist);
    let own: HashSet<_> = releases.iter().map(String::as_str).collect();
    let appearances = group_uris(&artist.appears_on_group)
        .filter(|uri| !own.contains(uri.as_str()))
        .collect::<Vec<_>>();
    let uris = releases
        .iter()
        .chain(&appearances)
        .cloned()
        .collect::<Vec<_>>();
    let requests: Vec<_> = uris
        .chunks(albums::ALBUM_BATCH)
        .map(|batch| albums::metadata(session, batch))
        .collect();
    let batches = stream::iter(requests).buffered(2);
    tokio::pin!(batches);
    let mut known = HashMap::new();
    while let Some(batch) = batches.next().await {
        known.extend(batch?);
        if uris.len() > albums::ALBUM_BATCH
            && let Some(preview) = preview.take()
        {
            let _ = preview.try_send(ArtistCatalogue {
                albums: catalogue_albums(&releases, &known),
                appears_on: catalogue_albums(&appearances, &known),
                top_tracks: Vec::new(),
            });
        }
    }
    Ok(ArtistCatalogue {
        albums: catalogue_albums(&releases, &known),
        appears_on: catalogue_albums(&appearances, &known),
        top_tracks,
    })
}

/// Keeps Spotify's ranked tracks first, then adds the artist's playable discography.
pub async fn tracks(session: &Session, artist_id: &str) -> Result<Vec<Track>> {
    let artist = metadata(session, artist_id).await?;
    let mut ranked = ranked_tracks(&artist);
    let uris = release_uris(&artist);
    let mut known = albums::project(session, &uris, |uri, album| {
        (
            albums::album_from(uri, album),
            albums::track_uris(album).collect::<Vec<_>>(),
        )
    })
    .await?;
    let mut releases = newest_first(
        uris.iter().filter_map(|uri| known.remove(uri)),
        |(album, _)| album,
    );
    releases.sort_by_key(|(album, _)| match album.release_type {
        ReleaseType::Album | ReleaseType::Ep => 0,
        ReleaseType::Single => 1,
        _ => 2,
    });

    let mut seen: HashSet<_> = ranked.iter().cloned().collect();
    let ranked_count = ranked.len();
    ranked.extend(
        releases
            .into_iter()
            .flat_map(|(_, tracks)| tracks)
            .filter(|uri| seen.insert(uri.clone())),
    );
    let mut known = collection::metadata(session, &ranked).await?;
    let (ranked_uris, other_uris) = ranked.split_at(ranked_count);
    let mut ranked: Vec<_> = ranked_uris
        .iter()
        .filter_map(|uri| known.remove(uri))
        .collect();
    let mut other: Vec<_> = other_uris
        .iter()
        .filter_map(|uri| known.remove(uri))
        .filter(|track| track.playable && track.id.is_some())
        .collect();
    other.sort_by_key(|track| std::cmp::Reverse(track.popularity));
    ranked.extend(other);
    Ok(crate::artist_songs(ranked, artist_id))
}

pub async fn profile(session: &Session, artist_id: &str) -> Result<ArtistProfile> {
    Ok(profile_from(&metadata(session, artist_id).await?))
}

/// Uses the ArtistV4 ranking when its usable groups agree.
fn ranked_tracks(artist: &ArtistMessage) -> Vec<String> {
    let mut rankings = artist.top_tracks.iter().filter_map(|group| {
        let mut seen = HashSet::new();
        let tracks: Vec<_> = group
            .tracks
            .iter()
            .filter_map(|track| spotifyapis::id::base62(track.gid()))
            .map(|id| format!("spotify:track:{id}"))
            .filter(|uri| seen.insert(uri.clone()))
            .collect();
        (!tracks.is_empty()).then_some(tracks)
    });
    if let Some(first) = rankings.next()
        && rankings.all(|tracks| tracks == first)
    {
        return first;
    }

    Vec::new()
}

/// Hydrates tracks, play counts, and album cards in one request per batch.
async fn overview_cards(
    session: &Session,
    ranked: &[String],
    releases: &[String],
) -> Result<(Vec<Track>, Vec<Album>)> {
    let requests = ranked
        .iter()
        .map(|uri| (uri.as_str(), collection::TRACK_DETAILS))
        .chain(
            releases
                .iter()
                .map(|uri| (uri.as_str(), &[ExtensionKind::AlbumV4][..])),
        )
        .collect::<Vec<_>>();
    let mut data = collection::extended_batch(session, &requests).await?;
    let ranked = ranked
        .iter()
        .filter_map(|uri| collection::take_track(&mut data, uri))
        .collect();
    let albums = newest_first(
        releases.iter().filter_map(|uri| {
            let album = take(&mut data, ExtensionKind::AlbumV4, uri)?;
            Some(albums::album_from(uri, &album))
        }),
        |album| album,
    );
    Ok((ranked, albums))
}

fn audience_count(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

/// Loads Spotify’s related releases and creators, then hydrates both in one batch.
pub(super) async fn album_catalogue(
    session: &Session,
    album_id: &str,
    artist_id: Option<&str>,
) -> Result<AlbumCatalogue> {
    let album_uri = format!("{ALBUM_PREFIX}{album_id}");
    let artist_uri = artist_id.map(|id| format!("{ARTIST_PREFIX}{id}"));
    let mut requests = vec![(album_uri.as_str(), &[ExtensionKind::RelatedReleases][..])];
    if let Some(uri) = artist_uri.as_deref() {
        requests.push((uri, &[ExtensionKind::RelatedCreatorsSection]));
    }
    let mut data = collection::extended_batch(session, &requests).await?;
    let mut seen = HashSet::from([album_uri.clone()]);
    let related: Option<RelatedEntities> =
        take(&mut data, ExtensionKind::RelatedReleases, &album_uri);
    let related: Vec<_> = related
        .into_iter()
        .flat_map(|related| related.entities)
        .map(|entry| entry.uri)
        .filter(|uri| uri.starts_with(ALBUM_PREFIX) && seen.insert(uri.clone()))
        .take(SUGGESTIONS)
        .collect();
    let mut seen: HashSet<_> = artist_id.into_iter().collect();
    let creators = artist_uri.as_deref().and_then(|uri| {
        take::<RelatedCreators>(&mut data, ExtensionKind::RelatedCreatorsSection, uri)
    });
    let artist_uris = creators
        .iter()
        .flat_map(|creators| &creators.sections)
        .flat_map(|section| &section.entities)
        .map(|entry| entry.uri.as_str())
        .filter(|uri| {
            uri.strip_prefix(ARTIST_PREFIX)
                .is_some_and(|id| seen.insert(id))
        })
        .take(SUGGESTIONS)
        .collect::<Vec<_>>();
    let requests: Vec<_> = artist_uris
        .iter()
        .map(|&uri| (uri, &[ExtensionKind::ArtistV4][..]))
        .chain(
            related
                .iter()
                .map(|uri| (uri.as_str(), &[ExtensionKind::AlbumV4][..])),
        )
        .collect();
    let mut data = collection::extended_batch(session, &requests).await?;
    let also_like = related
        .iter()
        .filter_map(|uri| {
            let album = take(&mut data, ExtensionKind::AlbumV4, uri)?;
            Some(albums::album_from(uri, &album))
        })
        .collect();
    let similar = artist_uris
        .into_iter()
        .filter_map(|uri| {
            let artist = take(&mut data, ExtensionKind::ArtistV4, uri)?;
            artist_card(uri, &artist)
        })
        .collect();
    Ok(AlbumCatalogue { also_like, similar })
}

fn catalogue_albums(uris: &[String], known: &HashMap<String, Album>) -> Vec<Album> {
    newest_first(
        uris.iter().filter_map(|uri| known.get(uri).cloned()),
        |album| album,
    )
}

/// Keeps one edition per title and release type, in newest-first order.
fn newest_first<T>(releases: impl Iterator<Item = T>, model: impl Fn(&T) -> &Album) -> Vec<T> {
    let mut seen = HashSet::new();
    let mut releases: Vec<_> = releases
        .filter(|entry| {
            let album = model(entry);
            seen.insert((album.name.to_lowercase(), album.release_type))
        })
        .collect();

    releases.sort_by(|left, right| model(right).release_date.cmp(&model(left).release_date));
    releases
}

async fn metadata(session: &Session, artist_id: &str) -> Result<ArtistMessage> {
    let uri = format!("{ARTIST_PREFIX}{artist_id}");
    collection::project(
        session,
        std::slice::from_ref(&uri),
        ExtensionKind::ArtistV4,
        1,
        |_, artist| Some(artist),
    )
    .await?
    .remove(&uri)
    .context("Spotify omitted the artist metadata")
}

pub async fn saved_artists(session: &Session) -> Result<Vec<SavedArtist>> {
    let items = collection::saved_items(session, collection::ARTISTS, ARTIST_PREFIX).await?;
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.uri.strip_prefix(ARTIST_PREFIX).map(str::to_owned))
        .collect();
    let mut known = cards(session, &ids).await?;

    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut artist = known.remove(item.uri.strip_prefix(ARTIST_PREFIX)?)?;
            artist.added_at = item.added_at.map(i64::from);
            Some(artist)
        })
        .collect())
}

pub async fn images(session: &Session, ids: &[String]) -> Result<HashMap<String, String>> {
    Ok(cards(session, ids)
        .await?
        .into_iter()
        .filter_map(|(id, artist)| artist.cover.map(|url| (id, url)))
        .collect())
}

pub(super) async fn cards(
    session: &Session,
    ids: &[String],
) -> Result<HashMap<String, SavedArtist>> {
    let uris: Vec<_> = ids
        .iter()
        .map(|id| format!("{ARTIST_PREFIX}{id}"))
        .collect();
    let cards = collection::project(
        session,
        &uris,
        ExtensionKind::ArtistV4,
        collection::BATCH,
        |uri, artist| artist_card(uri, &artist),
    )
    .await
    .context("cannot read artist portraits")?;
    Ok(cards
        .into_values()
        .map(|card| (card.id.clone(), card))
        .collect())
}

pub(super) fn artist_card(uri: &str, message: &ArtistMessage) -> Option<SavedArtist> {
    let id = uri.strip_prefix(ARTIST_PREFIX)?;
    let cover = portraits(message)
        .into_iter()
        .min_by_key(|image| wire::image_width(image))
        .and_then(|image| wire::image_url(image.file_id()));
    let name = match message.name() {
        "" => UNKNOWN,
        name => name,
    };
    Some(SavedArtist {
        id: id.to_owned(),
        name: name.to_owned(),
        cover,
        added_at: None,
    })
}

fn profile_from(artist: &ArtistMessage) -> ArtistProfile {
    let portraits = portraits(artist);

    ArtistProfile {
        name: artist.name().to_owned(),
        cover_large: portraits
            .iter()
            .max_by_key(|image| wire::image_width(image))
            .and_then(|image| wire::image_url(image.file_id())),
        biography: artist.biography.iter().find_map(|bio| {
            bio.text
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(plain_text)
                .filter(|text| !text.is_empty())
        }),
    }
}

fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut tag = String::new();
    let mut inside_tag = false;

    for character in html.chars() {
        match character {
            '<' if !inside_tag => {
                inside_tag = true;
                tag.clear();
            }
            '>' if inside_tag => {
                inside_tag = false;
                let tag = tag.trim().to_ascii_lowercase();
                if (tag.starts_with("br")
                    || tag.starts_with("/p")
                    || tag.starts_with("/li")
                    || tag.starts_with("/div"))
                    && !text.ends_with(char::is_whitespace)
                {
                    text.push(' ');
                }
            }
            _ if inside_tag => tag.push(character),
            _ => text.push(character),
        }
    }

    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn release_uris(artist: &ArtistMessage) -> Vec<String> {
    group_uris(
        artist
            .album_group
            .iter()
            .chain(&artist.single_group)
            .chain(&artist.compilation_group),
    )
    .collect()
}

/// Lists each release ID once, retaining the service's edition order.
fn group_uris<'a>(
    groups: impl IntoIterator<Item = &'a AlbumGroup> + 'a,
) -> impl Iterator<Item = String> + 'a {
    let mut seen = HashSet::new();
    groups
        .into_iter()
        .flat_map(|group| group.album.iter())
        .filter_map(|album| spotifyapis::id::base62(album.gid()))
        .filter(move |id| seen.insert(id.clone()))
        .map(|id| format!("{ALBUM_PREFIX}{id}"))
}

fn portraits(artist: &ArtistMessage) -> Vec<&Image> {
    let mut portraits: Vec<_> = artist
        .portrait_group
        .as_ref()
        .into_iter()
        .flat_map(|group| group.image.iter())
        .filter(|image| image.file_id.is_some())
        .collect();
    portraits.extend(
        artist
            .portrait
            .iter()
            .filter(|image| image.file_id.is_some()),
    );
    portraits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editions_keep_their_type_and_sort_by_date() {
        let releases = [
            ("Record", ReleaseType::Album, "2020"),
            ("RECORD", ReleaseType::Album, "2019"),
            ("Record", ReleaseType::Single, "2021"),
            ("Record Deluxe", ReleaseType::Album, "2022"),
        ]
        .map(|(name, release_type, release_date)| Album {
            name: name.into(),
            release_type,
            release_date: release_date.into(),
            ..Album::default()
        });
        assert_eq!(
            newest_first(releases.into_iter(), |album| album)
                .into_iter()
                .map(|album| album.release_date)
                .collect::<Vec<_>>(),
            ["2022", "2021", "2020"]
        );
        assert_eq!(plain_text("A &amp; B<br>Live"), "A & B Live");
    }
}
