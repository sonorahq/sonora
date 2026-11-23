//! SequenceProxy context and page messages.

use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use bytes::Bytes;

use crate::{Session, http};

use super::connect_state::{ContextPage, ContextTrack};

const MAX_PAGES: usize = 40;
const RESOLVE_CONTEXT: &str =
    "/sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext";
const RESOLVE_PAGE: &str =
    "/sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage";

/// A batch of context queries and the device making them.
#[derive(prost::Message)]
struct ContextRequest {
    #[prost(message, repeated, tag = "1")]
    pub queries: Vec<Query>,
    #[prost(message, optional, tag = "2")]
    pub client: Option<Client>,
}

/// Identifies a context by its Spotify URI.
#[derive(prost::Message)]
struct Query {
    #[prost(string, tag = "1")]
    pub context_id: String,
}

/// Client identity sent with a context request.
#[derive(prost::Message)]
struct Client {
    #[prost(message, optional, tag = "1")]
    pub device: Option<Device>,
}

/// Identifies the Spotify Connect device resolving the context.
#[derive(prost::Message)]
struct Device {
    #[prost(string, tag = "2")]
    pub device_id: String,
}

/// Context results keyed by the zero-based query index.
#[derive(prost::Message)]
struct ContextResponse {
    #[prost(map = "uint32, message", tag = "1")]
    pub results: HashMap<u32, ContextResult>,
}

/// A resolved context or a per-query service error.
#[derive(PartialEq, prost::Message)]
struct ContextResult {
    /// Encoded service error. The nested schema is unverified.
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub error: Option<Bytes>,
    #[prost(message, optional, tag = "2")]
    pub context: Option<Context>,
}

/// The inline or paginated sequence for a context.
#[derive(PartialEq, prost::Message)]
struct Context {
    // #[prost(bytes, optional, tag = "1")]
    // pub attributes: Option<Vec<u8>>,
    #[prost(message, optional, tag = "2")]
    pub metadata: Option<Metadata>,
    // #[prost(bytes, optional, tag = "3")]
    // pub provenance: Option<Vec<u8>>,
    #[prost(message, optional, tag = "7")]
    pub sequence: Option<Sequence>,
    #[prost(message, optional, tag = "8")]
    pub paged_sequence: Option<PagedSequence>,
    #[prost(message, optional, tag = "9")]
    pub chain_sequence: Option<ChainSequence>,
}

/// An ordered list of occurrences in the context.
#[derive(PartialEq, prost::Message)]
struct Sequence {
    #[prost(message, repeated, tag = "1")]
    pub children: Vec<Child>,
}

/// A child context and the occurrence ID Spotify assigns it in this sequence.
#[derive(PartialEq, prost::Message)]
struct Child {
    #[prost(string, tag = "1")]
    pub context_id: String,
    #[prost(string, tag = "2")]
    pub uid: String,
    #[prost(message, optional, tag = "9")]
    pub metadata: Option<Metadata>,
}

/// String metadata for a resolved context or a child occurrence.
#[derive(PartialEq, prost::Message)]
struct Metadata {
    #[prost(map = "string, string", tag = "1")]
    pub values: HashMap<String, String>,
}

/// A sequence with its pages listed in playback order.
#[derive(PartialEq, prost::Message)]
struct PagedSequence {
    #[prost(message, repeated, tag = "1")]
    pub pages: Vec<Page>,
}

/// The first page and continuation of a linked sequence.
#[derive(PartialEq, prost::Message)]
struct ChainSequence {
    #[prost(message, optional, tag = "1")]
    pub page: Option<Page>,
    #[prost(bytes = "bytes", tag = "3")]
    pub next_page_id: Bytes,
}

/// An ordered page whose ID can be resolved when its children are omitted.
#[derive(PartialEq, prost::Message)]
struct Page {
    #[prost(bytes = "bytes", tag = "1")]
    pub page_id: Bytes,
    #[prost(message, repeated, tag = "2")]
    pub children: Vec<Child>,
}

/// A batch of page queries and the device making them.
#[derive(prost::Message)]
struct PageRequest {
    #[prost(message, repeated, tag = "1")]
    pub queries: Vec<PageQuery>,
    #[prost(message, optional, tag = "2")]
    pub client: Option<Client>,
}

