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
    fn reads_every_uri_kind() {
        assert_eq!(
            read("spotify:track:6rqhFgbbKwnb9MLmUQDhG6"),
            Some(Linked::Track("6rqhFgbbKwnb9MLmUQDhG6".into()))
        );
        assert_eq!(
            read("spotify:album:1DFixLWuPkv3KT3TnV35m3"),
            Some(Linked::Album("1DFixLWuPkv3KT3TnV35m3".into()))
        );
        assert_eq!(
            read("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            Some(Linked::Playlist("37i9dQZF1DXcBWIGoYBM5M".into()))
        );
        assert_eq!(
            read("spotify:artist:0TnOYISbd1XYRBk9myaseg"),
            Some(Linked::Artist("0TnOYISbd1XYRBk9myaseg".into()))
        );
    }

    #[test]
    fn reads_web_links() {
        assert_eq!(
            read("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6"),
            Some(Linked::Track("6rqhFgbbKwnb9MLmUQDhG6".into()))
        );
        assert_eq!(
            read("http://www.open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3/"),
            Some(Linked::Album("1DFixLWuPkv3KT3TnV35m3".into()))
        );
    }

    #[test]
    fn drops_the_share_query() {
        assert_eq!(
            read("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6?si=abc123"),
            Some(Linked::Track("6rqhFgbbKwnb9MLmUQDhG6".into()))
        );
    }

    #[test]
    fn skips_the_locale_segment() {
        assert_eq!(
            read("https://open.spotify.com/intl-de/artist/0TnOYISbd1XYRBk9myaseg"),
            Some(Linked::Artist("0TnOYISbd1XYRBk9myaseg".into()))
        );
    }

    #[test]
    fn reads_the_legacy_user_playlist_uri() {
        assert_eq!(
            read("spotify:user:spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            Some(Linked::Playlist("37i9dQZF1DXcBWIGoYBM5M".into()))
        );
    }

    #[test]
    fn refuses_what_it_cannot_route() {
        assert_eq!(read("spotify:show:4rOoJ6Egrf8K2IrywzwOMk"), None);
        assert_eq!(read("spotify:track:"), None);
        assert_eq!(read("spotify:track"), None);
        assert_eq!(read("https://example.com/track/abc"), None);
        assert_eq!(read("nonsense"), None);
        assert_eq!(read(""), None);
    }
}
