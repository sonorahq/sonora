mod albums;
mod artists;
mod playlists;

use std::rc::Rc;

use crate::chrome::tools::{self, Sliders};
use crate::chrome::{Chrome, Searchable, Toolbar, Tooled};
use crate::shared::confirm::{Confirm, Kind};
use crate::shared::menus::{Item, new_playlist_menu};
use crate::shared::playlist_editor::{Edit, PlaylistEditor};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, MouseButton, Pixels, Point, Render, ScrollHandle,
    SharedString, WeakEntity, Window, div, point, px, relative,
};
use i18n::t;
use music::Track;
use router::{Destination, LibraryTab, LocalTab, navigate};
use state::{
    AppSettings, Library, LibraryPart, LibraryState, Origin, Playback, PlaybackState, Sonora,
};
use ui::{
    ActiveTheme as _, Button, Card, Deck, FilterChange, LEADING, Mode, Pinnable, Popovers, Popup,
    Scrollbar, Scroller, Sort, SortAxis, TableDelegate, TableEvent, TableSource, TableState, Text,
    Toggle, Vacancy, Viewport, clock, heading, quantize, scrolled, snapped, table,
};

use crate::shared::album_grid::{AlbumGrid, CardGrid};
use crate::shared::hero::{HeroMetaStrip, HeroPlayButton, PageHero};
use crate::shared::pins::Pinned as _;
use crate::shared::tracks::{
    self, LIBRARY_COLUMNS, PlaybackStatus, TrackField, TrackSource, Tracks, playback_status,
};
use crate::shared::{cards, cells, local, page};
use albums::{AlbumField, AlbumSource};
use artists::{ArtistField, ArtistSource};
use playlists::{PlaylistField, PlaylistSource};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Saved,
    Local,
}

impl Shelf {
    fn local(self) -> bool {
        self == Shelf::Local
    }
}

impl From<LibraryTab> for Section {
    fn from(tab: LibraryTab) -> Self {
        match tab {
            LibraryTab::Songs => Section::Favorites,
            LibraryTab::Albums => Section::Albums,
            LibraryTab::Playlists => Section::Playlists,
            LibraryTab::Artists => Section::Artists,
        }
    }
}

