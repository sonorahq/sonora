//! Yandex Music: sign-in with an OAuth token, the catalogue, the account's likes and playlists
//! over the mobile api the Android app uses, and files that arrive AES-encrypted.

mod client;
mod decrypt;
mod playback;
mod wire;

pub use client::YandexClient;
pub use decrypt::Sealed;
pub use playback::Factory;

/// A Yandex track downloading and decrypting as it plays.
pub type Stream = crate::stream::Stream<Sealed>;
