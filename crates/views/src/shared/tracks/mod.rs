mod columns;
mod sieve;
mod sort;

use i18n::t;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;
use ui::{ActiveTheme as _, Choice, ChoiceAxis, Filter, FilterChange, RangeAxis, Unit};

use crate::shared::menus::{ItemMenu, TrackColumns};
use gpui::accesskit::Role;
use gpui::prelude::FluentBuilder as _;
use gpui::prelude::StatefulInteractiveElement as _;
use gpui::{
    AnyElement, App, Context, Entity, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, WeakEntity, div,
};
use music::{Descriptors, Track};
use router::Destination;
use state::{Detail, History, Library, Network, Origin, Playback, Progress, Shelf, Sonora};
use ui::{
    ArrowRing, Button, Cell, ColumnSpec, Menu, Pending, Pin, ROW_GROUP, Scrollbar, TableSource,
    TableState,
};

use crate::shared::cells;
use crate::shared::confirm::{Confirm, Kind};
use crate::shared::pins::{Pinned as _, origin_tag};

pub(crate) use columns::{
    HISTORY_COLUMNS, LIBRARY_COLUMNS, TrackField, album_columns, artist_columns, playlist_columns,
};
pub(crate) use sieve::TrackSieve;
pub(crate) use sort::initial;

use sieve::{BPM, DURATION, FACETS, RANGES};
use sort::{genres_of, hits};

/// The keys of the filters that offer a list of options to pick from, each needing a submenu of
/// its own.
pub(crate) fn choice_filters() -> impl Iterator<Item = &'static str> {
    FACETS.iter().map(|(key, _)| *key)
}

/// How strongly an unsaved track's heart shows on its row.
const SETTLED: f32 = 0.5;

/// The current track, its `Playback::control` and its queue origin. A screen keeps the last
/// status and redraws only when it changes, including a move between collections of the same song.
pub(crate) type PlaybackStatus = (Option<String>, Option<bool>, Option<Origin>);

pub(crate) fn playback_status(playback: &Entity<Playback>, cx: &App) -> PlaybackStatus {
    let playback = playback.read(cx);
    let track = playback.track().and_then(|track| track.id.clone());
    (track, playback.control(), playback.origin(cx).cloned())
}
pub(crate) trait Tracks: 'static {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track];
    fn is_loading(&self, cx: &App) -> bool;

    /// What the provider says of the tracks by id, for the filters by tempo, genre, mood and
    /// tag. A list with no such facts leaves the default, and those filters stay out.
    fn descriptors<'a>(&self, _cx: &'a App) -> Option<&'a HashMap<String, Descriptors>> {
        None
    }

    /// The skeleton rows the table draws before the first tracks arrive. A list that leaves
    /// the default fills the viewport with them while it loads.
    fn pending(&self, cx: &App) -> Option<Pending> {
        self.is_loading(cx).then_some(Pending::Screen)
    }
}

pub(crate) fn first_playable(table: &Entity<TableState<TrackSource>>, cx: &App) -> Option<usize> {
    let state = table.read(cx);
    let delegate = state.delegate();

    (0..delegate.row_count()).find(|display| {
        delegate
            .source()
            .peek(delegate.row(*display), cx)
            .is_some_and(|track| track.playable)
    })
}

pub(crate) fn holds(table: &Entity<TableState<TrackSource>>, id: &str, cx: &App) -> bool {
    let state = table.read(cx);
    let delegate = state.delegate();

    (0..delegate.row_count()).any(|display| {
        delegate
            .source()
            .peek(delegate.row(display), cx)
            .is_some_and(|track| track.id.as_deref() == Some(id))
    })
}

pub(crate) fn whence(table: &Entity<TableState<TrackSource>>, cx: &App) -> Option<Origin> {
    table.read(cx).delegate().source().whence(cx)
}