impl From<LocalTab> for Section {
    fn from(tab: LocalTab) -> Self {
        match tab {
            LocalTab::Songs => Section::Songs,
            LocalTab::Favorites => Section::Favorites,
            LocalTab::Albums => Section::Albums,
            LocalTab::Playlists => Section::Playlists,
            LocalTab::Artists => Section::Artists,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Favorites,
    Songs,
    Albums,
    Playlists,
    Artists,
}

const PINNED: [&str; 3] = ["cover", "title", "name"];
const RECENT: Sort = Sort::Descending;

#[derive(Clone)]
enum LibraryMenu {
    Background,
    Item(Item),
    Track(Track),
}

#[derive(Clone)]
enum DeckRow {
    Heading(usize),
    Cards(Vec<(usize, usize)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeckKey {
    Heading(usize),
    Card(usize),
}

impl Section {
    const ALL: [Self; 5] = [
        Self::Favorites,
        Self::Songs,
        Self::Albums,
        Self::Playlists,
        Self::Artists,
    ];

    fn key(self, shelf: Shelf) -> &'static str {
        match (shelf, self) {
            (Shelf::Saved, Section::Favorites | Section::Songs) => "songs",
            (Shelf::Saved, Section::Albums) => "albums",
            (Shelf::Saved, Section::Playlists) => "playlists",
            (Shelf::Saved, Section::Artists) => "artists",
            (Shelf::Local, Section::Favorites) => "local-favorites",
            (Shelf::Local, Section::Songs) => "local-songs",
            (Shelf::Local, Section::Albums) => "local-albums",
            (Shelf::Local, Section::Playlists) => "local-playlists",
            (Shelf::Local, Section::Artists) => "local-artists",
        }
    }

    fn mode(self) -> Mode {
        match self {
            Section::Favorites | Section::Songs => Mode::List,
            Section::Albums | Section::Playlists | Section::Artists => Mode::Grid,
        }
    }

    fn slot(self) -> usize {
        match self {
            Section::Favorites => 0,
            Section::Songs => 1,
            Section::Albums => 2,
            Section::Playlists => 3,
            Section::Artists => 4,
        }
    }

    fn listing(self) -> bool {
        matches!(self, Section::Favorites | Section::Songs)
    }

    fn vacancy(self, shelf: Shelf) -> &'static str {
        match (shelf, self) {
            (Shelf::Saved, Section::Favorites | Section::Songs) => "library-no-songs",
            (Shelf::Saved, Section::Albums) => "library-no-albums",
            (Shelf::Saved, Section::Playlists) => "library-no-playlists",
            (Shelf::Saved, Section::Artists) => "library-no-artists",
            (Shelf::Local, Section::Favorites) => "library-no-local-favorites",
            (Shelf::Local, Section::Songs) => "library-no-local-songs",
            (Shelf::Local, Section::Albums) => "library-no-local-albums",
            (Shelf::Local, Section::Playlists) => "library-no-local-playlists",
            (Shelf::Local, Section::Artists) => "library-no-local-artists",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Section::Favorites => "icons/heart.svg",
            Section::Songs => "icons/music.svg",
            Section::Albums => "icons/disc-3.svg",
            Section::Playlists => "icons/list-music.svg",
            Section::Artists => "icons/user-round.svg",
        }
    }

    fn part(self) -> LibraryPart {
        match self {
            Section::Favorites | Section::Songs => LibraryPart::Tracks,
            Section::Albums => LibraryPart::Albums,
            Section::Playlists => LibraryPart::Playlists,
            Section::Artists => LibraryPart::Artists,
        }
    }

    fn origin(self, shelf: Shelf) -> Origin {
        match (shelf, self) {
            (Shelf::Saved, _) => Origin::saved(),
            (Shelf::Local, Section::Songs) => Origin::local(),
            (Shelf::Local, _) => Origin::local_favorites(),
        }
    }
}

struct ShelfTracks {
    library: Entity<Library>,
    shelf: Shelf,
    section: Section,
}

impl Tracks for ShelfTracks {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        let library = self.library.read(cx);
        let state = match (self.shelf, self.section) {
            (Shelf::Local, Section::Favorites) => return library.local_favorites(),
            (Shelf::Local, _) => library.local_state(),
            (Shelf::Saved, _) => library.state(),
        };
        match state {
            LibraryState::Ready { tracks, .. } => tracks.as_slice(),
            _ => &[],
        }
    }

    fn is_loading(&self, cx: &App) -> bool {
        loading(&self.library, self.shelf, self.section, cx)
    }
}

fn loading(library: &Entity<Library>, shelf: Shelf, section: Section, cx: &App) -> bool {
    let library = library.read(cx);
    match (shelf, section) {
        (Shelf::Local, Section::Favorites) => library.local_favorites_loading(),
        (Shelf::Local, _) => library.local_loading(section.part()),
        (Shelf::Saved, _) => library.loading(section.part()),
    }
}

pub struct LibraryView {
    shelf: Shelf,
    library: Entity<Library>,
    settings: Entity<AppSettings>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    section: Section,
    views: [Mode; 5],
    width: Pixels,
    card_columns: usize,
    card_tile: Pixels,
    card_heading: Pixels,
    card_rows: Rc<[DeckRow]>,
    cards_dirty: bool,
    card_scrollbar: Entity<Scrollbar>,
    scrollbar: Entity<Scrollbar>,
    favorites: Entity<TableState<TrackSource>>,
    songs: Entity<TableState<TrackSource>>,
    albums: Entity<TableState<AlbumSource>>,
    playlists: Entity<TableState<PlaylistSource>>,
    artists: Entity<TableState<ArtistSource>>,
    context_menu: Option<(LibraryMenu, Point<Pixels>)>,
    toolbar: Entity<Toolbar>,
    popovers: Popovers,
    sliders: [Sliders; 5],
    me: WeakEntity<Self>,
}

impl LibraryView {
    pub fn new(
        shelf: Shelf,
        library: Entity<Library>,
        playback: Entity<Playback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let width = cells::content_width(window, Pixels::ZERO, cx);
        let settings = Sonora::global(cx).settings.clone();
        let stored = |section: Section, cx: &App| {
            let settings = settings.read(cx);
            (
                settings.table(section.key(shelf)),
                settings.sorting(section.key(shelf)),
            )
        };
        let viewed = |section: Section, cx: &App| {
            settings
                .read(cx)
                .view_or(section.key(shelf), section.mode())
        };
        let views = Section::ALL.map(|section| viewed(section, cx));

        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        let scroll = scrollbar.read(cx).scroll().clone();

        let listed = |section: Section, cx: &mut Context<TableState<TrackSource>>| {
            let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
            let origin = section.origin(shelf);
            let source = TrackSource::new(
                LIBRARY_COLUMNS,
                ShelfTracks {
                    library: library.clone(),
                    shelf,
                    section,
                },
                playback.clone(),
                playlist_scrollbar,
            )
            .from(move |_| Some(origin.clone()));
            let source = match section == Section::Songs {
                true => source.with_liked(library.clone()),
                false => source,
            };
            let source = source.table(cx.weak_entity());
            let mut delegate = TableDelegate::new(source, width, cx);
            if section == Section::Favorites {
                delegate = delegate.with_sort(TrackField::AddedAt, Sort::Descending, cx);
            }
            let (layout, sorting) = stored(section, cx);
            delegate.set_layout(layout, cx);
            if let Some(sorting) = sorting {
                delegate.set_sorting(sorting, cx);
            }
            delegate
        };

        let favorites = cx.new(|cx| {
            let delegate = listed(Section::Favorites, cx);
            TableState::new(delegate, cx).follow(scroll.clone())
        });
        let songs = cx.new(|cx| {
            let delegate = listed(Section::Songs, cx);
            TableState::new(delegate, cx).follow(scroll.clone())
        });
        let albums = cx.new(|cx| {
            let source = AlbumSource::shelved(library.clone(), playback.clone(), shelf.local());
            let mut delegate =
                TableDelegate::new(source, width, cx).with_sort(AlbumField::AddedAt, RECENT, cx);
            let (layout, sorting) = stored(Section::Albums, cx);
            delegate.set_layout(layout, cx);
            if let Some(sorting) = sorting {
                delegate.set_sorting(sorting, cx);
            }
            TableState::new(delegate, cx).follow(scroll.clone())
        });
        let playlists = cx.new(|cx| {
            let source = PlaylistSource::shelved(library.clone(), playback.clone(), shelf.local());
            let mut delegate = TableDelegate::new(source, width, cx).with_sort(
                PlaylistField::Modified,
                RECENT,
                cx,
            );
            let (layout, sorting) = stored(Section::Playlists, cx);
            delegate.set_layout(layout, cx);
            if let Some(sorting) = sorting {
                delegate.set_sorting(sorting, cx);
            }
            TableState::new(delegate, cx).follow(scroll.clone())
        });
        let artists = cx.new(|cx| {
            let source = ArtistSource::shelved(library.clone(), playback.clone(), shelf.local());
            let mut delegate =
                TableDelegate::new(source, width, cx).with_sort(ArtistField::AddedAt, RECENT, cx);
            let (layout, sorting) = stored(Section::Artists, cx);
            delegate.set_layout(layout, cx);
            if let Some(sorting) = sorting {
                delegate.set_sorting(sorting, cx);
            }
            TableState::new(delegate, cx).follow(scroll)
        });

        cx.observe(&library, |this, _, cx| {
            this.rebuild(cx);
            cx.notify();
        })
        .detach();

        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let current_playback = playback_status(&playback, cx);
        cx.observe(&playback, |this, playback, cx| {
            let current = playback_status(&playback, cx);
            if this.playback_status == current {
                return;
            }
            this.playback_status = current;
            for table in this.tables() {
                table.refresh(cx);
            }
            cx.notify();
        })
        .detach();

        cx.subscribe(&favorites, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) => this.play(*display, cx),
            TableEvent::Activated(display) => {
                let table = this.tracks().clone();
                page::play_or_toggle(&table, &this.playback, *display, cx)
            }
            TableEvent::Removed => tracks::drop_picked(&this.favorites, cx),
            _ => this.persist(Section::Favorites, cx),
        })
        .detach();

