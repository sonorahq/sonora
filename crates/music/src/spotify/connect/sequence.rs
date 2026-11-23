use std::collections::HashSet;

use anyhow::{Context as _, Result};
use bytes::Bytes;

use crate::spotify::{auth::sign_in_error, collection, session::Session};
use spotifyapis::proto::{connect::ContextTrack, sequence as proto};

const MAX_PAGES: usize = 40;
const RADIO_TRACKS: usize = 50;

/// Ordered context rows and whether the resolver reached the end of the sequence.
pub(in crate::spotify) struct Resolved {
    pub(in crate::spotify) tracks: Vec<ContextTrack>,
    pub(in crate::spotify) complete: bool,
}

/// Reads Spotify's context and any pages that accompany it, preserving occurrence positions.
pub(in crate::spotify) async fn resolve(session: &Session, uri: &str) -> Result<Resolved> {
    let request = proto::ContextRequest {
        queries: vec![query(uri)],
        client: Some(sequence_client(session)),
    };
    let mut response: proto::ContextResponse = session
        .api()
        .resolve_context(request)
        .await
        .map_err(sign_in_error)?;
    let result = response
        .results
        .remove(&0)
        .context("missing Spotify context result")?;
    anyhow::ensure!(result.error.is_none(), "Spotify could not resolve context");
    let context = result.context.context("missing Spotify context")?;

    let (children, complete) = if let Some(sequence) = context.sequence {
        (sequence.children, true)
    } else if let Some(sequence) = context.paged_sequence {
        let complete = sequence.pages.len() <= MAX_PAGES;
        let page_ids = sequence
            .pages
            .iter()
            .take(MAX_PAGES)
            .filter(|page| page.children.is_empty() && !page.page_id.is_empty())
            .map(|page| page.page_id.clone())
            .collect();
        let mut resolved = resolve_pages(session, uri, page_ids).await?;
        let mut children = Vec::new();
        for page in sequence.pages.into_iter().take(MAX_PAGES) {
            if page.children.is_empty() && !page.page_id.is_empty() {
                let result = resolved.next().context("missing Spotify page result")??;
                children.extend(result.children);
            } else {
                children.extend(page.children);
            }
        }
        (children, complete)
    } else if let Some(sequence) = context.chain_sequence {
        read_chain(session, uri, sequence).await?
    } else {
        anyhow::bail!("Spotify context has no sequence");
    };

    Ok(Resolved {
        tracks: children
            .into_iter()
            .map(|child| ContextTrack {
                uri: Some(child.context_id).filter(|uri| !uri.is_empty()),
                uid: Some(child.uid).filter(|uid| !uid.is_empty()),
                gid: None,
                metadata: child
                    .metadata
                    .map(|metadata| metadata.values)
                    .unwrap_or_default(),
            })
            .collect(),
        complete,
    })
}

/// Resolves and hydrates a track station in Spotify's playback order.
pub(in crate::spotify) async fn track_radio(
    session: &Session,
    id: &str,
) -> Result<Vec<crate::Track>> {
    radio(session, "track", id).await
}

pub(in crate::spotify) fn radio_kind(seed: crate::RadioSeed) -> &'static str {
    match seed {
        crate::RadioSeed::Album => "album",
        crate::RadioSeed::Playlist => "playlist",
        crate::RadioSeed::Artist => "artist",
    }
}

/// Resolves and hydrates a station while preserving Spotify's order.
pub(in crate::spotify) async fn radio(
    session: &Session,
    kind: &str,
    id: &str,
) -> Result<Vec<crate::Track>> {
    let station = format!("spotify:station:{kind}:{id}");
    let uris: Vec<_> = resolve(session, &station)
        .await
        .context("cannot resolve Spotify radio")?
        .tracks
        .into_iter()
        .filter_map(|track| track.uri)
        .filter(|uri| uri.starts_with("spotify:track:"))
        .take(RADIO_TRACKS)
        .collect();
    collection::ordered_tracks(session, &uris, collection::TRACK_DETAILS).await
}

/// Follows the page IDs supplied by Spotify until the chain ends or the page limit is reached.
async fn read_chain(
    session: &Session,
    uri: &str,
    sequence: proto::ChainSequence,
) -> Result<(Vec<proto::Child>, bool)> {
    let mut children = Vec::new();
    let mut next = sequence.next_page_id;
    let mut seen = HashSet::new();
    let mut pages = 0;
    if let Some(page) = sequence.page {
        if !page.page_id.is_empty() {
            seen.insert(page.page_id.clone());
        }
        children = page.children;
        if children.is_empty() && !page.page_id.is_empty() {
            let page = resolve_page(session, uri, page.page_id).await?;
            children = page.children;
            if !page.next_page_id.is_empty() {
                next = page.next_page_id;
            }
        }
        pages = 1;
    }

    for _ in pages..MAX_PAGES {
        if next.is_empty() {
            break;
        }
        anyhow::ensure!(seen.insert(next.clone()), "Spotify repeated a context page");
        let page = resolve_page(session, uri, next).await?;
        children.extend(page.children);
        next = page.next_page_id;
    }
    Ok((children, next.is_empty()))
}

async fn resolve_page(session: &Session, uri: &str, page_id: Bytes) -> Result<proto::PageResult> {
    resolve_pages(session, uri, vec![page_id])
        .await?
        .next()
        .context("missing Spotify page result")?
}

/// Resolves already-known page IDs together and restores the order of the corresponding queries.
async fn resolve_pages(
    session: &Session,
    uri: &str,
    page_ids: Vec<Bytes>,
) -> Result<impl Iterator<Item = Result<proto::PageResult>>> {
    let count = page_ids.len();
    let request = proto::PageRequest {
        queries: page_ids
            .into_iter()
            .map(|page_id| proto::PageQuery {
                page_id,
                context_query: Some(query(uri)),
            })
            .collect(),
        client: Some(sequence_client(session)),
    };
    let mut response: proto::PageResponse = if count == 0 {
        proto::PageResponse::default()
    } else {
        session
            .api()
            .resolve_pages(request)
            .await
            .map_err(sign_in_error)?
    };
    Ok((0..count as u32).map(move |index| {
        let page = response
            .results
            .remove(&index)
            .context("missing Spotify page result")?;
        anyhow::ensure!(
            page.error.is_none(),
            "Spotify could not resolve context page"
        );
        Ok(page)
    }))
}

fn query(uri: &str) -> proto::Query {
    proto::Query {
        context_id: uri.to_owned(),
    }
}

fn sequence_client(session: &Session) -> proto::Client {
    proto::Client {
        device: Some(proto::Device {
            device_id: session.device_id().to_owned(),
        }),
    }
}