pub(crate) fn ordered(table: &Entity<TableState<TrackSource>>, cx: &App) -> Vec<Track> {
    let state = table.read(cx);
    let delegate = state.delegate();

    (0..delegate.row_count())
        .filter_map(|display| delegate.source().at(delegate.row(display), cx))
        .collect()
}

pub(crate) fn drop_picked(table: &Entity<TableState<TrackSource>>, cx: &mut App) {
    let rows = table.read(cx).delegate().picked();
    if rows.is_empty() {
        return;
    }
    let (tracks, playlist, history) = {
        let state = table.read(cx);
        let source = state.delegate().source();
        let tracks: Vec<Track> = rows.iter().filter_map(|&row| source.at(row, cx)).collect();
        (tracks, source.playlist.clone(), source.history.clone())
    };
    if tracks.is_empty() {
        return;
    }
    let kind = if playlist.is_some() {
        Kind::PlaylistSongs(tracks.len())
    } else if history.is_some() {
        Kind::History(tracks.len())
    } else {
        Kind::LibrarySongs(tracks.len())
    };
    let table = table.clone();
    Confirm::ask(
        kind,
        move |cx| {
            forget(&tracks, playlist.as_ref(), history.as_ref(), cx);
            table.update(cx, |table, cx| {
                table.delegate_mut().clear_selection();
                cx.notify();
            });
        },
        cx,
    );
}

fn forget(
    tracks: &[Track],
    playlist: Option<&Entity<Detail>>,
    history: Option<&Entity<History>>,
    cx: &mut App,
) {
    if let Some(detail) = playlist {
        let ids: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
        if !ids.is_empty() {
            detail.update(cx, |detail, cx| detail.remove_tracks_from_playlist(ids, cx));
        }
        return;
    }
    if let Some(history) = history {
        history.update(cx, |history, cx| {
            for track in tracks {
                history.remove(track, cx);
            }
        });
        return;
    }
    let library = Sonora::global(cx).library.clone();
    library.update(cx, |library, cx| {
        library.save_tracks(tracks.to_vec(), false, cx)
    });
}

type Whence = Rc<dyn Fn(&App) -> Option<Origin>>;

pub(crate) struct TrackSource {
    columns: &'static [ColumnSpec<TrackField>],
    whence: Option<Whence>,
    provider: Rc<dyn Tracks>,
    playback: Entity<Playback>,
    is_liked: Option<Entity<Library>>,
    starrable: Option<Shelf>,
    /// The library shelf the rows are, which offline mode keeps to its downloads.
    library_shelf: Option<Shelf>,
    album: Option<Entity<Detail>>,
    playlist: Option<Entity<Detail>>,
    history: Option<Entity<History>>,
    menu: ItemMenu,
    table: Option<WeakEntity<TableState<TrackSource>>>,
    sieve: TrackSieve,
    axes: RefCell<Option<(i18n::Language, Vec<Filter>)>>,
}

/// Builds a numeric filter from sorted values, with duration sliding freely between its ends.
fn range_axis(key: &'static str, mut values: Vec<f32>, unit: Unit) -> Option<Filter> {
    values.sort_by(f32::total_cmp);
    values.dedup();
    let bounds = (*values.first()?, *values.last()?);
    (bounds.1 - bounds.0 > f32::EPSILON).then(|| {
        Filter::Range(RangeAxis {
            key,
            label: i18n::lookup(key, None),
            bounds,
            value: bounds,
            unit,
            values: (key != DURATION && key != BPM).then_some(values),
        })
    })
}

/// Counts each option and offers the commonest first.
fn choices<'a>(words: impl Iterator<Item = &'a String>) -> Vec<Choice> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for word in words {
        *counts.entry(word).or_default() += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|(a, x), (b, y)| y.cmp(x).then(a.cmp(b)));
    ranked
        .into_iter()
        .map(|(id, _)| choice(id, false))
        .collect()
}