        cx.subscribe(&songs, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) => this.play(*display, cx),
            TableEvent::Activated(display) => {
                let table = this.tracks().clone();
                page::play_or_toggle(&table, &this.playback, *display, cx)
            }
            TableEvent::Removed => {}
            _ => this.persist(Section::Songs, cx),
        })
        .detach();

        cx.subscribe(&albums, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) | TableEvent::Activated(display) => {
                this.open_album(*display, cx)
            }
            TableEvent::Removed => this.drop_albums(cx),
            _ => {
                this.cards_dirty = true;
                this.persist(Section::Albums, cx);
            }
        })
        .detach();

        cx.subscribe(&playlists, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) | TableEvent::Activated(display) => {
                this.open_playlist(*display, cx)
            }
            TableEvent::Removed => this.drop_playlists(cx),
            _ => {
                this.cards_dirty = true;
                this.persist(Section::Playlists, cx);
            }
        })
        .detach();

        cx.subscribe(&artists, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) | TableEvent::Activated(display) => {
                this.open_artist(*display, cx)
            }
            TableEvent::Removed => this.drop_artists(cx),
            _ => {
                this.cards_dirty = true;
                this.persist(Section::Artists, cx);
            }
        })
        .detach();

        let me = cx.entity();
        let toolbar = Toolbar::searchable(&me, cx);

        let card_scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));

        Self {
            shelf,
            library,
            settings,
            playback,
            playback_status: current_playback,
            section: Section::Favorites,
            views,
            width,
            card_columns: 0,
            card_tile: Pixels::ZERO,
            card_heading: Pixels::ZERO,
            card_rows: Rc::from([]),
            cards_dirty: true,
            card_scrollbar,
            scrollbar,
            favorites,
            songs,
            albums,
            playlists,
            artists,
            context_menu: None,
            toolbar,
            popovers: Popovers::default(),
            sliders: Section::ALL.map(|_| Sliders::default()),
            me: me.downgrade(),
        }
    }

    fn create_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        PlaylistEditor::open(
            Edit::Create {
                tracks: Vec::new(),
                local: self.shelf.local(),
            },
            window,
            cx,
        );
        cx.notify();
    }

    pub fn section(&self) -> Section {
        self.section
    }

    fn table(&self, section: Section) -> &dyn ui::Listing {
        match section {
            Section::Favorites => &self.favorites,
            Section::Songs => &self.songs,
            Section::Albums => &self.albums,
            Section::Playlists => &self.playlists,
            Section::Artists => &self.artists,
        }
    }

    fn tables(&self) -> [&dyn ui::Listing; 5] {
        [
            &self.favorites,
            &self.songs,
            &self.albums,
            &self.playlists,
            &self.artists,
        ]
    }

    fn tracks(&self) -> &Entity<TableState<TrackSource>> {
        match self.section {
            Section::Songs => &self.songs,
            _ => &self.favorites,
        }
    }

    fn column_toggles(&self, cx: &App) -> Vec<Toggle> {
        self.table(self.section)
            .toggles(cx)
            .into_iter()
            .filter(|toggle| !PINNED.contains(&toggle.key))
            .collect()
    }

    fn switch_column(&mut self, key: &str, cx: &mut Context<Self>) {
        if PINNED.contains(&key) {
            return;
        }

        let mut layout = self.table(self.section).layout(cx);
        layout.toggle(key);
        self.table(self.section).set_layout(layout, cx);
        self.persist(self.section, cx);
        cx.notify();
    }

    fn persist(&mut self, section: Section, cx: &mut Context<Self>) {
        let key = section.key(self.shelf);
        page::store(&self.settings.clone(), self.table(section), key, key, cx);
    }

    fn unconfigured(&self, cx: &App) -> bool {
        self.shelf.local() && Sonora::global(cx).session.read(cx).local_path().is_none()
    }

    fn note(&self, cx: &App) -> Option<Vacancy> {
        if loading(&self.library, self.shelf, self.section, cx) {
            return None;
        }
        let library = self.library.read(cx);
        let table = self.table(self.section);
        let state = match self.shelf {
            Shelf::Local => library.local_state(),
            Shelf::Saved => library.state(),
        };
        match state {
            LibraryState::Loading => return None,
            LibraryState::Failed(_) => return Some(Vacancy::new(t!("library-not-loaded"))),
            _ if table.row_count(cx) > 0 => return None,
            _ => {}
        }

        let failed = match self.shelf {
            Shelf::Local => library.local_part_failed(self.section.part()),
            Shelf::Saved => library.part_failed(self.section.part()),
        };

        Some(match (table.filtering(cx), failed) {
            (true, _) => Vacancy::new(t!("library-no-matches")),
            (false, true) => Vacancy::new(t!("library-part-not-loaded")),
            (false, false) => Vacancy::new(i18n::lookup(self.section.vacancy(self.shelf), None))
                .icon(self.section.glyph()),
        })
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.shelf.local() {
            return;
        }
        self.library.update(cx, |library, cx| library.refresh(cx));
    }

    pub fn select(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.section != section {
            self.scrollbar
                .read(cx)
                .scroll()
                .set_offset(Point::default());
            self.cards_dirty = true;
        }
        self.section = section;
        if self.mode() == Mode::List {
            self.table(section).set_width(self.width, cx);
        }
        cx.notify();
    }

    fn viewport(scroll: &ScrollHandle, window: &Window) -> Viewport {
        quantize(scroll, window);
        let visible = scroll.bounds().size.height;

        Viewport::measured(scrolled(scroll), visible, window)
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.tracks().read(cx);
        let delegate = state.delegate();
        let count = delegate.row_count();
        let duration: std::time::Duration = (0..count)
            .filter_map(|display| delegate.source().peek(delegate.row(display), cx))
            .map(|track| track.duration)
            .sum();
        let mut strip = HeroMetaStrip::new().text(t!("count-songs", count = count));
        if !duration.is_zero() {
            strip = strip.text(clock(duration));
        }
        let songs = self.section == Section::Songs;
        let (title, icon, eyebrow) = match songs {
            true => (t!("nav-songs"), "icons/disc-3.svg", t!("nav-local")),
            false => (
                t!("library-liked-songs"),
                "icons/heart-filled.svg",
                t!("detail-playlist"),
            ),
        };

        PageHero::new("library-hero", title)
            .fallback(icon)
            .accent()
            .eyebrow(eyebrow)
            .meta(strip)
            .actions(HeroPlayButton::listed(
                "play-library",
                t!("library-play-liked-songs"),
                self.tracks(),
                self.playback.clone(),
            ))
            .into_any_element()
    }

    fn play(&mut self, display: usize, cx: &mut Context<Self>) {
        let table = self.tracks().clone();
        let queued = tracks::ordered(&table, cx);
        let from = tracks::whence(&table, cx);
        self.playback
            .update(cx, |playback, cx| playback.start(queued, display, from, cx));
    }

    fn open_album(&mut self, display: usize, cx: &mut Context<Self>) {
        let album = {
            let state = self.albums.read(cx);
            let row = state.delegate().row(display);
            state.delegate().source().at(row, cx)
        };
        let Some(album) = album else {
            return;
        };
        navigate(Destination::Album(album.id.into()), cx);
    }

    fn open_playlist(&mut self, display: usize, cx: &mut Context<Self>) {
        let playlist = {
            let state = self.playlists.read(cx);
            let row = state.delegate().row(display);
            state.delegate().source().at(row, cx)
        };
        let Some(playlist) = playlist else {
            return;
        };
        navigate(Destination::Playlist(playlist.id.into()), cx);
    }

    fn open_artist(&mut self, display: usize, cx: &mut Context<Self>) {
        let artist = {
            let state = self.artists.read(cx);
            let row = state.delegate().row(display);
            state.delegate().source().at(row, cx)
        };
        let Some(artist) = artist else {
            return;
        };
        navigate(Destination::Artist(artist.id.into()), cx);
    }

    fn drop_albums(&mut self, cx: &mut Context<Self>) {
        let rows = self.albums.read(cx).delegate().picked();
        let albums: Vec<_> = {
            let state = self.albums.read(cx);
            let source = state.delegate().source();
            rows.iter().filter_map(|&row| source.at(row, cx)).collect()
        };
        if albums.is_empty() {
            return;
        }
        let library = self.library.clone();
        let table = self.albums.clone();
        Confirm::ask(
            Kind::Albums(albums.len()),
            move |cx| {
                library.update(cx, |library, cx| {
                    for album in albums {
                        library.toggle_album(album, cx);
                    }
                });
                table.update(cx, |table, cx| {
                    table.delegate_mut().clear_selection();
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn drop_artists(&mut self, cx: &mut Context<Self>) {
        let rows = self.artists.read(cx).delegate().picked();
        let artists: Vec<_> = {
            let state = self.artists.read(cx);
            let source = state.delegate().source();
            rows.iter().filter_map(|&row| source.at(row, cx)).collect()
        };
        if artists.is_empty() {
            return;
        }
        let library = self.library.clone();
        let table = self.artists.clone();
        Confirm::ask(
            Kind::Artists(artists.len()),
            move |cx| {
                library.update(cx, |library, cx| {
                    for artist in artists {
                        library.toggle_artist(artist, cx);
                    }
                });
                table.update(cx, |table, cx| {
                    table.delegate_mut().clear_selection();
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn drop_playlists(&mut self, cx: &mut Context<Self>) {
        let rows = self.playlists.read(cx).delegate().picked();
        let ids: Vec<String> = {
            let state = self.playlists.read(cx);
            let source = state.delegate().source();
            rows.iter()
                .filter_map(|&row| source.at(row, cx))
                .filter(|playlist| !playlist.owned)
                .map(|playlist| playlist.id)
                .collect()
        };
        if ids.is_empty() {
            return;
        }
        let library = self.library.clone();
        let table = self.playlists.clone();
        Confirm::ask(
            Kind::Playlists(ids.len()),
            move |cx| {
                library.update(cx, |library, cx| {
                    for id in ids {
                        library.remove_playlist_from_library(id, cx);
                    }
                });
                table.update(cx, |table, cx| {
                    table.delegate_mut().clear_selection();
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn resize(&mut self, window: &Window, cx: &mut Context<Self>) {
        let width = cells::content_width(window, Pixels::ZERO, cx);
        if (width - self.width).abs() < px(0.5) {
            return;
        }
        self.width = width;

        if self.mode() == Mode::List {
            self.table(self.section).set_width(width, cx);
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.cards_dirty = true;
        for table in self.tables() {
            table.rebuild(cx);
        }
    }

    fn cards(&mut self, window: &Window, cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let inset = theme.metrics.inset;
        let room = cells::content_width(window, page::reserved(inset), cx);
        let layout = CardGrid::layout(room);
        let columns = layout.columns;
        let card = layout.card;

        let scroll = self.card_scrollbar.read(cx).scroll().clone();
        let gap = deck_gap(window);
        let tile = Card::tile_height(card, window, cx);
        let heading = head_height(window, cx);
        let depth = (scrolled(&scroll) - inset).max(Pixels::ZERO);

        let repacked = self.card_columns != columns
            || (self.card_tile - tile).abs() >= px(0.5)
            || (self.card_heading - heading).abs() >= px(0.5);
        let anchor = (repacked && self.card_columns != 0 && !self.cards_dirty)
            .then(|| {
                let heights = deck_heights(&self.card_rows, self.card_tile, self.card_heading);
                let (index, offset) = Deck::at(&heights, gap, depth);
                let share = match heights.get(index) {
                    Some(height) if *height > Pixels::ZERO => offset / *height,
                    _ => 0.,
                };
                deck_key(&self.card_rows, index).map(|key| (key, share))
            })
            .flatten();

        if self.card_columns != columns {
            self.card_columns = columns;
            self.cards_dirty = true;
        }
        if self.cards_dirty {
            self.card_rows = match self.section {
                Section::Favorites => deck(&self.favorites, columns, cx),
                Section::Songs => deck(&self.songs, columns, cx),
                Section::Albums => deck(&self.albums, columns, cx),
                Section::Playlists => deck(&self.playlists, columns, cx),
                Section::Artists => deck(&self.artists, columns, cx),
            }
            .into();
            self.cards_dirty = false;
        }
        self.card_tile = tile;
        self.card_heading = heading;

        let heights = deck_heights(&self.card_rows, tile, heading);
        if let Some((index, share)) =
            anchor.and_then(|(key, share)| deck_row(&self.card_rows, key).map(|row| (row, share)))
        {
            let top = Deck::tops(&heights, gap)
                .get(index)
                .copied()
                .unwrap_or(Pixels::ZERO);
            let into = heights.get(index).copied().unwrap_or(Pixels::ZERO) * share;
            scroll.set_offset(point(Pixels::ZERO, -(top + into + inset)));
        }

        let visible = scroll.bounds().size.height;
        let viewport = Viewport::measured(scrolled(&scroll) - inset, visible, window);
        let rows = self.card_rows.clone();
        let section = self.section;
        let view = self.me.clone();

        Scroller::new("library-cards", &self.card_scrollbar)
            .py(inset)
            .child(
                Deck::new("library-deck")
                    .viewport(viewport)
                    .rows(heights)
                    .gap(gap)
                    .draw(move |index, _, cx| {
                        let Some(row) = rows.get(index) else {
                            return div().into_any_element();
                        };
                        let Some(view) = view.upgrade() else {
                            return div().into_any_element();
                        };
                        let view = view.read(cx);

                        match row {
                            DeckRow::Heading(display) => {
                                let label = match section {
                                    Section::Favorites => {
                                        view.favorites.read(cx).delegate().group(*display, cx)
                                    }
                                    Section::Songs => {
                                        view.songs.read(cx).delegate().group(*display, cx)
                                    }
                                    Section::Albums => {
                                        view.albums.read(cx).delegate().group(*display, cx)
                                    }
                                    Section::Playlists => {
                                        view.playlists.read(cx).delegate().group(*display, cx)
                                    }
                                    Section::Artists => {
                                        view.artists.read(cx).delegate().group(*display, cx)
                                    }
                                };

                                div()
                                    .px(inset)
                                    .children(label.map(|label| head(label, cx)))
                                    .into_any_element()
                            }
                            DeckRow::Cards(cards) => {
                                let row = match section {
                                    Section::Favorites | Section::Songs => CardGrid::new(room)
                                        .children(cards.iter().filter_map(|&(display, row)| {
                                            view.track_card(display, row, card, cx)
                                        }))
                                        .into_any_element(),
                                    Section::Albums => {
                                        view.album_grid(cards, room, cx).into_any_element()
                                    }
                                    Section::Playlists => CardGrid::new(room)
                                        .children(cards.iter().filter_map(|&(display, row)| {
                                            view.playlist_card(display, row, card, cx)
                                        }))
                                        .into_any_element(),
                                    Section::Artists => CardGrid::new(room)
                                        .children(cards.iter().filter_map(|&(display, row)| {
                                            view.artist_card(display, row, card, cx)
                                        }))
                                        .into_any_element(),
                                };

                                div().px(inset).child(row).into_any_element()
                            }
                        }
                    }),
            )
            .into_any_element()
    }

    fn track_card(&self, display: usize, row: usize, card: Pixels, cx: &App) -> Option<AnyElement> {
        let theme = *cx.theme();
        let listing = self.tracks();
        let track = listing.read(cx).delegate().source().at(row, cx)?;
        let playable = track.playable;
        let pressed = (listing.clone(), self.playback.clone());
        let played = pressed.clone();
        let state = listing.read(cx).delegate().source().now_playing(row, cx);
        let playing = matches!(state, Some(PlaybackState::Playing));
        let artists = cells::artist_links(
            SharedString::from(format!("library-track-artist-{display}")),
            track.artist_refs.clone(),
            track.artists.clone(),
            theme.muted_foreground,
        )
        .text_size(theme.text(Text::Small))
        .truncate();

        let pin = track.pin();
        let context = track.clone();
        let view = self.me.clone();

        Some(
            Card::new(("library-track", display), SharedString::from(track.name))
                .tile(card)
                .cover(track.cover)
                .when_some(pin, Pinnable::pin)
                .weight(FontWeight::SEMIBOLD)
                .flat()
                .when(track.explicit, Card::explicit)
                .bare_meta(artists)
                .menu(move |event, _, cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this.tracks().read(cx).delegate().source().menu().reset(cx);
                        this.context_menu =
                            Some((LibraryMenu::Track(context.clone()), event.position));
                        cx.notify();
                    });
                })
                .when(playable, move |card| {
                    card.play(playing, move |_, _, cx| match &state {
                        Some(PlaybackState::Playing) => {
                            played.1.update(cx, |playback, cx| playback.pause(cx))
                        }
                        Some(PlaybackState::Paused) => {
                            played.1.update(cx, |playback, cx| playback.resume(cx))
                        }
                        _ => page::play(&played.0, &played.1, display, cx),
                    })
                    .press(move |_, _, cx| page::play(&pressed.0, &pressed.1, display, cx))
                })
                .into_any_element(),
        )
    }

    fn album_grid(&self, cards: &[(usize, usize)], room: Pixels, cx: &App) -> AlbumGrid {
        let albums = cards.iter().filter_map(|&(display, row)| {
            self.albums
                .read(cx)
                .delegate()
                .source()
                .at(row, cx)
                .map(|album| (display, album))
        });
        let view = self.me.clone();

        AlbumGrid::new("library-album", room, albums, self.playback.clone()).on_context(
            move |album, position, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.context_menu =
                        Some((LibraryMenu::Item(Item::Album(album.clone())), position));
                    cx.notify();
                });
            },
        )
    }

    fn playlist_card(
        &self,
        display: usize,
        row: usize,
        card: Pixels,
        cx: &App,
    ) -> Option<AnyElement> {
        let playlist = self.playlists.read(cx).delegate().source().at(row, cx)?;
        let view = self.me.clone();
        let build = match self.shelf.local() {
            true => cards::imported_playlist_card,
            false => cards::playlist_card,
        };

        Some(
            build(("library-playlist", display), &playlist, &self.playback, cx)
                .tile(card)
                .flat()
                .menu(move |event, _, cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this.context_menu = Some((
                            LibraryMenu::Item(Item::Playlist(playlist.clone())),
                            event.position,
                        ));
                        cx.notify();
                    });
                })
                .into_any_element(),
        )
    }

    fn artist_card(
        &self,
        display: usize,
        row: usize,
        card: Pixels,
        cx: &App,
    ) -> Option<AnyElement> {
        let artist = self.artists.read(cx).delegate().source().at(row, cx)?;
        let context = artist.clone();
        let view = self.me.clone();

        Some(
            cards::artist_card(("library-artist", display), &artist, &self.playback, cx)
                .tile(card)
                .flat()
                .menu(move |event, _, cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this.context_menu = Some((
                            LibraryMenu::Item(Item::Artist(context.clone())),
                            event.position,
                        ));
                        cx.notify();
                    });
                })
                .into_any_element(),
        )
    }
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.resize(window, cx);

        let theme = *cx.theme();
        let inset = theme.metrics.inset;
        let mode = self.mode();
        if mode == Mode::List {
            self.table(self.section).claim(cx);
            let scroll = self.scrollbar.read(cx).scroll().clone();
            let viewport = match self.section.listing() {
                true => page::viewport(&scroll, inset, window),
                false => Self::viewport(&scroll, window),
            };
            self.table(self.section).set_viewport(viewport, cx);
        }

        let context_menu = self.context_menu.clone().map(|(target, position)| {
            let menu = match target {
                LibraryMenu::Item(item) => item.menu(self.playback.clone(), false, cx),
                LibraryMenu::Track(track) => self
                    .tracks()
                    .read(cx)
                    .delegate()
                    .source()
                    .menu()
                    .for_track(&track, cx),
                LibraryMenu::Background => new_playlist_menu(cx.listener(|this, _, window, cx| {
                    this.create_playlist(window, cx);
                })),
            };
            Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
        });
        let view = cx.entity().downgrade();
        let section = self.section;
        let note = self.note(cx);
        let content = match (self.section, mode) {
            _ if self.unconfigured(cx) => local::unconfigured("configure-local-folder")
                .size_full()
                .into_any_element(),
            (section, Mode::List) if section.listing() => {
                Scroller::new("library-page", &self.scrollbar)
                    .pt(inset)
                    .pb(inset)
                    .child(div().px(inset).child(self.header(cx)))
                    .child(table(self.tracks()))
                    .when_some(note, |this, note| this.child(note))
                    .into_any_element()
            }
            (_, Mode::List) => Scroller::new("library-page", &self.scrollbar)
                .pb(inset)
                .child(self.table(self.section).element())
                .when_some(note, |this, note| this.child(note))
                .into_any_element(),
            (_, Mode::Grid) => match note {
                Some(note) => note.size_full().into_any_element(),
                None => self.cards(window, cx),
            },
        };

        div()
            .relative()
            .size_full()
            .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                if section != Section::Playlists {
                    return;
                }
                window.prevent_default();
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.context_menu = Some((LibraryMenu::Background, event.position));
                    cx.notify();
                });
            })
            .child(content)
            .when_some(context_menu, |this, menu| this.child(menu))
    }
}