/// An opaque page ID paired with its context.
#[derive(prost::Message)]
struct PageQuery {
    #[prost(bytes = "bytes", tag = "1")]
    pub page_id: Bytes,
    #[prost(message, optional, tag = "2")]
    pub context_query: Option<Query>,
}

/// Page results keyed by the zero-based query index.
#[derive(prost::Message)]
struct PageResponse {
    #[prost(map = "uint32, message", tag = "1")]
    pub results: HashMap<u32, PageResult>,
}

/// The page's children and the next ID for a linked sequence.
#[derive(PartialEq, prost::Message)]
struct PageResult {
    /// Encoded service error. The nested schema is unverified.
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub error: Option<Bytes>,
    #[prost(message, repeated, tag = "2")]
    pub children: Vec<Child>,
    #[prost(bytes = "bytes", tag = "4")]
    pub next_page_id: Bytes,
}

/// Ordered context rows and whether Spotify reached the end of the sequence.
pub struct ResolvedSequence {
    pub tracks: Vec<ContextTrack>,
    pub complete: bool,
}

impl ResolvedSequence {
    /// Flattens pages embedded in Connect messages and detects an unresolved continuation.
    pub fn from_context_pages(pages: Vec<ContextPage>) -> Self {
        let complete = pages.last().is_none_or(|page| {
            page.page_url
                .as_deref()
                .filter(|_| page.tracks.is_empty())
                .or(page.next_page_url.as_deref())
                .is_none_or(str::is_empty)
        });
        Self {
            complete,
            tracks: pages.into_iter().flat_map(|page| page.tracks).collect(),
        }
    }
}

impl Session {
    /// Resolves a playback context and follows its pages up to the safety limit.
    pub async fn resolve_sequence(&self, uri: &str, device_id: &str) -> Result<ResolvedSequence> {
        let request = ContextRequest {
            queries: vec![query(uri)],
            client: Some(sequence_client(device_id)),
        };
        let mut response: ContextResponse = http::read_rpc(self, RESOLVE_CONTEXT, &request).await?;
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
            let mut resolved = resolve_pages(self, uri, device_id, page_ids).await?;
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
            read_chain(self, uri, device_id, sequence).await?
        } else {
            anyhow::bail!("Spotify context has no sequence");
        };

        Ok(ResolvedSequence {
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
}

/// Follows the page IDs supplied by Spotify until the chain ends or the page limit is reached.
async fn read_chain(
    session: &Session,
    uri: &str,
    device_id: &str,
    sequence: ChainSequence,
) -> Result<(Vec<Child>, bool)> {
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
            let page = resolve_page(session, uri, device_id, page.page_id).await?;
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
        let page = resolve_page(session, uri, device_id, next).await?;
        children.extend(page.children);
        next = page.next_page_id;
    }
    Ok((children, next.is_empty()))
}

async fn resolve_page(
    session: &Session,
    uri: &str,
    device_id: &str,
    page_id: Bytes,
) -> Result<PageResult> {
    resolve_pages(session, uri, device_id, vec![page_id])
        .await?
        .next()
        .context("missing Spotify page result")?
}

/// Resolves already-known page IDs together and restores the order of the corresponding queries.
async fn resolve_pages(
    session: &Session,
    uri: &str,
    device_id: &str,
    page_ids: Vec<Bytes>,
) -> Result<impl Iterator<Item = Result<PageResult>>> {
    let count = page_ids.len();
    let request = PageRequest {
        queries: page_ids
            .into_iter()
            .map(|page_id| PageQuery {
                page_id,
                context_query: Some(query(uri)),
            })
            .collect(),
        client: Some(sequence_client(device_id)),
    };
    let mut response: PageResponse = if count == 0 {
        PageResponse::default()
    } else {
        http::read_rpc(session, RESOLVE_PAGE, &request).await?
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

fn query(uri: &str) -> Query {
    Query {
        context_id: uri.to_owned(),
    }
}

fn sequence_client(device_id: &str) -> Client {
    Client {
        device: Some(Device {
            device_id: device_id.to_owned(),
        }),
    }
}