/// One option for a word, picked or not.
fn choice(id: &str, on: bool) -> Choice {
    Choice {
        id: id.to_owned().into(),
        // Moods are Fluent keys, and a provider's other words are plain.
        label: match id.starts_with("stats-mood-") {
            true => i18n::lookup(id, None),
            false => id.to_owned().into(),
        },
        on,
    }
}

/// What the number at the head of a row counts.
#[derive(Clone, Copy, PartialEq)]
enum Numbering {
    /// The row's place in the list, for anything that is not an album in order.
    Listing,
    /// The track's own number on the album.
    Track,
    /// The track's number behind its disc, for an album that spans more than one.
    Disc,
}

/// The disc a track sits on. Providers disagree on what an absent disc tag means, so a
/// single-disc album reads as disc one either way.
fn disc(track: &Track) -> u32 {
    track.disc_number.max(1)
}

impl TrackSource {
    pub(crate) fn new(
        columns: &'static [ColumnSpec<TrackField>],
        provider: impl Tracks,
        playback: Entity<Playback>,
        playlist_scrollbar: Entity<Scrollbar>,
        cx: &mut App,
    ) -> Self {
        Self {
            columns,
            whence: None,
            provider: Rc::new(provider),
            playback,
            is_liked: None,
            starrable: None,
            library_shelf: None,
            album: None,
            playlist: None,
            history: None,
            menu: ItemMenu::new(playlist_scrollbar, cx),
            table: None,
            sieve: TrackSieve::default(),
            axes: RefCell::new(None),
        }
    }

    pub(crate) fn set_columns(&mut self, columns: &'static [ColumnSpec<TrackField>]) -> bool {
        let changed = !std::ptr::eq(self.columns.as_ptr(), columns.as_ptr());
        self.columns = columns;
        changed
    }

    /// Caches the whole list's filter options until the source rebuilds, so narrowing never
    /// removes its own options. Embedded genres work even without extra provider metadata.
    fn axes(&self, cx: &App) -> Vec<Filter> {
        let language = i18n::language();
        let mut cache = self.axes.borrow_mut();
        if cache
            .as_ref()
            .is_some_and(|(cached, _)| *cached != language)
        {
            *cache = None;
        }
        cache
            .get_or_insert_with(|| {
                let tracks = self.provider.tracks(cx);
                let found = self.provider.descriptors(cx);
                let mut axes: Vec<_> = RANGES
                    .iter()
                    .filter_map(|(key, read, unit)| {
                        range_axis(
                            key,
                            tracks
                                .iter()
                                .filter_map(|track| read(track, found))
                                .collect(),
                            *unit,
                        )
                    })
                    .collect();
                axes.extend(FACETS.iter().map(|(key, words)| {
                    let options = choices(tracks.iter().flat_map(|track| words(track, found)));
                    Filter::Choice(ChoiceAxis {
                        key,
                        label: i18n::lookup(key, None),
                        options,
                    })
                }));
                (language, axes)
            })
            .1
            .clone()
    }

    // resolved when play starts
    pub(crate) fn from(mut self, whence: impl Fn(&App) -> Option<Origin> + 'static) -> Self {
        self.whence = Some(Rc::new(whence));
        self
    }

    pub(crate) fn whence(&self, cx: &App) -> Option<Origin> {
        self.whence.as_ref().and_then(|whence| whence(cx))
    }

    /// The origin tag a song dragged from this table carries, for a move. A song its playlist
    /// holds more than once stays a copy, since the provider would take out every copy and the
    /// Undo would bring back one.
    fn origin_tag(&self, track: &Track, tracks: &[Track], cx: &App) -> Option<String> {
        let origin = self.whence(cx)?;
        let once = origin.whence != state::Whence::Playlist
            || tracks.iter().filter(|other| other.id == track.id).count() == 1;
        once.then(|| origin_tag(&origin)).flatten()
    }