impl Searchable for LibraryView {
    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.cards_dirty = true;
        for table in self.tables() {
            table.set_query(query, cx);
        }
        cx.notify();
    }

    fn hint() -> SharedString {
        "filter-library".into()
    }
}

impl LibraryView {
    fn sorts(&self, cx: &App) -> Vec<SortAxis> {
        self.table(self.section).sortables(cx)
    }

    fn set_sort(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.table(self.section).cycle_sort(key, cx);
        self.cards_dirty = true;
        cx.notify();
    }

    fn mode(&self) -> Mode {
        match self.section.listing() {
            true => Mode::List,
            false => self.views[self.section.slot()],
        }
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let section = self.section;
        self.views[section.slot()] = mode;
        if mode == Mode::List {
            self.table(section).set_width(self.width, cx);
        }

        let settings = self.settings.clone();
        let key = section.key(self.shelf);
        settings.update(cx, |settings, cx| settings.set_view(key, mode, cx));
        cx.notify();
    }
}

impl Tooled for LibraryView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, cx: &App) -> Vec<AnyElement> {
        let columned = self.me.clone();
        let filtered = self.me.clone();
        let sorted = self.me.clone();
        let viewed = self.me.clone();

        let columns = matches!(self.mode(), Mode::List).then(|| {
            tools::columns(&self.popovers, self.column_toggles(cx), move |key, cx| {
                columned
                    .update(cx, |view, cx| view.switch_column(key, cx))
                    .ok();
            })
        });

