//! Apple Music's links: music.apple.com pages, with or without the name part before the id.

use crate::link::{self, Linked};

const HOSTS: [&str; 3] = ["music.apple.com", "geo.music.apple.com", "itunes.apple.com"];

/// What an Apple Music link points at. A song shared from its album page is the album's address
/// with the song in `?i=`, and it opens the song.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)
        .filter(|url| matches!(url.scheme(), "https" | "http" | "music" | "itms" | "itmss"))?;
    if !HOSTS.contains(&link::host(&url)?) {
        return None;
    }
    let segments = link::segments(&url);
    let at = segments
        .iter()
        .position(|part| matches!(*part, "album" | "song" | "artist" | "playlist"))?;
    let id = segments.get(at + 1..)?.last()?.to_string();
    match segments[at] {
        "album" => match link::query(&url, "i") {
            Some(song) => Some(Linked::Track(song)),
            None => Some(Linked::Album(id)),
        },
        "song" => Some(Linked::Track(id)),
        "artist" => Some(Linked::Artist(id)),
        _ => Some(Linked::Playlist(id)),
    }
}
