//! Yandex Music's links: music.yandex pages on every regional domain, and the `yandexmusic://`
//! addresses the website opens the app with, which put the kind where a host would be.

use crate::link::{self, Linked};
use crate::yandex::wire;

/// The regional sites, which all lead to the same catalogue.
const HOSTS: [&str; 6] = [
    "music.yandex.ru",
    "music.yandex.com",
    "music.yandex.by",
    "music.yandex.kz",
    "music.yandex.uz",
    "music.yandex.com.tr",
];

/// What a Yandex Music link points at. A track inside an album, as the site shows it, comes
/// back as the track.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)?;
    let host = link::host(&url)?;
    let mut segments = link::segments(&url);
    match url.scheme() {
        "https" | "http" if HOSTS.contains(&host) => {}
        "yandexmusic" => segments.insert(0, host),
        _ => return None,
    }
    let numeric = |id: &&str| id.bytes().all(|byte| byte.is_ascii_digit());
    match segments.as_slice() {
        ["album", _, "track", track, ..] | ["track", track, ..] if numeric(track) => {
            Some(Linked::Track((*track).to_owned()))
        }
        ["album", album, ..] if numeric(album) => Some(Linked::Album((*album).to_owned())),
        ["artist", artist, ..] if numeric(artist) => Some(Linked::Artist((*artist).to_owned())),
        ["users", owner, "playlists", kind, ..] if numeric(kind) => {
            Some(Linked::Playlist(wire::playlist_id(owner, kind)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_links_read() {
        let cases = [
            (
                "https://music.yandex.ru/album/123/track/456",
                Linked::Track("456".into()),
            ),
            (
                "https://music.yandex.com/track/456?utm_source=x",
                Linked::Track("456".into()),
            ),
            (
                "https://music.yandex.ru/album/123",
                Linked::Album("123".into()),
            ),
            (
                "https://music.yandex.kz/artist/789/tracks",
                Linked::Artist("789".into()),
            ),
            (
                "https://music.yandex.ru/users/someone/playlists/1003",
                Linked::Playlist("someone:1003".into()),
            ),
            ("yandexmusic://album/1/track/2", Linked::Track("2".into())),
        ];
        for (text, expected) in cases {
            assert_eq!(read(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn other_links_do_not() {
        assert_eq!(read("https://example.com/album/123"), None);
        assert_eq!(read("https://music.yandex.ru/home"), None);
        assert_eq!(read("https://music.yandex.ru/album/abc"), None);
    }
}