        let created = self.me.clone();
        let create = (self.section == Section::Playlists).then(|| {
            Button::new("new-playlist")
                .icon("icons/plus.svg")
                .tooltip("menu-new-playlist")
                .small()
                .ghost()
                .on_click(move |_, window, cx| {
                    created
                        .update(cx, |view, cx| view.create_playlist(window, cx))
                        .ok();
                })
                .into_any_element()
        });

        let mut tools = Vec::new();
        tools.extend(create);
        tools.extend(columns);
        if self.section != Section::Artists {
            tools.push(tools::filters(
                &self.popovers,
                &self.sliders[self.section.slot()],
                self.table(self.section).filters(cx),
                move |change, cx| {
                    filtered.update(cx, |view, cx| view.filter(change, cx)).ok();
                },
                cx,
            ));
        }
        tools.push(tools::sorts(
            &self.popovers,
            self.sorts(cx),
            move |key, cx| {
                sorted.update(cx, |view, cx| view.set_sort(key, cx)).ok();
            },
            cx,
        ));
        let switchable = !self.section.listing();
        tools.extend(switchable.then(|| {
            tools::views(&self.popovers, self.mode(), move |mode, cx| {
                viewed.update(cx, |view, cx| view.set_mode(mode, cx)).ok();
            })
        }));
        tools
    }
}

