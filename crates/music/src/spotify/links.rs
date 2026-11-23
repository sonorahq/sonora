//! Spotify's links: `spotify:` and `spotify://` URIs, open.spotify.com pages and spotify.link short
//! links.

use crate::link::{self, Linked};
use crate::spotify::connect::INVITE;

const SCHEME: &str = "spotify:";
const HOST: &str = "open.spotify.com";
const SHORT_HOST: &str = "spotify.link";
/// The prefix of the locale part some page addresses carry before the kind, as in `intl-de`.
const REGION: &str = "intl-";

/// What a Spotify link points at.
pub(super) fn read(text: &str) -> Option<Linked> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix(SCHEME) {
        // `spotify://track/<id>` names the same thing as `spotify:track:<id>`
        return match rest.strip_prefix('/') {
            Some(path) => from_parts(path.split('/').filter(|part| !part.is_empty())),
            None => from_parts(rest.split(':')),
        };
    }
    let url = link::address(text).filter(|url| matches!(url.scheme(), "https" | "http"))?;
    match link::host(&url)? {
        SHORT_HOST => Some(Linked::Short),
        HOST => {
            let segments = link::segments(&url);
            let segments = match segments.first() {
                Some(region) if region.starts_with(REGION) => &segments[1..],
                _ => &segments[..],
            };
            from_parts(segments.iter().copied())
        }
        _ => None,
    }
}

/// Reads the kind and ID from a Spotify URI or web path.
fn from_parts<'a>(mut parts: impl Iterator<Item = &'a str>) -> Option<Linked> {
    let kind = parts.next()?;
    let id = parts.next()?;
    if kind == "user" && parts.next().is_some() {
        return None;
    }
    route(kind, id)
}

fn route(kind: &str, id: &str) -> Option<Linked> {
    let id = id.split(['?', '#']).next().filter(|id| !id.is_empty())?;
    let id = id.to_owned();
    match kind {
        "track" => Some(Linked::Track(id)),
        "album" => Some(Linked::Album(id)),
        "playlist" => Some(Linked::Playlist(id)),
        "artist" => Some(Linked::Artist(id)),
        "user" => Some(Linked::User(id)),
        "socialsession" => Some(Linked::Jam(format!("{INVITE}{id}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_links_preserve_kind_and_reject_ambiguous_addresses() {
        for (url, target) in [
            ("spotify://album/a", Some(Linked::Album("a".into()))),
            (
                "https://open.spotify.com/intl-de/track/t?si=x",
                Some(Linked::Track("t".into())),
            ),
            ("https://spotify.link/a", Some(Linked::Short)),
            ("https://open.spotify.com.example.org/track/t", None),
            ("spotify:user:u:playlist:p", None),
            ("spotify:track:", None),
        ] {
            assert_eq!(read(url), target, "{url}");
        }
    }
}
