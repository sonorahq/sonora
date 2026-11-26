//! The sources that serve nothing but lyrics: LRCLIB, Musixmatch, NetEase, Kugou and lrc.red.
//! Each is a `music::LyricsProvider` that reads its sheets through the `lyrics` crate. Only
//! `sonora` names them, so an edit here rebuilds this crate and the binary alone.

pub mod kugou;
pub mod lrclib;
pub mod lrcred;
pub mod musixmatch;
pub mod netease;