impl LibraryView {
    fn filter(&mut self, change: FilterChange, cx: &mut Context<Self>) {
        self.cards_dirty = true;
        self.table(self.section).filter(change, cx);
        cx.notify();
    }
}

fn deck<S: TableSource>(state: &Entity<TableState<S>>, columns: usize, cx: &App) -> Vec<DeckRow> {
    let state = state.read(cx);
    let delegate = state.delegate();
    let mut rows = Vec::new();
    let mut cards = Vec::with_capacity(columns);
    let mut group: Option<SharedString> = None;

    for display in 0..delegate.row_count() {
        let label = delegate.group(display, cx);
        match &label {
            Some(text) if group.as_ref() != Some(text) => {
                if !cards.is_empty() {
                    rows.push(DeckRow::Cards(std::mem::take(&mut cards)));
                }
                rows.push(DeckRow::Heading(display));
            }
            _ => {}
        }
        group = label;
        cards.push((display, delegate.row(display)));
        if cards.len() == columns {
            rows.push(DeckRow::Cards(std::mem::take(&mut cards)));
            cards.reserve(columns);
        }
    }
    if !cards.is_empty() {
        rows.push(DeckRow::Cards(cards));
    }

    rows
}

fn deck_key(rows: &[DeckRow], item_ix: usize) -> Option<DeckKey> {
    match rows.get(item_ix)? {
        DeckRow::Heading(display) => Some(DeckKey::Heading(*display)),
        DeckRow::Cards(cards) => cards.first().map(|(display, _)| DeckKey::Card(*display)),
    }
}