    pub(crate) fn table(mut self, table: WeakEntity<TableState<TrackSource>>) -> Self {
        self.table = Some(table);
        self
    }

    pub(crate) fn with_liked(mut self, library: Entity<Library>) -> Self {
        self.is_liked = Some(library);
        self
    }

    /// Offers a favorites filter, and hearts on the rows, only when the shelf lists more than the
    /// favorites. Needs `with_liked`, which supplies the library both ask.
    pub(crate) fn starrable(mut self, shelf: Shelf) -> Self {
        self.starrable = Some(shelf);
        self
    }

    /// Marks the rows as a library shelf, which offline mode keeps to what was downloaded.
    pub(crate) fn shelved(mut self, shelf: Shelf) -> Self {
        self.library_shelf = Some(shelf);
        self
    }

    /// Whether offline mode keeps these rows to their downloads.
    fn downloads_only(&self, cx: &App) -> bool {
        self.library_shelf
            .is_some_and(|shelf| Network::downloads_only(shelf, cx))
    }

    fn catalog(&self, cx: &App) -> bool {
        match (self.starrable, &self.is_liked) {
            (Some(shelf), Some(library)) => library.read(cx).shape(shelf).is_catalog(),
            _ => false,
        }
    }

    /// How the rows of this table are numbered. A catalog shelf holds only the files it was
    /// given, so an album there is numbered by each track's own place in it and a missing file
    /// leaves a gap.
    fn numbering(&self, cx: &App) -> Numbering {
        let Some(id) = self.album.as_ref().and_then(|album| album.read(cx).id()) else {
            return Numbering::Listing;
        };
        if !Sonora::global(cx)
            .library
            .read(cx)
            .shape(Shelf::of(id))
            .is_catalog()
        {
            return Numbering::Listing;
        }

        let tracks = self.provider.tracks(cx);
        let first = tracks.first().map(disc);
        match tracks.iter().any(|track| Some(disc(track)) != first) {
            true => Numbering::Disc,
            false => Numbering::Track,
        }
    }

    /// The label a row rests at, or `None` to let the cell count the rows.
    fn number(&self, track: &Track, cx: &App) -> Option<SharedString> {
        if track.track_number == 0 {
            return None;
        }

        match self.numbering(cx) {
            Numbering::Listing => None,
            Numbering::Track => Some(track.track_number.to_string().into()),
            Numbering::Disc => Some(format!("{}.{}", disc(track), track.track_number).into()),
        }
    }

    fn starred(&self, track: &Track, cx: &App) -> bool {
        match (&self.is_liked, track.id.as_deref()) {
            (Some(library), Some(id)) => library.read(cx).saved(id),
            _ => false,
        }
    }

    pub(crate) fn with_playlist(mut self, detail: Entity<Detail>) -> Self {
        self.playlist = Some(detail);
        self
    }

    pub(crate) fn with_album(mut self, detail: Entity<Detail>) -> Self {
        self.album = Some(detail);
        self
    }

    pub(crate) fn with_history(mut self, history: Entity<History>) -> Self {
        self.history = Some(history);
        self
    }

    fn artist_cell(&self, cell: &Cell<TrackField>, track: &Track, color: Hsla) -> AnyElement {
        cells::artists(
            cell,
            track.artist_refs.clone(),
            track.artists.clone(),
            color,
        )
    }

    fn album_cell(&self, cell: &Cell<TrackField>, track: &Track, color: Hsla) -> AnyElement {
        let Some(album) = track.album_id.clone() else {
            return cells::dim(cell, track.album.clone(), color);
        };

        cells::link(
            cell,
            "album",
            track.album.clone(),
            color,
            Destination::Album(album.into()),
        )
    }

    /// Whether a row can start playback now, which a track that has to be streamed cannot while
    /// the network is gone.
    fn plays_now(&self, track: &Track, cx: &App) -> bool {
        track.playable
            && self
                .playback
                .read(cx)
                .unreachable_reason(track, cx)
                .is_none()
    }

