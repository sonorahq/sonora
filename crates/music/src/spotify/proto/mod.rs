//! Spotify protobuf schemas.
//!
//! Commented-out fields inventory the wire format without adding fields or
//! allocations to decoded Rust messages.

pub(super) mod audio;
pub(super) mod collection;
pub(super) mod connect;
pub(super) mod feeds;
pub(super) mod lyrics;
pub(super) mod metadata;
pub(super) mod playlist;
pub(super) mod profile;
pub(super) mod search;
pub(super) mod sequence;
