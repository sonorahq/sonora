//! SoundCloud's links: `soundcloud.com` pages and the `on.soundcloud.com` short links its
//! share sheet hands out. A page's path is a permalink, not the numeric id the api wants, so
//! the client resolves it when it is asked to load the item.

use crate::link::{self, Linked};

const HOST: &str = "soundcloud.com";
const SHORT_HOSTS: [&str; 2] = ["on.soundcloud.com", "sc.com"];

/// What a SoundCloud link points at. A bare user name is the user's own page, two path parts a
/// track, and a `sets` path a playlist (SoundCloud's albums share the `sets` shape).
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)?;
    let host = link::host(&url)?;
    let segments = link::segments(&url);
    match url.scheme() {
        "https" | "http" if SHORT_HOSTS.contains(&host) => return Some(Linked::Short),
        "https" | "http" if host == HOST => {}
        _ => return None,
    }
    match segments.as_slice() {
        [user] => Some(Linked::User((*user).to_owned())),
        [user, "sets", slug, ..] => Some(Linked::Playlist(format!("{user}/sets/{slug}"))),
        [user, slug, ..] => Some(Linked::Track(format!("{user}/{slug}"))),
        _ => None,
    }
}
