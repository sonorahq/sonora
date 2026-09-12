use gpui::prelude::*;
use gpui::{App, ElementId, Entity, FontWeight, SharedString, div};
use i18n::t;
use music::{Album, ArtistRef, Playlist, SavedArtist};
use router::{Destination, navigate};
use state::{FolderRow, Origin, Playback, PlaybackState};
use ui::{ActiveTheme as _, Card, Pin, PinKind, Pinnable, Text, Theme};

use crate::shared::cells;
use crate::shared::pins::Pinned as _;

const BULLET: SharedString = SharedString::new_static("·");
const FOLDER: &str = "icons/folder.svg";

pub(crate) fn album_card(
    id: impl Into<ElementId>,
    album: &Album,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    let cover = album.cover_large.clone().or_else(|| album.cover.clone());
    let origin = Origin::album(album.id.clone()).named(album.name.clone());
    let playing = matches!(
        playback.read(cx).playing_from(&origin),
        Some(PlaybackState::Playing)
    );
    let pin = album.pin();
    let opened = SharedString::from(album.id.clone());
    let toggled = playback.clone();

    Card::new(id, SharedString::from(album.name.clone()))
        .cover(cover)
        .weight(FontWeight::SEMIBOLD)
        .underline()
        .hint()
        .bare_meta(released(
            SharedString::new_static("album-card-artist"),
            album.year,
            album.artist_refs.clone(),
            album.artists.clone(),
            cx.theme(),
        ))
        .play(playing, move |_, _, cx| {
            toggled.update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
        })
        .press(move |_, _, cx| navigate(Destination::Album(opened.clone()), cx))
        .when_some(pin, Pinnable::pin)
}

pub(crate) fn playlist_card(
    id: impl Into<ElementId>,
    playlist: &Playlist,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    let origin = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
    let playing = matches!(
        playback.read(cx).playing_from(&origin),
        Some(PlaybackState::Playing)
    );
    let pin = playlist.pin();
    let opened = SharedString::from(playlist.id.clone());
    let toggled = playback.clone();

    Card::new(id, SharedString::from(playlist.name.clone()))
        .cover(playlist.cover.clone())
        .weight(FontWeight::SEMIBOLD)
        .underline()
        .meta(SharedString::from(playlist.owner.clone()))
        .play(playing, move |_, _, cx| {
            toggled.update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
        })
        .press(move |_, _, cx| navigate(Destination::Playlist(opened.clone()), cx))
        .when_some(pin, Pinnable::pin)
}

pub(crate) fn released(
    id: impl Into<SharedString>,
    year: i32,
    artists: Vec<ArtistRef>,
    fallback: impl Into<SharedString>,
    theme: &Theme,
) -> impl IntoElement {
    let small = theme.text(Text::Small);
    let muted = theme.muted_foreground;
    let year = match year {
        0 => None,
        year => Some(SharedString::from(year.to_string())),
    };
    let artists = cells::artist_links(id, artists, fallback, muted)
        .text_size(small)
        .truncate();

    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .text_size(small)
        .text_color(muted)
        .when_some(year, |this, year| {
            this.child(div().flex_none().child(year))
                .child(div().flex_none().child(BULLET))
        })
        .child(artists)
}

pub(crate) fn imported_playlist_card(
    id: impl Into<ElementId>,
    playlist: &Playlist,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    playlist_card(id, playlist, playback, cx).meta(t!("count-tracks", count = playlist.track_count))
}

pub(crate) fn artist_card(
    id: impl Into<ElementId>,
    artist: &SavedArtist,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    let origin = Origin::artist(artist.id.clone()).named(artist.name.clone());
    let playing = matches!(
        playback.read(cx).playing_from(&origin),
        Some(PlaybackState::Playing)
    );
    let pin = artist.pin();
    let opened = SharedString::from(artist.id.clone());
    let toggled = playback.clone();

    Card::new(id, SharedString::from(artist.name.clone()))
        .cover(artist.cover.clone())
        .circle()
        .weight(FontWeight::SEMIBOLD)
        .underline()
        .meta(i18n::lookup("artist-eyebrow", None))
        .play(playing, move |_, _, cx| {
            toggled.update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
        })
        .press(move |_, _, cx| navigate(Destination::Artist(opened.clone()), cx))
        .when_some(pin, Pinnable::pin)
}

/// A folder among the playlists it holds. It carries a mosaic of what is inside, marked by a
/// glyph so it cannot be mistaken for a playlist, and it opens rather than plays.
pub(crate) fn folder_card(
    id: impl Into<ElementId>,
    folder: &FolderRow,
    cover: Option<String>,
) -> Card {
    let opened = SharedString::from(folder.id.clone());
    let pin = folder_pin(folder, cover.clone());
    let covered = cover.is_some();

    Card::new(id, SharedString::from(folder.name.clone()))
        .cover(cover)
        .fallback(FOLDER)
        .when(covered, |card| card.glyph(FOLDER))
        .weight(FontWeight::SEMIBOLD)
        .underline()
        .meta(holding(folder))
        .press(move |_, _, cx| navigate(Destination::Folder(opened.clone()), cx))
        .pin(pin)
}

/// The pin a folder drags into the sidebar. It keeps the mosaic, so pinning does not turn the
/// card back into a bare glyph.
pub(crate) fn folder_pin(folder: &FolderRow, cover: Option<String>) -> Pin {
    Pin::new(PinKind::Folder, folder.id.clone(), folder.name.clone()).cover(cover)
}

/// What a folder holds, counting only what sits directly inside it.
pub(crate) fn holding(folder: &FolderRow) -> SharedString {
    match folder.folders {
        0 => t!("count-playlists", count = folder.playlists),
        folders => SharedString::from(format!(
            "{} {BULLET} {}",
            t!("count-playlists", count = folder.playlists),
            t!("count-folders", count = folders)
        )),
    }
}
