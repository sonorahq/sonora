//! Collection v2 paging and mutation messages.

use bytes::Bytes;

pub const UPDATE_TOPIC: &str = "hm://collection/collection";
pub(crate) const PAGING: &str = "/collection/v2/paging";
pub(crate) const WRITE: &str = "/collection/v2/write";
pub(crate) const CONTENT: &str = "application/vnd.collection-v2.spotify.proto";

#[derive(prost::Message)]
pub struct CollectionItem {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(int32, optional, tag = "2")]
    pub added_at: Option<i32>,
    #[prost(bool, tag = "3")]
    pub is_removed: bool,
    // #[prost(string, optional, tag = "4")]
    // pub context_uri: Option<String>,
}

#[derive(prost::Message)]
pub struct PageRequest {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(string, tag = "2")]
    pub set: String,
    #[prost(string, tag = "3")]
    pub token: String,
    #[prost(int32, tag = "4")]
    pub limit: i32,
}

#[derive(prost::Message)]
pub struct PageResponse {
    #[prost(message, repeated, tag = "1")]
    pub items: Vec<CollectionItem>,
    #[prost(string, tag = "2")]
    pub next: String,
    // #[prost(string, tag = "3")]
    // pub sync_token: String,
}

#[derive(prost::Message)]
pub struct WriteRequest {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(string, tag = "2")]
    pub set: String,
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<CollectionItem>,
    #[prost(string, tag = "4")]
    pub update_id: String,
}

/// Saved collection changes pushed by Dealer.
#[derive(prost::Message)]
pub struct CollectionUpdate {
    #[prost(message, repeated, tag = "1")]
    pub items: Vec<ChangedItem>,
}

/// One added or removed item, keyed by its Spotify binary ID.
#[derive(prost::Message)]
pub struct ChangedItem {
    #[prost(uint64, tag = "1")]
    pub kind: u64,
    #[prost(bytes = "bytes", optional, tag = "2")]
    pub gid: Option<Bytes>,
    #[prost(bool, tag = "6")]
    pub removed: bool,
}
