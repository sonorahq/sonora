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
use music::{Arrival, Track};
use router::Destination;
use state::{
    Detail, History, Library, LibraryPart, Network, Origin, Playback, Progress, Reorder, Shelf,
    Sonora,
};
use ui::{
    ArrowRing, Button, Cell, ColumnSpec, Menu, Pending, Pin, ROW_GROUP, RowMoves, Scrollbar,
    TableSource, TableState,
};

use crate::shared::blend::{Joined, Reach};
use crate::shared::cells;
use crate::shared::pins::{Pinned as _, origin_tag};

pub(crate) use columns::{
    ALBUM_COLUMNS, ARTIST_COLUMNS, HISTORY_COLUMNS, LIBRARY_COLUMNS, TrackField, playlist_columns,
};
pub(crate) use sieve::TrackSieve;
pub(crate) use sort::initial;

use sieve::{BPM, DURATION, FACETS, RANGES, filter_label};
use sort::hits;

/// Choice filter keys used to initialize their submenus.
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
    let (track, playing) = track_playback(playback, cx);
    (track, playing, playback.shown_origin(cx))
}

fn track_playback(playback: &Playback, cx: &App) -> (Option<String>, Option<bool>) {
    if let Some(remote) = playback.steered(cx) {
        return (
            remote.state.track.as_ref().map(|track| track.id.clone()),
            remote.state.track.as_ref().map(|_| remote.state.playing),
        );
    }
    let track = playback.track().and_then(|track| track.id.clone());
    (track, playback.control())
}
pub(crate) trait Tracks: 'static {
    fn tracks<'a>(&self, cx: &'a App) -> Joined<'a, Track>;
    fn is_loading(&self, cx: &App) -> bool;

    /// The skeleton rows the table draws before the first tracks arrive. A list that leaves
    /// the default fills the viewport with them while it loads.
    fn pending(&self, cx: &App) -> Option<Pending> {
        self.is_loading(cx).then_some(Pending::Screen)
    }

    /// The order the rows were added in, for a list with no date on each row. None for a list
    /// that has dates or comes in some other order.
    fn arrival(&self, _cx: &App) -> Option<Arrival> {
        None
    }

    /// When a row was added, as the Date added column shows and sorts it. A list may leave out
    /// a date the track carries when its neighbours from the same provider have none.
    fn added_at(&self, track: &Track, _cx: &App) -> Option<i64> {
        track.added_at
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
    drop_picked_if(table, |_| true, cx);
}

/// Drops the picked rows that `keep` holds to, and leaves the rest of the pick alone. It runs
/// without asking, since every removal it makes is reported in a toast with an Undo.
pub(crate) fn drop_picked_if(
    table: &Entity<TableState<TrackSource>>,
    keep: impl Fn(&Track) -> bool,
    cx: &mut App,
) {
    let rows = table.read(cx).delegate().picked();
    if rows.is_empty() {
        return;
    }
    let (tracks, playlist, history) = {
        let state = table.read(cx);
        let source = state.delegate().source();
        let tracks: Vec<Track> = rows
            .iter()
            .filter_map(|&row| source.at(row, cx))
            .filter(|track| keep(track))
            .collect();
        (tracks, source.playlist.clone(), source.history.clone())
    };
    if tracks.is_empty() {
        return;
    }
    forget(&tracks, playlist.as_ref(), history.as_ref(), cx);
    table.update(cx, |table, cx| {
        table.delegate_mut().clear_selection();
        cx.notify();
    });
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
    /// Whether the rows are the library's own, both shelves of it. Offline mode keeps the
    /// streaming ones to their downloads, and only a catalog shelf's rows carry hearts.
    shelved: bool,
    /// The shelf the Local only or Streaming only filter keeps the library rows to.
    reach: Reach,
    album: Option<Entity<Detail>>,
    playlist: Option<Entity<Detail>>,
    history: Option<Entity<History>>,
    menu: ItemMenu,
    table: Option<WeakEntity<TableState<TrackSource>>>,
    sieve: TrackSieve,
    /// Retains active slider bounds when their source values disappear.
    active_ranges: HashMap<&'static str, RangeAxis>,
    axes: RefCell<Option<(i18n::Language, Vec<Filter>)>>,
}

