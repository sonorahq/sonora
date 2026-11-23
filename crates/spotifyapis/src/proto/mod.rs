//! Spotify protobuf schemas.
//!
//! Commented-out fields inventory the wire format without adding fields or
//! allocations to decoded Rust messages.

use serde::{Deserialize, Deserializer};

pub mod access_point;
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
pub mod storage;

// Spotify's ProtoJSON can use null for an absent scalar, map, or list.
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    input: D,
) -> Result<T, D::Error> {
    Ok(Option::deserialize(input)?.unwrap_or_default())
}
