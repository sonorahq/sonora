//! Apple Music's links: music.apple.com pages, with or without the name part before the id.

use music::link::{self, Linked};

const HOSTS: [&str; 3] = ["music.apple.com", "geo.music.apple.com", "itunes.apple.com"];

/// What an Apple Music link points at. A song shared from its album page is the album's address
/// with the song in `?i=`, and it opens the song. A station opens as the playlist the client
/// reads stations as: its `ra.` id never collides with a playlist's.
pub(super) fn read(text: &str) -> Option<Linked> {
    let url = link::address(text)
        .filter(|url| matches!(url.scheme(), "https" | "http" | "music" | "itms" | "itmss"))?;
    if !HOSTS.contains(&link::host(&url)?) {
        return None;
    }
    let segments = link::segments(&url);
    let at = segments
        .iter()
        .position(|part| matches!(*part, "album" | "song" | "artist" | "playlist" | "station"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_station_opens_as_a_playlist() {
        assert_eq!(
            read(
                "https://music.apple.com/de/station/peters-sender/ra.u-6e4d72dd33513ccc1abb70ff757aa95f"
            ),
            Some(Linked::Playlist(
                "ra.u-6e4d72dd33513ccc1abb70ff757aa95f".into()
            ))
        );
        assert_eq!(
            read("https://music.apple.com/us/station/ra.978194965"),
            Some(Linked::Playlist("ra.978194965".into()))
        );
    }

    #[test]
    fn a_playlist_still_opens_as_one() {
        assert_eq!(
            read("https://music.apple.com/de/playlist/today/pl.f4d106fed2bd41149aaacabb233eb5eb"),
            Some(Linked::Playlist(
                "pl.f4d106fed2bd41149aaacabb233eb5eb".into()
            ))
        );
    }
}
