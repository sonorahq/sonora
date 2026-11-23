//! SequenceProxy context and page messages.

use std::collections::HashMap;

use bytes::Bytes;

pub(crate) const RESOLVE_CONTEXT: &str =
    "/sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext";
pub(crate) const RESOLVE_PAGE: &str =
    "/sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage";

/// A batch of context queries and the device making them.
#[derive(prost::Message)]
pub struct ContextRequest {
    #[prost(message, repeated, tag = "1")]
    pub queries: Vec<Query>,
    #[prost(message, optional, tag = "2")]
    pub client: Option<Client>,
}

/// Identifies a context by its Spotify URI.
#[derive(prost::Message)]
pub struct Query {
    #[prost(string, tag = "1")]
    pub context_id: String,
}

/// Client identity sent with a context request.
#[derive(prost::Message)]
pub struct Client {
    #[prost(message, optional, tag = "1")]
    pub device: Option<Device>,
}

/// Identifies the Spotify Connect device resolving the context.
#[derive(prost::Message)]
pub struct Device {
    #[prost(string, tag = "2")]
    pub device_id: String,
}

/// Context results keyed by the zero-based query index.
#[derive(prost::Message)]
pub struct ContextResponse {
    #[prost(map = "uint32, message", tag = "1")]
    pub results: HashMap<u32, ContextResult>,
}

/// A resolved context or a per-query service error.
#[derive(PartialEq, prost::Message)]
pub struct ContextResult {
    /// Encoded service error. The nested schema is unverified.
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub error: Option<Bytes>,
    #[prost(message, optional, tag = "2")]
    pub context: Option<Context>,
}

/// The inline or paginated sequence for a context.
#[derive(PartialEq, prost::Message)]
pub struct Context {
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
pub struct Sequence {
    #[prost(message, repeated, tag = "1")]
    pub children: Vec<Child>,
}

/// A child context and the occurrence ID Spotify assigns it in this sequence.
#[derive(PartialEq, prost::Message)]
pub struct Child {
    #[prost(string, tag = "1")]
    pub context_id: String,
    #[prost(string, tag = "2")]
    pub uid: String,
    #[prost(message, optional, tag = "9")]
    pub metadata: Option<Metadata>,
}

/// String metadata for a resolved context or a child occurrence.
#[derive(PartialEq, prost::Message)]
pub struct Metadata {
    #[prost(map = "string, string", tag = "1")]
    pub values: HashMap<String, String>,
}

/// A sequence with its pages listed in playback order.
#[derive(PartialEq, prost::Message)]
pub struct PagedSequence {
    #[prost(message, repeated, tag = "1")]
    pub pages: Vec<Page>,
}

/// The first page and continuation of a linked sequence.
#[derive(PartialEq, prost::Message)]
pub struct ChainSequence {
    #[prost(message, optional, tag = "1")]
    pub page: Option<Page>,
    #[prost(bytes = "bytes", tag = "3")]
    pub next_page_id: Bytes,
}

/// An ordered page whose ID can be resolved when its children are omitted.
#[derive(PartialEq, prost::Message)]
pub struct Page {
    #[prost(bytes = "bytes", tag = "1")]
    pub page_id: Bytes,
    #[prost(message, repeated, tag = "2")]
    pub children: Vec<Child>,
}

/// A batch of page queries and the device making them.
#[derive(prost::Message)]
pub struct PageRequest {
    #[prost(message, repeated, tag = "1")]
    pub queries: Vec<PageQuery>,
    #[prost(message, optional, tag = "2")]
    pub client: Option<Client>,
}

/// An opaque page ID paired with its context.
#[derive(prost::Message)]
pub struct PageQuery {
    #[prost(bytes = "bytes", tag = "1")]
    pub page_id: Bytes,
    #[prost(message, optional, tag = "2")]
    pub context_query: Option<Query>,
}

/// Page results keyed by the zero-based query index.
#[derive(prost::Message)]
pub struct PageResponse {
    #[prost(map = "uint32, message", tag = "1")]
    pub results: HashMap<u32, PageResult>,
}

/// The page's children and the next ID for a linked sequence.
#[derive(PartialEq, prost::Message)]
pub struct PageResult {
    /// Encoded service error. The nested schema is unverified.
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub error: Option<Bytes>,
    #[prost(message, repeated, tag = "2")]
    pub children: Vec<Child>,
    #[prost(bytes = "bytes", tag = "4")]
    pub next_page_id: Bytes,
}