    fn index_cell(&self, cell: &Cell<TrackField>, track: &Track, cx: &App) -> AnyElement {
        let playing = self.now_playing(cell.row, cx);
        let playable = self.plays_now(track, cx);
        let (preload, press) = match playable {
            false => (None, None),
            true => {
                let playback = self.playback.clone();
                let source = self.provider.clone();
                let at = cell.row;
                let preload: Option<cells::Tap> = Some(Box::new(move |cx| {
                    let Some(track) = source.tracks(cx).get(at).cloned() else {
                        return;
                    };
                    playback.update(cx, |playback, _| playback.preload(&track));
                }));
                let provider = self.provider.clone();
                let table = self.table.clone();
                let whence = self.whence.clone();
                let row = cell.row;
                let display = cell.display;
                let press = cells::toggle(&self.playback, playing, move |playback, cx| {
                    let from = whence.as_ref().and_then(|whence| whence(cx));
                    match table.as_ref().and_then(|table| table.upgrade()) {
                        Some(table) => playback.start(ordered(&table, cx), display, from, cx),
                        None => playback.start(provider.tracks(cx).to_vec(), row, from, cx),
                    }
                });
                (preload, press)
            }
        };

        let number = self.number(track, cx);

        cells::index(cell, playing, playable, number, preload, press, cx)
    }

    fn title_cell(
        &self,
        cell: &Cell<TrackField>,
        track: &Track,
        color: Option<Hsla>,
        cx: &App,
    ) -> AnyElement {
        cells::title(
            cell,
            track.name.clone(),
            color,
            track.explicit.then(|| t!("song-explicit")),
            None,
            download_mark(cell, track, cx),
            self.liked_button(cell, track, cx),
        )
    }

    fn liked_button(&self, cell: &Cell<TrackField>, track: &Track, cx: &App) -> Option<AnyElement> {
        let library = self.is_liked.as_ref()?;
        // on a saved shelf every listed track is a favorite, so the heart would say nothing
        if self.starrable.is_some() && !self.catalog(cx) {
            return None;
        }
        let id = track.id.clone()?;
        let theme = *cx.theme();
        let state = library.read(cx);
        let saved = state.saved(&id);
        let pending = state.pending(&id);
        let library = library.clone();
        let source = self.provider.clone();
        let at = cell.row;

        Some(
            Button::new(("toggle-liked-track", cell.row))
                .ghost()
                .backgroundless()
                .small()
                .icon(match saved {
                    true => "icons/heart-filled.svg",
                    false => "icons/heart.svg",
                })
                .tooltip(match saved {
                    true => "menu-remove-from-library",
                    false => "menu-add-to-library",
                })
                .tint(match saved {
                    true => theme.primary,
                    false => theme.muted_foreground.opacity(SETTLED),
                })
                .when(!saved, |this| {
                    this.invisible()
                        .group_hover(ROW_GROUP, |style| style.visible())
                })
                .disabled(pending)
                .on_click(move |_, _, cx| {
                    let Some(track) = source.tracks(cx).get(at).cloned() else {
                        return;
                    };
                    library.update(cx, |library, cx| library.toggle(track, cx));
                })
                .into_any_element(),
        )
    }

    /// The row's `Playback::control` when it holds the current track. A track listed twice
    /// counts only at its first row.
    pub(crate) fn now_playing(&self, row: usize, cx: &App) -> Option<bool> {
        let playback = self.playback.read(cx);
        let current = playback.track()?.id.as_deref()?;
        let tracks = self.provider.tracks(cx);
        if tracks.get(row)?.id.as_deref() != Some(current) {
            return None;
        }
        let sole = tracks
            .iter()
            .position(|track| track.id.as_deref() == Some(current))?;

        (sole == row).then(|| playback.control()).flatten()
    }

    pub(crate) fn at(&self, row: usize, cx: &App) -> Option<Track> {
        self.provider.tracks(cx).get(row).cloned()
    }

