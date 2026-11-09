//! Cards for whatever a shelf or a grid lists: albums, playlists, artists, genres and tracks,
//! each with its cover, its caption and a play button that plays through the store. Each
//! returns the [`ui::Card`] builder, ready to build, so a caller can still dress it.

use std::rc::Rc;

use i18n::t;
use music::{Album, ArtistRef, Genre, GenreItem, Playlist, ReleaseType, SavedArtist, Track};
use router::Destination;
use rune::{El, PointerPress, TextStyle, Tx, row, text};
use state::Origin;
use ui::{Card, Cover, Text, Theme, Weight};

use crate::Ui;
use crate::shared::cells;

const BULLET: &str = "·";
/// How many of a release's credits a card lists. Anything past this is clipped by the card's
/// own width before it can be read.
const CARD_LINKS: usize = 3;
/// The space between the parts of a caption.
const CAPTION_GAP: f32 = 4.;

/// Opens the context menu of a shelf item, at the window point a right-click landed on. The
/// screen owns the menu, so a card only reports what it shows.
pub(crate) type OnItem = Rc<dyn Fn(&mut Ui, GenreItem, PointerPress, &mut Tx)>;

/// How a card is laid out: a tile `width` wide in a grid or a rail, its art square over the
/// text, or a list row with the art beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    Tile(f32),
    Row,
}

impl Fit {
    /// The side of the art at this fit.
    fn art(self, theme: &Theme) -> f32 {
        match self {
            Self::Tile(width) => width,
            Self::Row => (theme.metrics.list_row.round() - theme.metrics.pad * 2.).round(),
        }
    }
}

/// A card for `title` with `cover` as its art, laid out to `fit`. Round art for `circle`.
fn base(
    ui: &Ui,
    key: impl Into<String>,
    title: impl Into<String>,
    cover: Option<&str>,
    fit: Fit,
    circle: bool,
    fallback: Option<&str>,
) -> Card {
    let theme = &ui.theme;
    let side = fit.art(theme);
    let mut art = Cover::new(ui.artwork(), cover).size(side);
    if circle {
        art = art.circle();
    } else if let Fit::Tile(_) = fit {
        art = art.radius(theme.radius);
    }
    if let Some(fallback) = fallback {
        art = art.fallback(fallback);
    }
    let card = Card::new(key, title)
        .art(art.build(theme))
        .play_fill(theme.cover_fill(cover.and_then(|url| ui.artwork().cover_palette(url))));
    let card = match fit {
        Fit::Tile(width) => card.tile(width),
        Fit::Row => card,
    };
    match circle {
        true => card.circle(),
        false => card,
    }
}

/// Hangs the screen's menu for `item` on a card.
fn menu(card: Card, item: GenreItem, on_item: Option<&OnItem>) -> Card {
    match on_item {
        Some(on_item) => {
            let on_item = on_item.clone();
            card.menu(move |ui: &mut Ui, at: PointerPress, tx: &mut Tx| {
                on_item(ui, item.clone(), at, tx)
            })
        }
        None => card,
    }
}

/// Whether the player plays from `origin` right now.
fn playing_from(ui: &Ui, origin: &Origin) -> bool {
    ui.store
        .playback
        .playing_from(origin, &ui.store.queue)
        == Some(true)
}

/// The play button of a collection's card: pauses or resumes while the queue came from it,
/// starts it otherwise.
fn toggle(origin: Origin) -> impl Fn(&mut Ui, &mut Tx) + 'static {
    move |ui: &mut Ui, _: &mut Tx| {
        ui.store.toggle_collection(&origin);
        ui.settle();
    }
}

