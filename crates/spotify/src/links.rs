//! Spotify URI, web, and short-link handling.

use crate::connect::INVITE;
use music::link::{self, Linked};

const SCHEME: &str = "spotify:";
const HOST: &str = "open.spotify.com";
const SHORT_HOST: &str = "spotify.link";
/// Prefix for optional locale segments such as `intl-de`.
const REGION: &str = "intl-";

/// Entity referenced by a Spotify link.
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

/// Reads the kind and ID from a Spotify URI or web path, including the old form that names a
/// playlist under its owner as `user/<owner>/playlist/<id>`.
fn from_parts<'a>(mut parts: impl Iterator<Item = &'a str>) -> Option<Linked> {
    match parts.next()? {
        "user" => {
            let user = parts.next()?;
            match (parts.next(), parts.next()) {
                (Some("playlist"), Some(id)) => route("playlist", id),
                (None, _) => route("user", user),
                _ => None,
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
    fn spotify_links_preserve_kind_and_reject_ambiguous_addresses() {
        for (url, target) in [
            ("spotify://album/a", Some(Linked::Album("a".into()))),
            (
                "https://open.spotify.com/intl-de/track/t?si=x",
                Some(Linked::Track("t".into())),
            ),
            ("https://spotify.link/a", Some(Linked::Short)),
            ("https://open.spotify.com.example.org/track/t", None),
            (
                "spotify:user:u:playlist:p",
                Some(Linked::Playlist("p".into())),
            ),
            (
                "https://open.spotify.com/user/u/playlist/p",
                Some(Linked::Playlist("p".into())),
            ),
            ("spotify:user:u:collection", None),
            ("spotify:track:", None),
        ] {
            assert_eq!(read(url), target, "{url}");
        }
    }
}