/// Builds a range axis, collecting stops only for snapped sliders.
fn range_axis(
    key: &'static str,
    mut values: impl Iterator<Item = f32>,
    unit: Unit,
) -> Option<Filter> {
    let first = values.next()?;
    let mut stops = (key != DURATION && key != BPM).then(|| vec![first]);
    let bounds = values.fold((first, first), |(low, high), value| {
        if let Some(stops) = &mut stops {
            stops.push(value);
        }
        (low.min(value), high.max(value))
    });
    if let Some(stops) = &mut stops {
        stops.sort_by(f32::total_cmp);
        stops.dedup();
    }
    (bounds.1 - bounds.0 > f32::EPSILON).then(|| {
        Filter::Range(RangeAxis {
            key,
            label: filter_label(key),
            bounds,
            value: bounds,
            unit,
            values: stops.map(Into::into),
        })
    })
}

/// Ranks options by frequency, then by id.
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

/// Builds an option, translating inferred mood labels.
fn choice(id: &str, on: bool) -> Choice {
    let id: SharedString = id.to_owned().into();
    let label = sort::mood_label(&id);
    Choice { id, label, on }
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
            shelved: false,
            reach: Reach::default(),
            album: None,
            playlist: None,
            history: None,
            menu: ItemMenu::new(playlist_scrollbar, cx),
            table: None,
            sieve: TrackSieve::default(),
            active_ranges: HashMap::new(),
            axes: RefCell::new(None),
        }
    }

    pub(crate) fn set_columns(&mut self, columns: &'static [ColumnSpec<TrackField>]) -> bool {
        let changed = !std::ptr::eq(self.columns.as_ptr(), columns.as_ptr());
        self.columns = columns;
        changed
    }

    /// Caches unfiltered options until the source changes. Filtering must not hide its own controls.
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
                let mut axes: Vec<_> = RANGES
                    .iter()
                    .filter_map(|(key, read, unit)| {
                        range_axis(key, tracks.iter().filter_map(read), *unit)
                    })
                    .collect();
                axes.extend(FACETS.iter().map(|(key, words)| {
                    Filter::Choice(ChoiceAxis {
                        key,
                        label: filter_label(key),
                        options: Rc::new(choices(tracks.iter().flat_map(words))),
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
    fn origin_tag(&self, track: &Track, tracks: Joined<'_, Track>, cx: &App) -> Option<String> {
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

    /// Marks the rows as the library's own. Offline mode keeps the streaming ones to what was
    /// downloaded, a shelf listing more than its favorites gets hearts and a favorites filter,
    /// and the Local only and Streaming only filters are offered. Needs `with_liked`, which
    /// supplies the library these ask.
    pub(crate) fn shelved(mut self) -> Self {
        self.shelved = true;
        self
    }

    /// Whether the table badges its local tracks, which only a library page that mixes both
    /// shelves does, see `Reach::marks_local`.
    pub(crate) fn marks_local(&self, cx: &App) -> bool {
        self.shelved && self.reach.marks_local(self.provider.tracks(cx).mixed())
    }

    /// Whether offline mode keeps a library row of `shelf` to its downloads.
    fn downloads_only(&self, shelf: Shelf, cx: &App) -> bool {
        self.shelved && Network::downloads_only(shelf, cx)
    }

    /// Whether offline mode keeps every library row to its downloads. The local files are never
    /// kept that way, so this holds only while there are none.
    fn all_downloads_only(&self, cx: &App) -> bool {
        self.downloads_only(Shelf::Streaming, cx)
            && self
                .is_liked
                .as_ref()
                .is_some_and(|library| library.read(cx).state(Shelf::Local).tracks().is_empty())
    }

    /// Whether a library row of `shelf` sits among more than the favorites, so its heart says
    /// something.
    fn catalog(&self, shelf: Shelf, cx: &App) -> bool {
        match (self.shelved, &self.is_liked) {
            (true, Some(library)) => library.read(cx).shape(shelf).is_catalog(),
            _ => false,
        }
    }

    /// Whether any shelf with songs listed here is a catalog, so a favorites filter has
    /// something to drop.
    fn starrable(&self, cx: &App) -> bool {
        let Some(library) = self.is_liked.as_ref().filter(|_| self.shelved) else {
            return false;
        };
        let library = library.read(cx);
        [Shelf::Streaming, Shelf::Local].into_iter().any(|shelf| {
            library.shape(shelf).is_catalog() && !library.state(shelf).tracks().is_empty()
        })
    }

    /// How the rows of this table are numbered. An album that spans more than one disc counts
    /// each track behind its disc on every shelf. A single-disc album on a catalog shelf is
    /// numbered by each track's own place, since the shelf holds only the files it was given and a
    /// missing one leaves a gap.
    fn numbering(&self, cx: &App) -> Numbering {
        let Some(id) = self.album.as_ref().and_then(|album| album.read(cx).id()) else {
            return Numbering::Listing;
        };
        let tracks = self.provider.tracks(cx);
        let first = tracks.first().map(disc);
        if tracks.iter().any(|track| Some(disc(track)) != first) {
            return Numbering::Disc;
        }

        let catalog = Sonora::global(cx)
            .library
            .read(cx)
            .shape(Shelf::of(id))
            .is_catalog();
        match catalog {
            true => Numbering::Track,
            false => Numbering::Listing,
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

    /// The day the listener favorited a track, when a catalog shelf knows it.
    fn liked_at(&self, track: &Track, cx: &App) -> Option<i64> {
        let (Some(library), Some(id)) = (&self.is_liked, track.id.as_deref()) else {
            return None;
        };
        library.read(cx).liked_at(LibraryPart::Tracks, id)
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
        let playable = self.plays_now(track, cx)
            && !self
                .playlist
                .as_ref()
                .is_some_and(|detail| detail.read(cx).previewing_playlist());
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
            match self.marks_local(cx)
                && track.id.as_deref().is_some_and(|id| Shelf::of(id).local())
            {
                true => Some(cells::local_badge(cell.row)),
                false => download_mark(cell, track, cx),
            },
            self.liked_button(cell, track, cx),
        )
    }

    fn liked_button(&self, cell: &Cell<TrackField>, track: &Track, cx: &App) -> Option<AnyElement> {
        let library = self.is_liked.as_ref()?;
        let id = track.id.clone()?;
        // on a saved shelf every listed track is a favorite, so the heart would say nothing
        if self.shelved && !self.catalog(Shelf::of(&id), cx) {
            return None;
        }
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

    /// Whether the current track is playing locally or remotely, for its first row only.
    pub(crate) fn now_playing(&self, row: usize, cx: &App) -> Option<bool> {
        let (current, playing) = track_playback(self.playback.read(cx), cx);
        let current = current.as_deref()?;
        let tracks = self.provider.tracks(cx);
        if tracks.get(row)?.id.as_deref() != Some(current) {
            return None;
        }
        let sole = tracks
            .iter()
            .position(|track| track.id.as_deref() == Some(current))?;

        playing.filter(|_| sole == row)
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

    /// A list that comes in the order it was added shows no Date added column, even once a
    /// song added here has been stamped with today's date.
    fn populated(&self, field: TrackField, cx: &App) -> bool {
        if field == TrackField::AddedAt && self.provider.arrival(cx).is_some() {
            return false;
        }
        if field == TrackField::LikedAt {
            return self
                .is_liked
                .as_ref()
                .is_some_and(|library| library.read(cx).dates_likes(LibraryPart::Tracks));
        }
        let tracks = self.provider.tracks(cx);
        if tracks.is_empty() {
            return true;
        }

        match field {
            TrackField::Artists => tracks.iter().any(|track| !track.artists.is_empty()),
            TrackField::Album => tracks.iter().any(|track| !track.album.is_empty()),
            TrackField::Genre | TrackField::Mood | TrackField::Tags => tracks
                .iter()
                .any(|track| !sort::words(track, field).is_empty()),
            TrackField::Bpm => tracks.iter().any(|track| track.bpm.is_some()),
            TrackField::Year => tracks.iter().any(|track| track.release_date.is_some()),
            TrackField::Popularity => tracks.iter().any(|track| track.popularity > 0),
            TrackField::AddedAt => tracks
                .iter()
                .any(|track| self.provider.added_at(track, cx).is_some()),
            TrackField::PlayedAt => tracks.iter().any(|track| track.added_at.is_some()),
            TrackField::AddedBy => tracks.iter().any(|track| track.added_by.is_some()),
            TrackField::Plays => tracks.iter().any(|track| track.playcount.is_some()),
            _ => true,
        }
    }

    /// A list that comes in the order it was added sorts by Date added through that order.
    fn sorts_by(&self, field: TrackField, cx: &App) -> bool {
        match field {
            TrackField::AddedAt if self.provider.arrival(cx).is_some() => true,
            _ => self.populated(field, cx),
        }
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.peek(row, cx).is_some_and(|track| {
            if !self.sieve.keeps(track) {
                return false;
            }
            if self.sieve.favorites && !self.starred(track, cx) {
                return false;
            }
            let id = track.id.as_deref();
            let shelf = id.map_or(Shelf::Streaming, Shelf::of);
            if (self.sieve.downloaded || self.downloads_only(shelf, cx))
                && !id.is_some_and(|id| state::downloaded(id, cx))
            {
                return false;
            }
            if self.reach.active() && !id.is_some_and(|id| self.reach.keeps(id)) {
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
        for saved in self.active_ranges.values() {
            let mut kept = saved.clone();
            kept.label = filter_label(saved.key);
            if let Some(Filter::Range(current)) = axes
                .iter_mut()
                .find(|axis| matches!(axis, Filter::Range(axis) if axis.key == saved.key))
            {
                // Widen for new data without losing the selected range's original ends.
                kept.bounds = (
                    kept.bounds.0.min(current.bounds.0),
                    kept.bounds.1.max(current.bounds.1),
                );
                if let (Some(values), Some(fresh)) = (&kept.values, &current.values)
                    && values != fresh
                {
                    let mut merged = values.to_vec();
                    merged.extend(fresh.iter().copied());
                    merged.sort_by(f32::total_cmp);
                    merged.dedup();
                    kept.values = Some(merged.into());
                }
                *current = kept;
            } else {
                axes.push(Filter::Range(kept));
            }
        }
        axes.sort_by_key(|axis| match axis {
            Filter::Range(axis) => RANGES.iter().position(|(key, _, _)| *key == axis.key),
            _ => Some(RANGES.len()),
        });
        for axis in &mut axes {
            match axis {
                Filter::Range(axis) => {
                    let (low, high) = self
                        .sieve
                        .ranges
                        .get(axis.key)
                        .copied()
                        .unwrap_or(axis.bounds);
                    axis.value = (
                        low.clamp(axis.bounds.0, axis.bounds.1),
                        high.clamp(axis.bounds.0, axis.bounds.1),
                    );
                }
                Filter::Choice(axis) => {
                    // The cached options start with nothing picked and are shared, so an axis
                    // nobody picked from stays untouched instead of being copied every frame.
                    if let Some(picked) = self.sieve.picks.get(axis.key) {
                        let options = Rc::make_mut(&mut axis.options);
                        for option in options.iter_mut() {
                            option.on = picked.contains(option.id.as_ref());
                        }
                        // Keep stale picks visible so they can still be cleared.
                        for id in picked {
                            if !options.iter().any(|option| option.id == id.as_str()) {
                                options.push(choice(id, true));
                            }
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
                .flags(self.starrable(cx), offered && !self.all_downloads_only(cx)),
        );
        if self.shelved {
            axes.extend(self.reach.axes(self.provider.tracks(cx).mixed()));
        }
        axes
    }

    fn filter(&mut self, change: FilterChange, cx: &App) -> bool {
        if self.reach.filter(&change) {
            return true;
        }
        match change {
            FilterChange::Range(key, value) => {
                if !RANGES.iter().any(|(known, ..)| *known == key)
                    || self.sieve.ranges.get(key).copied() == value
                {
                    return false;
                }
                match value {
                    Some(value) => {
                        // The axis as it first narrowed keeps its bounds if the values behind
                        // them disappear. Later steps of the same drag leave it alone.
                        if !self.active_ranges.contains_key(key) {
                            let saved =
                                self.filter_axes(cx)
                                    .into_iter()
                                    .find_map(|axis| match axis {
                                        Filter::Range(axis) if axis.key == key => Some(axis),
                                        _ => None,
                                    });
                            self.active_ranges.extend(saved.map(|axis| (key, axis)));
                        }
                        self.sieve.ranges.insert(key, value);
                    }
                    None => {
                        self.active_ranges.remove(key);
                        self.sieve.ranges.remove(key);
                    }
                }
            }
            FilterChange::Choice(key, id, on) => {
                self.sieve.pick(key, &id, on);
            }
            FilterChange::Flag(key, value) => match key {
                "filter-hide-explicit" => self.sieve.hide_explicit = value,
                "filter-collaborations" => self.sieve.collaborations = value,
                "filter-favorites" => self.sieve.favorites = value,
                "filter-downloaded" => self.sieve.downloaded = value,
                _ => return false,
            },
            FilterChange::Reset => {
                self.sieve = Default::default();
                self.active_ranges.clear();
                self.reach = Reach::default();
            }
        }
        true
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.sieve.active() || self.reach.active()
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
            TrackField::Genre | TrackField::Tags => {
                cells::dim(&cell, sort::words(track, cell.field).join(", "), detail)
            }
            TrackField::Mood => cells::dim(
                &cell,
                track
                    .moods
                    .iter()
                    .map(|mood| sort::mood_label(mood).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                detail,
            ),
            TrackField::Bpm => cells::dim(
                &cell,
                track.bpm.map(|bpm| format!("{bpm:.1}")).unwrap_or_default(),
                detail,
            ),
            TrackField::Year => cells::dim(
                &cell,
                track
                    .release_date
                    .map(|date| date.year().to_string())
                    .unwrap_or_default(),
                detail,
            ),
            TrackField::Popularity => cells::dim(
                &cell,
                match track.popularity {
                    0 => String::new(),
                    popularity => popularity.to_string(),
                },
                detail,
            ),
            TrackField::AddedAt => cells::credited(
                &cell,
                track.added_by.as_deref(),
                cells::stamp(self.provider.added_at(track, cx)),
                detail,
            ),
            TrackField::LikedAt => {
                cells::dim(&cell, cells::stamp(self.liked_at(track, cx)), detail)
            }
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

    /// A playlist's rows move when its provider keeps the order, and are refused with a
    /// message when the playlist is the listener's but the provider keeps none.
    fn row_moves(&self, cx: &App) -> RowMoves {
        let Some(detail) = &self.playlist else {
            return RowMoves::Off;
        };
        match detail.read(cx).reorder(cx) {
            Reorder::On => RowMoves::On,
            Reorder::Unsupported | Reorder::Ruled => RowMoves::Refused,
            Reorder::Off => RowMoves::Off,
        }
    }

    fn context_menu_will_open(&self, _rows: &[usize], cx: &App) {
        self.menu.reset(cx);
    }

    fn compare(&self, field: TrackField, a: usize, b: usize, cx: &App) -> Ordering {
        let tracks = self.provider.tracks(cx);
        match (field, self.provider.arrival(cx)) {
            (TrackField::AddedAt, Some(arrival)) => sort::by_arrival(arrival, a, b),
            (TrackField::AddedAt, None) => {
                let at = |row| {
                    tracks
                        .get(row)
                        .and_then(|track| self.provider.added_at(track, cx))
                };
                at(a).cmp(&at(b))
            }
            (TrackField::LikedAt, _) => {
                let at = |row| tracks.get(row).and_then(|track| self.liked_at(track, cx));
                at(a).cmp(&at(b))
            }
            _ => sort::compare(tracks, field, a, b),
        }
    }

    fn group(&self, field: TrackField, row: usize, cx: &App) -> Option<SharedString> {
        sort::group(self.provider.tracks(cx), field, row)
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
