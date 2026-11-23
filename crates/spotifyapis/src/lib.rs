//! Spotify OAuth, service discovery, and protobuf APIs.
//!
//! Restore or authorize an account with [auth::OAuth], then create a [Session] to
//! send requests on Tokio. The crate shares its HTTP clients and returns Spotify's
//! wire types so callers can map them into their own models. For native audio,
//! use [Session::audio_urls] and [access_point::AudioKeys] with your own downloader.

pub mod access_point;
pub mod auth;
pub mod dealer;
mod http;
pub mod id;
mod image_upload;
mod player_command;
pub mod proto;
mod session;

pub use http::{media_client, reachable, status as http_status};
pub use session::Session;