/// An album's card: its cover, its year and artists, opening its page.
pub(crate) fn album_card(
    ui: &Ui,
    key: impl Into<String>,
    album: &Album,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    let cover = album.cover_large.as_deref().or(album.cover.as_deref());
    let origin = Origin::album(album.id.clone()).named(album.name.clone());
    let playing = playing_from(ui, &origin);
    let opened = Destination::Album(album.id.as_str().into());
    let card = base(ui, key, album.name.clone(), cover, fit, false, None)
        .description(spoken_caption([
            year_label(album.year, Some(album.release_type)).unwrap_or_default(),
            album.artists.clone(),
        ]))
        .weight(Weight::SEMIBOLD)
        .hint()
        .bare_meta(released(
            "album-card-artist",
            album.year,
            Some(album.release_type),
            &album.artist_refs,
            album.artists.clone(),
            theme,
        ))
        .play(playing, toggle(origin))
        .on_press(move |ui: &mut Ui| ui.go(opened.clone()));
    menu(card, GenreItem::Album(album.clone()), on_item)
}

/// A playlist's card: its cover and owner, opening its page.
pub(crate) fn playlist_card(
    ui: &Ui,
    key: impl Into<String>,
    playlist: &Playlist,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    let origin = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
    let playing = playing_from(ui, &origin);
    let opened = Destination::Playlist(playlist.id.as_str().into());
    let card = base(
        ui,
        key,
        playlist.name.clone(),
        playlist.cover.as_deref(),
        fit,
        false,
        None,
    )
    .description(playlist.owner.clone())
    .weight(Weight::SEMIBOLD)
    .meta(playlist.owner.clone(), theme)
    .play(playing, toggle(origin))
    .on_press(move |ui: &mut Ui| ui.go(opened.clone()));
    menu(card, GenreItem::Playlist(playlist.clone()), on_item)
}

/// A playlist's card with its track count under it, for a playlist just imported.
pub(crate) fn imported_playlist_card(
    ui: &Ui,
    key: impl Into<String>,
    playlist: &Playlist,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let count = t!("count-tracks", count = playlist.track_count).into_owned();
    playlist_card(ui, key, playlist, fit, on_item)
        .description(count.clone())
        .meta(count, &ui.theme)
}

/// An artist's card: a round portrait, opening the artist's page.
pub(crate) fn artist_card(
    ui: &Ui,
    key: impl Into<String>,
    artist: &SavedArtist,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    let origin = Origin::artist(artist.id.clone()).named(artist.name.clone());
    let playing = playing_from(ui, &origin);
    let opened = Destination::Artist(artist.id.as_str().into());
    let kind = i18n::lookup("artist-eyebrow", None).into_owned();
    let card = base(
        ui,
        key,
        artist.name.clone(),
        artist.cover.as_deref(),
        fit,
        true,
        Some("icons/user.svg"),
    )
    .description(kind.clone())
    .weight(Weight::SEMIBOLD)
    .meta(kind, theme)
    .play(playing, toggle(origin))
    .on_press(move |ui: &mut Ui| ui.go(opened.clone()));
    menu(card, GenreItem::Artist(artist.clone()), on_item)
}

/// A genre's card, opening its page.
pub(crate) fn genre_card(ui: &Ui, key: impl Into<String>, genre: &Genre, fit: Fit) -> Card {
    let opened = Destination::Genre(genre.id.as_str().into());
    base(
        ui,
        key,
        genre.name.clone(),
        genre.cover.as_deref(),
        fit,
        false,
        Some("icons/music.svg"),
    )
    .weight(Weight::SEMIBOLD)
    .on_press(move |ui: &mut Ui| ui.go(opened.clone()))
}

/// The card of a track with nothing wired to play it: name, cover, explicit mark and the
/// screen's menu, tinted while it is the one playing. Whoever lists it adds the caption and the
/// play and press it wants; [`track_status`] says where playback stands.
pub(crate) fn track_card(
    ui: &Ui,
    key: impl Into<String>,
    track: &Track,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    let (current, _) = track_status(ui, track);
    let tint = match current {
        true => theme.primary,
        false => theme.foreground,
    };
    let card = base(
        ui,
        key,
        track.name.clone(),
        track.cover.as_deref(),
        fit,
        false,
        None,
    )
    .description(track_caption(None, track))
    .tint(tint)
    .hint();
    let card = match track.explicit {
        true => card.explicit(),
        false => card,
    };
    menu(card, GenreItem::Track(track.clone()), on_item)
}

