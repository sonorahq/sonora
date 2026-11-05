//! TIDAL's links: tidal.com and listen.tidal.com pages, and the `tidal://` addresses its website
//! opens the app with.

use crate::link::{self, Linked};
use crate::tidal::wire::MIX;

const HOSTS: [&str; 2] = ["tidal.com", "listen.tidal.com"];

/// What a TIDAL link points at. A mix opens as the playlist the client reads mixes as.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)?;
    let mut segments = link::segments(&url);
    match url.scheme() {
        // the app's own addresses put the kind where a host would be, as in tidal://track/1
        "tidal" => segments.insert(0, url.host_str()?),
        "https" | "http" if HOSTS.contains(&link::host(&url)?) => {}
        _ => return None,
    }
    let at = segments
        .iter()
        .position(|part| matches!(*part, "track" | "album" | "artist" | "playlist" | "mix"))?;
    let id = (*segments.get(at + 1)?).to_owned();
    match segments[at] {
        "track" => Some(Linked::Track(id)),
        "album" => Some(Linked::Album(id)),
        "artist" => Some(Linked::Artist(id)),
        "mix" => Some(Linked::Playlist(format!("{MIX}{id}"))),
        _ => Some(Linked::Playlist(id)),
    }
}