    pub(crate) fn peek<'a>(&self, row: usize, cx: &'a App) -> Option<&'a Track> {
        self.provider.tracks(cx).get(row)
    }

    pub(crate) fn menu(&self) -> &ItemMenu {
        &self.menu
    }
}

impl TableSource for TrackSource {
    type Field = TrackField;

    fn invalidate(&mut self) {
        *self.axes.get_mut() = None;
    }

    fn columns(&self) -> &'static [ColumnSpec<TrackField>] {
        self.columns
    }

    fn rows(&self, cx: &App) -> usize {
        self.provider.tracks(cx).len()
    }

    fn populated(&self, field: TrackField, cx: &App) -> bool {
        let tracks = self.provider.tracks(cx);
        if tracks.is_empty() {
            return true;
        }

        match field {
            TrackField::Artists => tracks.iter().any(|track| !track.artists.is_empty()),
            TrackField::Album => tracks.iter().any(|track| !track.album.is_empty()),
            TrackField::Genre => {
                let found = self.provider.descriptors(cx);
                tracks
                    .iter()
                    .any(|track| !genres_of(track, found).is_empty())
            }
            TrackField::AddedAt | TrackField::PlayedAt => {
                tracks.iter().any(|track| track.added_at.is_some())
            }
            TrackField::AddedBy => tracks.iter().any(|track| track.added_by.is_some()),
            TrackField::Plays => tracks.iter().any(|track| track.playcount.is_some()),
            _ => true,
        }
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.peek(row, cx).is_some_and(|track| {
            if !self.sieve.keeps(track, self.provider.descriptors(cx)) {
                return false;
            }
            if self.sieve.favorites && !self.starred(track, cx) {
                return false;
            }
            if (self.sieve.downloaded || self.downloads_only(cx))
                && !track
                    .id
                    .as_deref()
                    .is_some_and(|id| state::downloaded(id, cx))
            {
                return false;
            }
            hits(track, query)
        })
    }

    fn filter_axes(&self, cx: &App) -> Vec<Filter> {
        if self.provider.tracks(cx).len() <= 1 && !self.sieve.active() {
            return Vec::new();
        }
        let mut axes = self.axes(cx);
        for axis in &mut axes {
            match axis {
                Filter::Range(axis) => {
                    axis.value = self
                        .sieve
                        .ranges
                        .get(axis.key)
                        .copied()
                        .unwrap_or(axis.bounds);
                    *axis = axis.clone().clamped();
                }
                Filter::Choice(axis) => {
                    let picked = self.sieve.picks.get(axis.key);
                    for option in &mut axis.options {
                        option.on =
                            picked.is_some_and(|picked| picked.contains(option.id.as_ref()));
                    }
                    // Keep stale picks visible so they can still be cleared.
                    for id in picked.into_iter().flatten() {
                        if !axis.options.iter().any(|option| option.id == id.as_str()) {
                            axis.options.push(choice(id, true));
                        }
                    }
                }
                Filter::Flag(_) => {}
            }
        }
        axes.retain(|axis| !matches!(axis, Filter::Choice(axis) if axis.options.is_empty()));
        // Offline mode already keeps a shelf to its downloads, so that filter would do nothing.
        let offered = self.sieve.downloaded || !Sonora::global(cx).downloads.read(cx).is_empty();
        axes.extend(
            self.sieve
                .flags(self.catalog(cx), offered && !self.downloads_only(cx)),
        );
        axes
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        match change {
            FilterChange::Range(key, value) => {
                let known = RANGES.iter().any(|(known, ..)| *known == key);
                if !known {
                    return false;
                }
                match value {
                    Some(value) => self.sieve.ranges.insert(key, value),
                    None => self.sieve.ranges.remove(key),
                };
                true
            }
            FilterChange::Choice(key, id, on) => {
                self.sieve.pick(key, &id, on);
                true
            }
            FilterChange::Flag(key, value) => {
                match key {
                    "filter-hide-explicit" => self.sieve.hide_explicit = value,
                    "filter-playable" => self.sieve.playable = value,
                    "filter-collaborations" => self.sieve.collaborations = value,
                    "filter-favorites" => self.sieve.favorites = value,
                    "filter-downloaded" => self.sieve.downloaded = value,
                    _ => return false,
                }
                true
            }
            FilterChange::Reset => {
                self.sieve = Default::default();
                true
            }
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.sieve.active()
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.now_playing(row, cx).is_some()
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.provider.is_loading(cx)
    }

    /// Why a track that has to be streamed cannot play while the network is gone, in the words
    /// of the toast picking it would raise.
    fn tip(&self, row: usize, cx: &App) -> Option<SharedString> {
        let track = self.provider.tracks(cx).get(row)?;
        let reason = self.playback.read(cx).unreachable_reason(track, cx)?;
        let mut args = i18n::FluentArgs::new();
        if let Some(provider) = reason.provider() {
            args.set("name", i18n::Value::value(provider.as_ref()));
        }
        Some(i18n::lookup(reason.key(), Some(&args)))
    }

    fn pending(&self, cx: &App) -> Option<Pending> {
        self.provider.pending(cx)
    }

    /// The track's title and artists, which is what a screen reader announces for its row.
    fn name(&self, row: usize, cx: &App) -> Option<SharedString> {
        let track = self.provider.tracks(cx).get(row)?;
        Some(match track.artists.is_empty() {
            true => SharedString::from(track.name.clone()),
            false => t!(
                "track-row-name",
                title = track.name.as_str(),
                artists = track.artists.as_str()
            ),
        })
    }

    fn row_key(&self, row: usize, cx: &App) -> Option<SharedString> {
        let id = self.provider.tracks(cx).get(row)?.id.clone()?;
        Some(id.into())
    }

    fn pin(&self, row: usize, cx: &App) -> Option<Pin> {
        let tracks = self.provider.tracks(cx);
        let track = tracks.get(row)?;
        Some(track.pin()?.origin(self.origin_tag(track, tracks, cx)))
    }

    fn cell(&self, cell: Cell<TrackField>, cx: &mut App) -> AnyElement {
        let muted = cx.theme().muted_foreground;

        let Some(track) = self.provider.tracks(cx).get(cell.row) else {
            return cells::blank(&cell);
        };

        if cell.field == TrackField::Index {
            return self.index_cell(&cell, track, cx);
        }
        let faded = muted.opacity(0.5);
        let (title, detail) = match self.plays_now(track, cx) {
            true => (None, muted),
            false => (Some(faded), faded),
        };

        match cell.field {
            TrackField::Cover => cells::artwork(&cell, track.cover.clone()),
            TrackField::Title => self.title_cell(&cell, track, title, cx),
            TrackField::Artists => self.artist_cell(&cell, track, detail),
            TrackField::Album => self.album_cell(&cell, track, detail),
            TrackField::Genre => cells::dim(
                &cell,
                genres_of(track, self.provider.descriptors(cx)).join(", "),
                detail,
            ),
            TrackField::AddedAt => cells::credited(
                &cell,
                track.added_by.as_deref(),
                cells::stamp(track.added_at),
                detail,
            ),
            TrackField::AddedBy => match track.added_by.as_deref() {
                Some(added) => cells::added_by(&cell, added, detail),
                None => cells::blank(&cell),
            },
            TrackField::PlayedAt => {
                cells::dim(&cell, cells::relative_stamp(track.added_at), detail)
            }
            TrackField::Plays => cells::dim(
                &cell,
                track.playcount.map(cells::count).unwrap_or_default(),
                detail,
            ),
            TrackField::Duration => cells::length(&cell, track.duration, detail),
            TrackField::Index => cells::blank(&cell),
        }
    }

    fn picking(&self) -> bool {
        true
    }

    fn context_menu(&self, rows: &[usize], visible: &[TrackField], cx: &App) -> Option<Menu> {
        let tracks: Vec<Track> = rows.iter().filter_map(|&row| self.at(row, cx)).collect();
        if tracks.is_empty() {
            return None;
        }
        let columns = match Sonora::global(cx).settings.read(cx).adaptive_menu() {
            true => TrackColumns {
                album: visible.contains(&TrackField::Album),
                artists: visible.contains(&TrackField::Artists),
            },
            false => TrackColumns::default(),
        };
        if let Some(history) = &self.history {
            return Some(
                self.menu
                    .for_history_tracks(&tracks, history.clone(), columns, cx),
            );
        }
        Some(match (&self.album, &self.playlist) {
            (Some(detail), _) => {
                let id = detail.read(cx).id()?;
                self.menu.for_album_tracks(&tracks, id, columns, cx)
            }
            (_, Some(detail)) => {
                self.menu
                    .for_playlist_tracks(&tracks, detail.clone(), columns, cx)
            }
            (None, None) => self.menu.for_table_tracks(&tracks, columns, cx),
        })
    }

    fn context_menu_will_open(&self, _rows: &[usize], cx: &App) {
        self.menu.reset(cx);
    }

    fn compare(&self, field: TrackField, a: usize, b: usize, cx: &App) -> Ordering {
        sort::compare(
            self.provider.tracks(cx),
            self.provider.descriptors(cx),
            field,
            a,
            b,
        )
    }

    fn group(&self, field: TrackField, row: usize, cx: &App) -> Option<SharedString> {
        sort::group(
            self.provider.tracks(cx),
            self.provider.descriptors(cx),
            field,
            row,
        )
    }
}

