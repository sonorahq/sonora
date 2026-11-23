//! Spotify service requests and their protobuf schemas.
//!
//! Commented-out fields inventory the wire format without adding fields or
//! allocations to decoded Rust messages.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Deserializer};

pub(super) mod access_point;
pub mod browse;
pub mod collection;
pub mod connect;
pub mod home;
pub mod jam;
pub mod list_read;
pub mod lyrics;
pub mod metadata;
pub mod playlist;
pub mod profile;
pub mod search;
pub mod sequence;
mod storage;

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

// Spotify's ProtoJSON can use null for an absent scalar, map, or list.
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    input: D,
) -> Result<T, D::Error> {
    Ok(Option::deserialize(input)?.unwrap_or_default())
}

fn component(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

fn dealer_headers(connection_id: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if !connection_id.is_empty() {
        headers.insert("x-spotify-connection-id", connection_id.parse()?);
    }
    Ok(headers)
}

// Search and Lyrics reject requests without a platform.
fn linux_headers() -> HeaderMap {
    HeaderMap::from_iter([(
        header::HeaderName::from_static("app-platform"),
        HeaderValue::from_static("Linux"),
    )])
}