fn deck_row(rows: &[DeckRow], key: DeckKey) -> Option<usize> {
    rows.iter().position(|row| match (row, key) {
        (DeckRow::Heading(display), DeckKey::Heading(anchor)) => *display == anchor,
        (DeckRow::Cards(cards), DeckKey::Card(anchor)) => {
            cards.iter().any(|(display, _)| *display == anchor)
        }
        _ => false,
    })
}

fn head(label: SharedString, cx: &App) -> AnyElement {
    heading(label, cx)
        .w_full()
        .pt_2()
        .line_height(relative(LEADING))
        .into_any_element()
}

fn head_height(window: &Window, cx: &App) -> Pixels {
    let title = px((cx.theme().text(Text::Title) / px(1.) * LEADING).round());

    snapped(window.rem_size() * 0.5 + title, window)
}

fn deck_gap(window: &Window) -> Pixels {
    snapped(window.rem_size() * 1.5, window)
}

fn deck_heights(rows: &[DeckRow], tile: Pixels, heading: Pixels) -> Vec<Pixels> {
    rows.iter()
        .map(|row| match row {
            DeckRow::Heading(_) => heading,
            DeckRow::Cards(_) => tile,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_anchor_follows_a_repacked_row() {
        let before = vec![
            DeckRow::Cards(vec![(0, 10), (1, 11)]),
            DeckRow::Cards(vec![(2, 12), (3, 13)]),
        ];
        let after = vec![
            DeckRow::Cards(vec![(0, 10)]),
            DeckRow::Cards(vec![(1, 11)]),
            DeckRow::Cards(vec![(2, 12)]),
            DeckRow::Cards(vec![(3, 13)]),
        ];

        let anchor = deck_key(&before, 1).unwrap();

        assert_eq!(anchor, DeckKey::Card(2));
        assert_eq!(deck_row(&after, anchor), Some(2));
    }

    #[test]
    fn heading_anchor_survives_a_repack() {
        let rows = vec![DeckRow::Heading(0), DeckRow::Cards(vec![(0, 10), (1, 11)])];

        let anchor = deck_key(&rows, 0).unwrap();

        assert_eq!(anchor, DeckKey::Heading(0));
        assert_eq!(deck_row(&rows, anchor), Some(0));
    }
}
