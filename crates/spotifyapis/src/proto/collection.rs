//! Collection v2 paging and mutation messages.

use super::dealer_headers;
use crate::{Session, http};
use anyhow::Result;
use bytes::{Buf as _, Bytes};
use prost::{
    Message as _,
    encoding::{DecodeContext, WireType, decode_key, fixed32, fixed64},
};

pub const UPDATE_TOPIC: &str = "hm://collection/collection";
const PAGING: &str = "/collection/v2/paging";
const WRITE: &str = "/collection/v2/write";
const CONTENT: &str = "application/vnd.collection-v2.spotify.proto";

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

/// Collection paging has used multiple wire types for the saved timestamp.
#[derive(prost::Message)]
struct EncodedPage {
    #[prost(bytes = "bytes", repeated, tag = "1")]
    items: Vec<Bytes>,
    #[prost(string, tag = "2")]
    next: String,
}

fn decode_page(bytes: Bytes) -> Result<PageResponse> {
    let EncodedPage { items, next } = EncodedPage::decode(bytes)?;
    let items = items
        .into_iter()
        .map(|mut bytes| {
            let mut item = CollectionItem::default();
            while bytes.has_remaining() {
                let (tag, wire_type) = decode_key(&mut bytes)?;
                let ctx = DecodeContext::default();
                match (tag, wire_type) {
                    (2, WireType::ThirtyTwoBit) => {
                        let mut date = 0;
                        fixed32::merge(wire_type, &mut date, &mut bytes, ctx)?;
                        item.added_at = i32::try_from(date).ok();
                    }
                    (2, WireType::SixtyFourBit) => {
                        let mut date = 0;
                        fixed64::merge(wire_type, &mut date, &mut bytes, ctx)?;
                        item.added_at = i32::try_from(date).ok();
                    }
                    _ => item.merge_field(tag, wire_type, &mut bytes, ctx)?,
                }
            }
            Ok::<_, prost::DecodeError>(item)
        })
        .collect::<std::result::Result<_, _>>()?;
    Ok(PageResponse { items, next })
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

impl Session {
    /// Reads a page of saved collection items.
    pub async fn collection_page(&self, request: &PageRequest) -> Result<PageResponse> {
        let bytes = http::read_proto(self, PAGING, CONTENT, request).await?;
        decode_page(bytes)
    }

    /// Applies updates to the saved collection or pins.
    pub async fn update_collection(
        &self,
        request: &WriteRequest,
        connection_id: &str,
    ) -> Result<()> {
        http::post_proto(
            self,
            WRITE,
            CONTENT,
            dealer_headers(connection_id)?,
            request,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_page_reads_saved_dates_removals_and_cursor() {
        for bytes in [
            &b"\x0a\x06\x0a\x01a\x10\xac\x02\x0a\x05\x0a\x01b\x18\x01\x12\x04next"[..],
            &b"\x0a\x08\x0a\x01a\x15\x2c\x01\x00\x00\x0a\x05\x0a\x01b\x18\x01\x12\x04next"[..],
            &b"\x0a\x0c\x0a\x01a\x11\x2c\x01\x00\x00\x00\x00\x00\x00\x0a\x05\x0a\x01b\x18\x01\x12\x04next"[..],
        ] {
            let page = decode_page(Bytes::from_static(bytes)).unwrap();
            assert_eq!(page.next, "next");
            assert_eq!(
                page.items.iter()
                    .map(|item| (item.uri.as_str(), item.added_at, item.is_removed))
                    .collect::<Vec<_>>(),
                [("a", Some(300), false), ("b", None, true)]
            );
        }
        let overflow = b"\x0a\x0c\x0a\x01a\x11\x2c\x01\x00\x00\x01\x00\x00\x00";
        let page = decode_page(Bytes::from_static(overflow)).unwrap();
        assert_eq!(page.items[0].added_at, None);
    }
}
