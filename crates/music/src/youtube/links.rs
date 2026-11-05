//! YouTube Music's links, and the youtube.com and youtu.be ones that name the same videos,
//! playlists and channels.

use crate::link::{self, Linked};

const HOSTS: [&str; 3] = ["music.youtube.com", "youtube.com", "youtu.be"];

/// What a YouTube link points at. A video opens as a track and a channel as an artist.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text).filter(|url| matches!(url.scheme(), "https" | "http"))?;
    let host = link::host(&url)?;
    if !HOSTS.contains(&host) {
        return None;
    }
    let segments = link::segments(&url);
    if host == "youtu.be" {
        return segments.first().map(|id| Linked::Track((*id).to_owned()));
    }
    match segments.as_slice() {
        ["watch", ..] => link::query(&url, "v").map(Linked::Track),
        ["playlist", ..] => link::query(&url, "list").map(Linked::Playlist),
        ["channel", id, ..] => Some(Linked::Artist((*id).to_owned())),
        ["browse", id, ..] => browsed(id),
        _ => None,
    }
}

/// What a browse id names, which its prefix tells. A playlist loses its `VL`, since the client
/// takes playlist ids without it.
fn browsed(id: &str) -> Option<Linked> {
    if let Some(playlist) = id.strip_prefix("VL") {
        return Some(Linked::Playlist(playlist.to_owned()));
    }
    if id.starts_with("MPREb_") {
        return Some(Linked::Album(id.to_owned()));
    }
    id.starts_with("UC").then(|| Linked::Artist(id.to_owned()))
}
