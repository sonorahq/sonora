use gpui::prelude::*;
use gpui::{App, ElementId, Entity, FontWeight, SharedString, div};
use i18n::t;
use music::{Album, ArtistRef, Genre, GenreItem, Playlist, ReleaseType, SavedArtist, Track};
use router::{Destination, navigate};
use state::{FolderRow, Origin, Playback, PlaybackState};
use ui::{ActiveTheme as _, Card, InlineLinks, Pin, PinKind, Pinnable, Text, Theme};

use crate::shared::cells;
use crate::shared::menus::{CardMenu, Item};
use crate::shared::pins::Pinned as _;

const BULLET: SharedString = SharedString::new_static("·");
/// How many of a release's credits a card lists. Anything past this is clipped by the card's
/// own width before it can be read.
const CARD_LINKS: usize = 3;
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
            Some(album.release_type),
            &album.artist_refs,
            album.artists.clone(),
            cx.theme(),
        ))
        .play(playing, move |_, _, cx| {
            toggled.update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
        })
        .press(move |_, _, cx| navigate(Destination::Album(opened.clone()), cx))
        .menu(CardMenu::opener(
            Item::Album(album.clone()),
            playback.clone(),
        ))
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
        .menu(CardMenu::opener(
            Item::Playlist(playlist.clone()),
            playback.clone(),
        ))
        .when_some(pin, Pinnable::pin)
}

/// The card of a track with nothing wired to play it: name, cover, explicit mark, pin and the
/// shared context menu, tinted while it is the one playing. Whoever lists it adds the caption
/// and the play and press it wants, and `track_status` tells them where playback stands.
pub(crate) fn track_card(
    id: impl Into<ElementId>,
    track: &Track,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    let theme = *cx.theme();
    let (current, _) = track_status(track, playback, cx);
    let tint = match current {
        true => theme.primary,
        false => theme.foreground,
    };

    Card::new(id, SharedString::from(track.name.clone()))
        .cover(track.cover.clone())
        .tint(tint)
        .hint()
        .when(track.explicit, Card::explicit)
        .menu(CardMenu::opener(
            Item::Track(track.clone()),
            playback.clone(),
        ))
        .when_some(track.pin(), Pinnable::pin)
}

/// Whether the track is the one in the player, and whether that one is playing.
pub(crate) fn track_status(track: &Track, playback: &Entity<Playback>, cx: &App) -> (bool, bool) {
    let playback = playback.read(cx);
    let current = track.id.is_some()
        && playback.track().and_then(|playing| playing.id.as_deref()) == track.id.as_deref();

    (
        current,
        current && playback.state() == &PlaybackState::Playing,
    )
}

/// The artists of a track as the small muted links under its name.
pub(crate) fn track_artists(
    id: impl Into<SharedString>,
    track: &Track,
    theme: &Theme,
) -> InlineLinks {
    cells::artist_links(
        id,
        track.artist_refs.clone(),
        track.artists.clone(),
        theme.muted_foreground,
    )
    .text_size(theme.text(Text::Small))
    .truncate()
}

/// A shelf item as a card, whatever it is: an album, playlist or artist opens its page and
/// plays from its play control, a track starts its radio, a genre opens its page.
pub(crate) fn item_card(
    id: impl Into<ElementId>,
    item: &GenreItem,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    match item {
        GenreItem::Album(album) => album_card(id, album, playback, cx),
        GenreItem::Playlist(playlist) => {
            playlist_card(id, playlist, playback, cx).meta(playlist_meta(playlist))
        }
        GenreItem::Artist(artist) => artist_card(id, artist, playback, cx),
        GenreItem::Genre(genre) => genre_card(id, genre),
        GenreItem::Track(track) => {
            let theme = *cx.theme();
            let (current, playing) = track_status(track, playback, cx);
            let toggled = playback.clone();
            let pressed = playback.clone();
            let seed = track.clone();
            let restarted = track.clone();

            track_card(id, track, playback, cx)
                .weight(FontWeight::SEMIBOLD)
                .bare_meta(track_artists(
                    SharedString::new_static("item-card-artist"),
                    track,
                    &theme,
                ))
                .play(playing, move |_, _, cx| match current {
                    true => toggled.update(cx, |playback, cx| playback.toggle_play(cx)),
                    false => toggled.update(cx, |playback, cx| playback.play_radio(&seed, cx)),
                })
                .press(move |_, _, cx| {
                    pressed.update(cx, |playback, cx| playback.play_radio(&restarted, cx));
                })
        }
    }
}

pub(crate) fn genre_card(id: impl Into<ElementId>, genre: &Genre) -> Card {
    let opened = SharedString::from(genre.id.clone());

    Card::new(id, SharedString::from(genre.name.clone()))
        .cover(genre.cover.clone())
        .fallback("icons/music.svg")
        .weight(FontWeight::SEMIBOLD)
        .press(move |_, _, cx| navigate(Destination::Genre(opened.clone()), cx))
}

/// Which releases a release listing keeps: everything, or one kind of release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseFilter {
    All,
    Albums,
    Singles,
    Eps,
}

impl ReleaseFilter {
    const ALL: [Self; 4] = [Self::All, Self::Singles, Self::Albums, Self::Eps];

