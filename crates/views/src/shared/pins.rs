use music::{Album, Playlist, SavedArtist, Track};
use state::{Hit, Leaving, Origin, Whence};
use ui::{Pin, PinKind};

/// The origin tag of a song dragged from the favorites. A playlist's tag is its id.
const LIKED: &str = "liked";

pub(crate) trait Pinned {
    fn pin(&self) -> Option<Pin>;
}

impl Pinned for Track {
    fn pin(&self) -> Option<Pin> {
        let id = self.id.clone()?;

        Some(Pin::new(PinKind::Song, id, self.name.clone()).cover(self.cover.clone()))
    }
}

impl Pinned for Album {
    fn pin(&self) -> Option<Pin> {
        let cover = self.cover_large.clone().or_else(|| self.cover.clone());

        Some(Pin::new(PinKind::Album, self.id.clone(), self.name.clone()).cover(cover))
    }
}

impl Pinned for Playlist {
    fn pin(&self) -> Option<Pin> {
        Some(
            Pin::new(PinKind::Playlist, self.id.clone(), self.name.clone())
                .cover(self.cover.clone()),
        )
    }
}

impl Pinned for SavedArtist {
    fn pin(&self) -> Option<Pin> {
        Some(
            Pin::new(PinKind::Artist, self.id.clone(), self.name.clone()).cover(self.cover.clone()),
        )
    }
}

impl Pinned for Hit {
    fn pin(&self) -> Option<Pin> {
        let (kind, id, name, cover) = match self {
            Self::Song(track) => return track.pin(),
            Self::Artist(artist) => (
                PinKind::Artist,
                artist.id.clone()?,
                artist.name.clone(),
                artist.cover.clone(),
            ),
            Self::Album(album) => (
                PinKind::Album,
                album.id.clone(),
                album.name.clone(),
                album.cover.clone(),
            ),
            Self::Playlist(playlist) => (
                PinKind::Playlist,
                playlist.id.clone(),
                playlist.name.clone(),
                playlist.cover.clone(),
            ),
        };

        Some(Pin::new(kind, id, name).cover(cover))
    }
}

/// The drag tag for a song picked up from `origin`: the favorites or a playlist id, else `None`.
pub(crate) fn origin_tag(origin: &Origin) -> Option<String> {
    match origin.whence {
        Whence::Saved => Some(LIKED.to_owned()),
        Whence::Playlist => Some(origin.id.clone()),
        _ => None,
    }
}

/// Reads back a tag written by `origin_tag`.
pub(crate) fn leaving(tag: &str) -> Leaving {
    match tag {
        LIKED => Leaving::Liked,
        id => Leaving::Playlist(id.to_owned()),
    }
}
