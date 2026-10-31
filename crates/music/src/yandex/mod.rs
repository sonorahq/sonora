//! Yandex Music: sign-in with an OAuth token, and the catalogue, the account's likes and
//! playlists over the mobile api the Android app uses.

mod client;
mod wire;

pub use client::YandexClient;
