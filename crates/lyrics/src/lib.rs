//! The lyrics sources that are not a music provider: LRCLIB, Musixmatch, NetEase, Kugou and
//! Binimum. Each is a `music::LyricsProvider`. The ranking, parsing and romanisation every
//! source shares is `music::lyrics`, and a provider with lyrics of its own keeps them in its
//! own crate.

pub mod binimum;
pub mod kugou;
pub mod lrclib;
pub mod musixmatch;
pub mod netease;
