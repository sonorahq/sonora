//! Spotify OAuth, service discovery, and protobuf APIs.
//!
//! Restore or authorize an account with [auth::OAuth], then create a [Session] to
//! send requests on Tokio. The crate shares its HTTP clients and returns Spotify's
//! wire types so callers can map them into their own models.

pub mod auth;
pub mod dealer;
pub mod http;
pub mod proto;
mod services;
mod session;

pub use services::SearchKind;
pub use session::Session;
