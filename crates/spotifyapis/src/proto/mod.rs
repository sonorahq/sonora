//! Spotify service requests and their protobuf schemas.
//!
//! Commented-out fields inventory the wire format without adding fields or
//! allocations to decoded Rust messages.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Deserializer};

pub(super) mod access_point;
pub mod browsita;
pub mod casita;
pub mod collection;
pub mod color_lyrics;
pub mod connect_state;
pub mod extended_metadata;
pub mod list_read_service;
pub mod playlist_v2;
pub mod searchview;
pub mod sequence_proxy;
pub mod social_connect;
mod storage_resolve;
pub mod user_profile_view;

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

pub(super) fn between_devices(prefix: &str, from: &str, to: &str) -> String {
    format!("{prefix}/from/{}/to/{}", component(from), component(to))
}

pub(super) fn dealer_headers(connection_id: &str) -> anyhow::Result<HeaderMap> {
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