/// Whether the track is the one in the player, and whether its play button shows pause.
pub(crate) fn track_status(ui: &Ui, track: &Track) -> (bool, bool) {
    let playback = &ui.store.playback;
    let current = track.id.is_some()
        && playback.track().and_then(|playing| playing.id.as_deref()) == track.id.as_deref();
    (current, current && playback.control() == Some(true))
}

/// The artists of a track as the small muted links under its name.
pub(crate) fn track_artists(key: impl Into<String>, track: &Track, theme: &Theme) -> El {
    cells::artist_links(
        key,
        &track.artist_refs,
        track.artists.clone(),
        theme.muted_foreground,
    )
    .text_size(theme.text(Text::Small))
    .truncate()
    .build(theme)
}

/// A shelf item as a card, whatever it is: an album, playlist or artist opens its page and
/// plays from its play control, a track starts its radio, a genre opens its page.
pub(crate) fn item_card(
    ui: &Ui,
    key: impl Into<String>,
    item: &GenreItem,
    fit: Fit,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    match item {
        GenreItem::Album(album) => album_card(ui, key, album, fit, on_item),
        GenreItem::Playlist(playlist) => playlist_card(ui, key, playlist, fit, on_item)
            .description(playlist_meta(playlist))
            .meta(playlist_meta(playlist), theme),
        GenreItem::Artist(artist) => artist_card(ui, key, artist, fit, on_item),
        GenreItem::Genre(genre) => genre_card(ui, key, genre, fit),
        GenreItem::Track(track) => {
            let (current, playing) = track_status(ui, track);
            let seed = track.clone();
            let restarted = track.clone();
            track_card(ui, key, track, fit, on_item)
                .weight(Weight::SEMIBOLD)
                .bare_meta(track_artists("item-card-artist", track, theme))
                .play(playing, move |ui: &mut Ui, _: &mut Tx| {
                    match current {
                        true => ui.store.toggle_play(),
                        false => ui.store.play_radio(&seed),
                    }
                    ui.settle();
                })
                .on_press(move |ui: &mut Ui| {
                    ui.store.play_radio(&restarted);
                    ui.settle();
                })
        }
    }
}

/// A shelf item as one row of a list, at the plain weight of a listed row. Tracks, releases and
/// playlists say what they are under their name, as in "Song · Artist", "EP · 2023 · Artist" or
/// "Playlist · Made for you · 50 songs".
pub(crate) fn listed(
    ui: &Ui,
    key: impl Into<String>,
    item: &GenreItem,
    on_item: Option<&OnItem>,
) -> Card {
    let theme = &ui.theme;
    let card = item_card(ui, key, item, Fit::Row, on_item).weight(Weight::NORMAL);
    match item {
        GenreItem::Track(track) => {
            let kind = t!("kind-song").into_owned();
            card.description(track_caption(Some(kind.clone()), track))
                .bare_meta(tagged(
                    kind,
                    track_artists("listed-artist", track, theme),
                    theme,
                ))
        }
        GenreItem::Album(album) => {
            let kind = i18n::lookup(release_key(album.release_type), None).into_owned();
            card.description(spoken_caption([
                kind.clone(),
                year_label(album.year, None).unwrap_or_default(),
                album.artists.clone(),
            ]))
            .bare_meta(tagged(
                kind,
                released(
                    "listed-artist",
                    album.year,
                    None,
                    &album.artist_refs,
                    album.artists.clone(),
                    theme,
                ),
                theme,
            ))
        }
        GenreItem::Playlist(playlist) => {
            let kind = t!("kind-playlist").into_owned();
            let meta = playlist_meta(playlist);
            card.description(spoken_caption([kind.clone(), meta.clone()]))
                .bare_meta(tagged(kind, small(meta, theme), theme))
        }
        _ => card,
    }
}