/// How far a download the listener asked for has got, shown before the heart. The arrow sits on
/// a filled disc like the explicit badge, the accent ring fades once the download lands, and a
/// track still waiting its turn or only cached because it played shows nothing.
fn download_mark(cell: &Cell<TrackField>, track: &Track, cx: &App) -> Option<AnyElement> {
    let id = track.id.as_deref()?;
    let downloads = Sonora::global(cx).downloads.read(cx);
    let theme = cx.theme();
    let mark = ("download-mark", cell.row);
    let ring = |done| {
        ArrowRing::new(mark, done)
            .color(theme.primary)
            .arrow(theme.muted_foreground)
            .backdrop(theme.muted)
    };
    let (ring, landed) = match downloads.track(id)? {
        Progress::Done => (ring(Some(1.)), true),
        Progress::Running => (ring(downloads.fraction(id)), false),
        Progress::Waiting => return None,
        Progress::Failed => (ring(Some(0.)).arrow(theme.danger), false),
    };
    // The id stays the same in every state, since the ring keeps its animation under it and a
    // new id on landing would skip the fade. Only a landed download keeps a name, since no
    // catalogued string says what a running or failed one is doing here.
    let ring = div()
        .id(("downloaded-mark", cell.row))
        .flex_none()
        .child(ring);
    Some(match landed {
        true => ring
            .role(Role::Image)
            .aria_label(t!("filter-downloaded"))
            .into_any_element(),
        false => ring.into_any_element(),
    })
}

/// Redraws a track table whenever downloads move, so the marks on its rows keep up.
pub(crate) fn follow_downloads<T: 'static>(cx: &mut Context<T>) {
    let downloads = Sonora::global(cx).downloads.clone();
    cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
}

#[cfg(test)]
mod fixture {
    use std::time::Duration;

    use music::Track;

    pub(super) fn track(seconds: u64, explicit: bool, playable: bool) -> Track {
        Track {
            id: Some("id".to_owned()),
            playable,
            duration: Duration::from_secs(seconds),
            explicit,
            ..Default::default()
        }
    }
}
