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

/// Reads the parts of a URI after `spotify:` or of a page path after the locale, including the
/// old form that names a playlist under its owner as `user/<owner>/playlist/<id>`.
fn from_parts<'a>(mut parts: impl Iterator<Item = &'a str>) -> Option<Linked> {
    match parts.next()? {
        "user" => {
            let user = parts.next()?;
            match (parts.next(), parts.next()) {
                (Some(kind), Some(id)) => route(kind, id),
                _ => route("user", user),
            }
        }
        kind => route(kind, parts.next()?),
    }
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
    fn recognises_spotify_link_forms() {
        for (link, expected) in [
            ("spotify:track:t", Linked::Track("t".into())),
            ("spotify://album/a", Linked::Album("a".into())),
            ("spotify:user:u:playlist:p", Linked::Playlist("p".into())),
            (
                "https://open.spotify.com/intl-de/artist/a?si=abc",
                Linked::Artist("a".into()),
            ),
            (
                "http://www.open.spotify.com/user/u/",
                Linked::User("u".into()),
            ),
            ("https://spotify.link/abc", Linked::Short),
        ] {
            assert_eq!(read(link), Some(expected), "{link}");
        }
    }

    #[test]
    fn refuses_foreign_hosts_and_unroutable_uris() {
        for link in [
            "https://example.com/track/t",
            "spotify:show:s",
            "spotify:track:",
            "spotify:track",
        ] {
            assert_eq!(read(link), None, "{link}");
        }
    }
}