/// Small muted text, cut short where it runs out of room.
fn small(value: impl Into<String>, theme: &Theme) -> El {
    text(value)
        .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
        .color(theme.muted_foreground.color())
        .truncate()
        .shrink()
        .min_w(0.)
}

/// A caption led by what kind of thing it captions: "Song · " and then the rest.
fn tagged(kind: String, rest: El, theme: &Theme) -> El {
    row()
        .center_y()
        .gap(CAPTION_GAP)
        .min_w(0.)
        .child(small(format!("{kind} {BULLET}"), theme).a11y_hidden())
        .child(row().min_w(0.).shrink().child(rest))
        .a11y_hidden()
}

/// The line under a playlist on a shelf: who it is by, and how many songs when that is known,
/// so a recap reads "Made for you · 50 songs" the way its provider lists it. A library card
/// keeps the owner alone.
fn playlist_meta(playlist: &Playlist) -> String {
    let owner = playlist.owner.as_str();
    match (owner.is_empty(), playlist.track_count) {
        (_, 0) => owner.to_owned(),
        (true, count) => t!("count-songs", count = count).into_owned(),
        (false, count) => format!("{owner} {BULLET} {}", t!("count-songs", count = count)),
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

/// The line under a release's title: its year, or its kind when the year is unknown, then who
/// it is credited to, up to `CARD_LINKS` of them. The bullet between the two is left out when
/// the release credits no one. The line is clipped to the card's width, so a compilation's
/// twentieth credit could never be read anyway.
pub(crate) fn released(
    key: &str,
    year: i32,
    kind: Option<ReleaseType>,
    artists: &[ArtistRef],
    fallback: impl Into<String>,
    theme: &Theme,
) -> El {
    let year = year_label(year, kind);
    let fallback = fallback.into();
    let credited: Vec<ArtistRef> = artists
        .iter()
        .filter(|artist| !artist.name.is_empty())
        .take(CARD_LINKS)
        .cloned()
        .collect();
    let named = !credited.is_empty() || !fallback.is_empty();
    let artists = cells::artist_links(key, &credited, fallback, theme.muted_foreground)
        .text_size(theme.text(Text::Small))
        .truncate()
        .build(theme);
    let mut line = row().center_y().gap(CAPTION_GAP).min_w(0.);
    if let Some(year) = year {
        line = line.child(small(year, theme));
        if named {
            line = line.child(small(BULLET, theme));
        }
    }
    line.child(artists)
}

/// What a release's caption leads with: its year, or its kind when the year is unknown.
pub(crate) fn year_label(year: i32, kind: Option<ReleaseType>) -> Option<String> {
    match year {
        0 => kind.map(|kind| i18n::lookup(release_key(kind), None).into_owned()),
        year => Some(year.to_string()),
    }
}

/// The parts of a card's caption that are not empty, joined by bullets, for screen readers to
/// read after the card's name.
fn spoken_caption(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(&format!(" {BULLET} "))
}

/// What a screen reader hears after a track card's name: an optional kind, the artists, and
/// whether the track is explicit.
fn track_caption(kind: Option<String>, track: &Track) -> String {
    spoken_caption(
        kind.into_iter()
            .chain([track.artists.clone()])
            .chain(track.explicit.then(|| t!("song-explicit").into_owned())),
    )
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

    /// The key of the filter's pill.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::All => "release-filter-all",
            Self::Albums => "release-filter-albums",
            Self::Singles => "release-filter-singles",
            Self::Eps => "release-filter-eps",
        }
    }

    /// The pill's label, resolved for the active language.
    pub(crate) fn label(self) -> String {
        match self {
            Self::All => t!("artist-filter-all"),
            Self::Albums => t!("artist-filter-albums"),
            Self::Singles => t!("artist-filter-singles"),
            Self::Eps => t!("artist-filter-eps"),
        }
        .into_owned()
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
