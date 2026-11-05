//! Deezer's links: deezer.com pages, the `deezer://` addresses its website opens the app with in
//! both the `deezer://www.deezer.com/track/1` and `deezer://track/1` shapes, and the short links
//! its share sheet hands out.

use crate::link::{self, Linked};

const HOST: &str = "deezer.com";
const SHORT_HOSTS: [&str; 3] = ["link.deezer.com", "deezer.page.link", "dzr.page.link"];

/// What a Deezer link points at. The language part some pages carry before the kind is skipped.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)?;
    let host = link::host(&url)?;
    let mut segments = link::segments(&url);
    match url.scheme() {
        "https" | "http" | "deezer" if SHORT_HOSTS.contains(&host) => return Some(Linked::Short),
        "https" | "http" | "deezer" if host == HOST => {}
        // the app's own addresses may also put the kind where a host would be, as in
        // deezer://track/1
        "deezer" => segments.insert(0, host),
        _ => return None,
    }
    let at = segments
        .iter()
        .position(|part| matches!(*part, "track" | "album" | "artist" | "playlist"))?;
    let id = segments
        .get(at + 1)
        .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))?
        .to_string();
    match segments[at] {
        "track" => Some(Linked::Track(id)),
        "album" => Some(Linked::Album(id)),
        "artist" => Some(Linked::Artist(id)),
        _ => Some(Linked::Playlist(id)),
    }
}