    /// The element id of the filter's pill.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::All => "release-filter-all",
            Self::Albums => "release-filter-albums",
            Self::Singles => "release-filter-singles",
            Self::Eps => "release-filter-eps",
        }
    }

    /// The pill's label, resolved for the active language.
    pub(crate) fn label(self) -> SharedString {
        match self {
            Self::All => t!("artist-filter-all"),
            Self::Albums => t!("artist-filter-albums"),
            Self::Singles => t!("artist-filter-singles"),
            Self::Eps => t!("artist-filter-eps"),
        }
    }

    /// Whether a release of this kind passes the filter.
    pub(crate) fn matches(self, kind: ReleaseType) -> bool {
        self == Self::All
            || matches!(
                (self, kind),
                (Self::Albums, ReleaseType::Album)
                    | (Self::Singles, ReleaseType::Single)
                    | (Self::Eps, ReleaseType::Ep)
            )
    }
}

/// The filters a listing earns: all of them, and one per kind the listing holds. A local
/// listing has none, since its files carry no release kinds to split by.
pub(crate) fn release_filters(
    local: bool,
    releases: impl IntoIterator<Item = ReleaseType>,
) -> Vec<ReleaseFilter> {
    if local {
        return Vec::new();
    }
    let releases = releases.into_iter().collect::<Vec<_>>();
    ReleaseFilter::ALL
        .into_iter()
        .filter(|filter| {
            *filter == ReleaseFilter::All || releases.iter().any(|release| filter.matches(*release))
        })
        .collect()
}

/// A shelf item as one row of a list: `item_card` at the plain weight of a listed row, and
/// a track or a playlist saying what it is under its name, so a mix is never mistaken for a
/// song: "Song · Artist", "Playlist · Made for you · 50 songs".
pub(crate) fn listed(
    id: impl Into<ElementId>,
    item: &GenreItem,
    playback: &Entity<Playback>,
    cx: &App,
) -> Card {
    let theme = *cx.theme();
    let card = item_card(id, item, playback, cx).weight(FontWeight::NORMAL);

    match item {
        GenreItem::Track(track) => card.bare_meta(tagged(
            t!("kind-song"),
            track_artists(SharedString::new_static("listed-artist"), track, &theme),
            &theme,
        )),
        GenreItem::Playlist(playlist) => {
            card.meta(tagged(t!("kind-playlist"), playlist_meta(playlist), &theme))
        }
        _ => card,
    }
}

/// A caption led by what kind of thing it captions: "Song · " and then the rest.
fn tagged(kind: SharedString, rest: impl IntoElement, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .child(div().flex_none().child(format!("{kind} {BULLET}")))
        .child(div().min_w_0().truncate().child(rest))
}

/// The line under a playlist on a shelf: who it is by, and how many songs when that is known,
/// so a recap reads "Made for you · 50 songs" the way its provider lists it. A library card
/// keeps the owner alone.
fn playlist_meta(playlist: &Playlist) -> SharedString {
    let owner = playlist.owner.as_str();
    match (owner.is_empty(), playlist.track_count) {
        (_, 0) => SharedString::from(owner.to_owned()),
        (true, count) => t!("count-songs", count = count),
        (false, count) => SharedString::from(format!(
            "{owner} {BULLET} {}",
            t!("count-songs", count = count)
        )),
    }
}

/// The i18n key naming a release kind.
pub(crate) fn release_key(kind: ReleaseType) -> &'static str {
    match kind {
        ReleaseType::Album => "release-album",
        ReleaseType::Single => "release-single",
        ReleaseType::Compilation => "release-compilation",
        ReleaseType::Ep => "release-ep",
        ReleaseType::Audiobook => "release-audiobook",
        ReleaseType::Podcast => "release-podcast",
    }
}

/// The line under an album: the year and the artists, or the kind of release in the year's
/// place when the provider gave none, so a new single still says it is one.
/// The eyebrow under a release's title: its year, or its kind when the year is unknown, and
/// who it is credited to, up to `CARD_LINKS` of them. The line is clipped to the card's
/// width, so a compilation's twentieth credit could never be read anyway, while every name
/// past the first few costs the grid an interactive element and a string on every frame a
/// scroll asks for.
pub(crate) fn released(
    id: impl Into<SharedString>,
    year: i32,
    kind: Option<ReleaseType>,
    artists: &[ArtistRef],
    fallback: impl Into<SharedString>,
    theme: &Theme,
) -> impl IntoElement {
    let small = theme.text(Text::Small);
    let muted = theme.muted_foreground;
    let year = match year {
        0 => kind.map(|kind| i18n::lookup(release_key(kind), None)),
        year => Some(SharedString::from(year.to_string())),
    };
    let credited: Vec<ArtistRef> = artists.iter().take(CARD_LINKS).cloned().collect();
    let artists = cells::artist_links(id, credited, fallback, muted)
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
        .menu(CardMenu::opener(
            Item::Artist(artist.clone()),
            playback.clone(),
        ))
        .when_some(pin, Pinnable::pin)
}

/// A folder among the playlists it holds. It carries a mosaic of what is inside, marked by a
/// glyph so it cannot be mistaken for a playlist, and it opens rather than plays.
pub(crate) fn folder_card(
    id: impl Into<ElementId>,
    folder: &FolderRow,
    cover: Option<String>,
    playback: &Entity<Playback>,
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
        .menu(CardMenu::opener(
            Item::Folder(folder.clone()),
            playback.clone(),
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_releases_have_no_filters() {
        assert!(release_filters(true, [ReleaseType::Album]).is_empty());
    }

    #[test]
    fn listed_releases_only_show_populated_filters() {
        assert_eq!(
            release_filters(false, [ReleaseType::Album, ReleaseType::Single]),
            [
                ReleaseFilter::All,
                ReleaseFilter::Singles,
                ReleaseFilter::Albums,
            ]
        );
    }
}
