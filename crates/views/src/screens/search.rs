use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, WeakEntity, Window, div,
};
use i18n::t;
use input::SEARCH_CONTEXT;
use music::{Album, Playlist, ReleaseType, SavedArtist, Track};
use router::{Destination, navigate};
use ui::Input;

use crate::chrome::Chrome;
use crate::shared::menus::{ItemMenu, album_menu, artist_menu, playlist_menu};
use state::{AlbumHit, ArtistHit, Genres, Hit, Kind, Playback, PlaylistHit, Search, Sonora};
use ui::ActiveTheme as _;
use ui::{
    Activate, Card, Deck, Deselect, Pinnable, Popup, Room, Scrollbar, Scroller, SelectLeft,
    SelectNext, SelectPrevious, SelectRight, Separator, Text, Theme, VAST, clock, eyebrow,
    scrolled, snapped, tabular, vacant,
};

use crate::shared::cards;
use crate::shared::cells;
use crate::shared::pins::Pinned as _;
use crate::shared::shelves;

const RAIL: Pixels = gpui::px(12.);
const ROW_GAP: f32 = 0.25;
const SONGS: &[Kind] = &[Kind::Song];
const ARTISTS: &[Kind] = &[Kind::Artist];
const RELEASES: &[Kind] = &[Kind::Album, Kind::Playlist];
use crate::shared::tracks::{PlaybackStatus, playback_status};

type Play = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

enum Press {
    Song(Box<Track>),
    Artist(String),
    Album(String),
    Playlist(String),
}

#[derive(Clone)]
enum HitMenu {
    Song(Box<Track>),
    Album(AlbumHit),
    Playlist(PlaylistHit),
    Artist(ArtistHit),
}

impl HitMenu {
    fn of(hit: &Hit) -> Option<Self> {
        match hit {
            Hit::Song(track) => Some(Self::Song(Box::new(track.clone()))),
            Hit::Album(album) => Some(Self::Album(album.clone())),
            Hit::Playlist(list) => Some(Self::Playlist(list.clone())),
            Hit::Artist(artist) if artist.id.is_some() => Some(Self::Artist(artist.clone())),
            Hit::Artist(_) => None,
        }
    }
}

pub(crate) struct SearchView {
    input: Entity<Input>,
    search: Entity<Search>,
    genres: Entity<Genres>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    songs: Entity<Scrollbar>,
    artists: Entity<Scrollbar>,
    albums: Entity<Scrollbar>,
    mixed: Entity<Scrollbar>,
    browsing: Entity<Scrollbar>,
    track_menu: ItemMenu,
    context_menu: Option<(HitMenu, Point<Pixels>)>,
    focus: FocusHandle,
    cursor: Option<(usize, usize)>,
    rows: [usize; 3],
    lead: Rc<Cell<Pixels>>,
}

impl SearchView {
    pub(crate) fn new(
        search: Entity<Search>,
        genres: Entity<Genres>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&genres, |_, _, cx| cx.notify()).detach();
        genres.update(cx, |genres, cx| genres.load(cx));
        let input = cx.new(|cx| {
            Input::new("search-placeholder", cx)
                .icon("icons/search.svg")
                .clearable()
        });

        cx.observe(&input, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            this.search.update(cx, |search, cx| search.ask(&query, cx));
        })
        .detach();

        cx.observe(&search, |this, _, cx| {
            this.track_menu.reset(cx);
            this.context_menu = None;
            this.cursor = None;
            this.rows = [0; 3];
            cx.notify();
        })
        .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        let current_playback = playback_status(&playback, cx);
        cx.observe(&playback, |this, playback, cx| {
            let current = playback_status(&playback, cx);
            if this.playback_status != current {
                this.playback_status = current;
                cx.notify();
            }
        })
        .detach();

        let asked = input.read(cx).text().to_owned();
        search.update(cx, |search, cx| search.ask(&asked, cx));

        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));

        Self {
            input,
            search,
            genres,
            playback,
            playback_status: current_playback,
            songs: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            artists: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            albums: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            mixed: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            browsing: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            track_menu: ItemMenu::new(playlist_scrollbar),
            context_menu: None,
            focus: cx.focus_handle(),
            cursor: None,
            rows: [0; 3],
            lead: Rc::new(Cell::new(Pixels::ZERO)),
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).query().trim().is_empty() {
            return;
        }
        let stacked = stacked(window, cx);
        match self.cursor {
            None => {
                let Some(column) = self.first_filled(0, stacked, cx) else {
                    return;
                };
                self.place(column, 0);
            }
            Some((column, row)) => {
                let last = self
                    .seats(kinds(column, stacked), cx)
                    .len()
                    .saturating_sub(1);
                if row < last {
                    self.place(column, row + 1);
                }
            }
        }
        window.focus(&self.focus, cx);
        self.reveal(window, cx);
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, window: &mut Window, cx: &mut Context<Self>) {
        let Some((column, row)) = self.cursor else {
            return;
        };
        if row == 0 {
            self.cursor = None;
            self.input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        self.place(column, row - 1);
        window.focus(&self.focus, cx);
        self.reveal(window, cx);
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        let stacked = stacked(window, cx);
        let Some((column, _)) = self.cursor else {
            return;
        };
        let Some(next) = self.prev_filled(column, stacked, cx) else {
            return;
        };
        self.hop(next, stacked, cx);
        window.focus(&self.focus, cx);
        self.reveal(window, cx);
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        let stacked = stacked(window, cx);
        let Some((column, _)) = self.cursor else {
            return;
        };
        let Some(next) = self.next_filled(column, stacked, cx) else {
            return;
        };
        self.hop(next, stacked, cx);
        window.focus(&self.focus, cx);
        self.reveal(window, cx);
        cx.notify();
    }

    fn activate_hit(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        let Some((column, row)) = self.cursor else {
            return;
        };
        let stacked = stacked(window, cx);
        let seats = self.seats(kinds(column, stacked), cx);
        let Some(&(at, _)) = seats.get(row) else {
            return;
        };
        let Some(hit) = self.search.read(cx).hits().get(at).cloned() else {
            return;
        };
        match hit {
            Hit::Song(track) => {
                let current = track.id.is_some() && track.id == self.playback_status.0;
                self.playback.update(cx, |playback, cx| match current {
                    true => playback.toggle_play(cx),
                    false => playback.play_radio(&track, cx),
                });
            }
            Hit::Artist(artist) => {
                if let Some(id) = artist.id {
                    navigate(Destination::Artist(id.into()), cx);
                }
            }
            Hit::Album(album) => navigate(Destination::Album(album.id.into()), cx),
            Hit::Playlist(list) => navigate(Destination::Playlist(list.id.into()), cx),
        }
    }

    fn deselect_hit(&mut self, _: &Deselect, window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor.take().is_none() {
            return;
        }
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn place(&mut self, column: usize, row: usize) {
        if let Some(slot) = self.rows.get_mut(column) {
            *slot = row;
        }
        self.cursor = Some((column, row));
    }

    fn hop(&mut self, column: usize, stacked: bool, cx: &App) {
        let last = self
            .seats(kinds(column, stacked), cx)
            .len()
            .saturating_sub(1);
        let row = self.rows.get(column).copied().unwrap_or(0).min(last);
        self.place(column, row);
    }

    fn first_filled(&self, start: usize, stacked: bool, cx: &App) -> Option<usize> {
        (start..columns(stacked)).find(|&column| !self.seats(kinds(column, stacked), cx).is_empty())
    }

    fn next_filled(&self, column: usize, stacked: bool, cx: &App) -> Option<usize> {
        self.first_filled(column + 1, stacked, cx)
    }

    fn prev_filled(&self, column: usize, stacked: bool, cx: &App) -> Option<usize> {
        (0..column)
            .rev()
            .find(|&at| !self.seats(kinds(at, stacked), cx).is_empty())
    }

    fn reveal(&self, window: &Window, cx: &App) {
        let Some((column, row)) = self.cursor else {
            return;
        };
        let stacked = stacked(window, cx);
        let scroll = self.bar(column, stacked).read(cx).scroll().clone();
        let theme = *cx.theme();
        let height = snapped(theme.metrics.list_row, window);
        let gap = theme.font_size * ROW_GAP;
        let above = match stacked {
            true => self.lead.get(),
            false => Pixels::ZERO,
        };
        let top = (height + gap) * row as f32 + above;
        let visible = scroll.bounds().size.height;
        if visible <= Pixels::ZERO {
            return;
        }
        let offset = scroll.offset();
        let shown = -offset.y;
        let delta = if top < shown {
            top - shown
        } else if top + height > shown + visible {
            top + height - (shown + visible)
        } else {
            return;
        };
        scroll.set_offset(gpui::point(offset.x, offset.y - delta));
    }

    fn bar(&self, column: usize, stacked: bool) -> &Entity<Scrollbar> {
        match (stacked, column) {
            (true, _) => &self.mixed,
            (false, 0) => &self.songs,
            (false, 1) => &self.artists,
            _ => &self.albums,
        }
    }

    fn subtitle(&self, hit: &Hit, place: usize, compact: bool, theme: &Theme) -> AnyElement {
        if let (Hit::Album(album), false) = (hit, compact) {
            return cards::released(
                format!("album-artist-{place}"),
                album.year,
                album.artist_refs.clone(),
                album.artists.clone(),
                theme,
            )
            .into_any_element();
        }

        let (kind, id, artists, fallback) = match hit {
            Hit::Song(track) => (
                Kind::Song,
                format!("song-artist-{place}"),
                track.artist_refs.clone(),
                track.artists.clone(),
            ),
            Hit::Album(album) => (
                Kind::Album,
                format!("album-artist-{place}"),
                album.artist_refs.clone(),
                album.artists.clone(),
            ),
            Hit::Playlist(list) if !list.owner.is_empty() => (
                Kind::Playlist,
                format!("playlist-owner-{place}"),
                Vec::new(),
                list.owner.clone(),
            ),
            Hit::Artist(_) | Hit::Playlist(_) => {
                return meta(hit, compact).into_any_element();
            }
        };

        let links = cells::artist_links(id, artists, fallback, theme.muted_foreground).truncate();

        match compact {
            true => div()
                .flex()
                .min_w_0()
                .gap_1()
                .child(div().flex_none().child(tag(kind)))
                .child(links)
                .into_any_element(),
            false => links.into_any_element(),
        }
    }

    fn row(
        &self,
        hit: &Hit,
        place: usize,
        compact: bool,
        chosen: bool,
        me: &WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        let theme = *cx.theme();
        let meta = self.subtitle(hit, place, compact, &theme);

        let card = match hit {
            Hit::Song(track) => {
                let current = track.id.is_some() && track.id == self.playback_status.0;
                let tint = match current {
                    true => theme.primary,
                    false => theme.foreground,
                };
                Card::new(("song", place), track.name.clone())
                    .cover(track.cover.clone())
                    .tint(tint)
                    .meta(meta)
                    .when(track.explicit, |card| card.explicit())
                    .trailing(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .font_features(tabular())
                            .child(clock(track.duration)),
                    )
                    .press(pressed(Press::Song(Box::new(track.clone())), me))
            }
            Hit::Artist(artist) => {
                let card = Card::new(("artist", place), artist.name.clone())
                    .cover(artist.cover.clone())
                    .circle()
                    .underline()
                    .meta(meta);
                match &artist.id {
                    Some(id) => card.press(pressed(Press::Artist(id.clone()), me)),
                    None => card,
                }
            }
            Hit::Album(album) => Card::new(("album", place), album.name.clone())
                .cover(album.cover.clone())
                .underline()
                .meta(meta)
                .press(pressed(Press::Album(album.id.clone()), me)),
            Hit::Playlist(list) => Card::new(("playlist", place), list.name.clone())
                .cover(list.cover.clone())
                .underline()
                .meta(meta)
                .press(pressed(Press::Playlist(list.id.clone()), me)),
        };

        card.when_some(self.transport(hit, me, cx), |card, (playing, play)| {
            card.play(playing, play)
        })
        .when_some(hit.pin(), Pinnable::pin)
        .when_some(HitMenu::of(hit), |card, target| card.menu(menu(target, me)))
        .chosen(chosen)
        .into_any_element()
    }

    fn transport(&self, hit: &Hit, me: &WeakEntity<Self>, cx: &App) -> Option<(bool, Play)> {
        let me = me.clone();
        let origin = match hit {
            Hit::Song(track) => {
                let current = track.id.is_some() && track.id == self.playback_status.0;
                let playing =
                    current && matches!(self.playback_status.1, state::PlaybackState::Playing);
                let track = track.clone();
                let play: Play = Box::new(move |_, _, cx| {
                    me.update(cx, |this, cx| {
                        this.playback.update(cx, |playback, cx| match current {
                            true => playback.toggle_play(cx),
                            false => playback.play_radio(&track, cx),
                        });
                    })
                    .ok();
                });
                return Some((playing, play));
            }
            Hit::Artist(artist) => {
                state::Origin::artist(artist.id.clone()?).named(artist.name.clone())
            }
            Hit::Album(album) => state::Origin::album(album.id.clone()).named(album.name.clone()),
            Hit::Playlist(list) => {
                state::Origin::playlist(list.id.clone()).named(list.name.clone())
            }
        };
        let playing =
            self.playback.read(cx).playing_from(&origin) == Some(state::PlaybackState::Playing);
        let play: Play = Box::new(move |_, _, cx| {
            me.update(cx, |this, cx| {
                this.playback
                    .update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
            })
            .ok();
        });

        Some((playing, play))
    }

    fn best(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let theme = *cx.theme();
        let hit = self.search.read(cx).best()?;
        let (kind, title, artists, target) = match hit {
            Hit::Song(track) => (
                Kind::Song,
                track.name.clone(),
                track.artist_refs.clone(),
                Some(Press::Song(Box::new(track.clone()))),
            ),
            Hit::Artist(artist) => (
                Kind::Artist,
                artist.name.clone(),
                Vec::new(),
                artist.id.clone().map(Press::Artist),
            ),
            Hit::Album(album) => (
                Kind::Album,
                album.name.clone(),
                album.artist_refs.clone(),
                Some(Press::Album(album.id.clone())),
            ),
            Hit::Playlist(list) => (
                Kind::Playlist,
                list.name.clone(),
                Vec::new(),
                Some(Press::Playlist(list.id.clone())),
            ),
        };

        let me = cx.entity().downgrade();
        let card = Card::new("best", title)
            .art(theme.metrics.cover * 0.45)
            .cover(cover(hit).clone())
            .when(matches!(kind, Kind::Artist), Card::circle)
            .eyebrow(noun(kind))
            .size(Text::Title)
            .weight(FontWeight::BOLD)
            .bare_meta(
                cells::artist_links(
                    "best-artist-link",
                    artists,
                    meta(hit, false),
                    theme.muted_foreground,
                )
                .text_size(theme.text(Text::Small))
                .truncate(),
            )
            .flat()
            .gap_4()
            .p_3()
            .bg(theme.secondary)
            .when_some(self.transport(hit, &me, cx), |card, (playing, play)| {
                card.play(playing, play)
            })
            .when_some(hit.pin(), Pinnable::pin)
            .when_some(target, |card, target| card.press(pressed(target, &me)))
            .when_some(HitMenu::of(hit), |card, target| {
                card.on_mouse_down(MouseButton::Right, menu(target, &me))
            })
            .into_any_element();

        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_2()
                .child(eyebrow(t!("search-best-match"), cx).pb_1())
                .child(card)
                .into_any_element(),
        )
    }

    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let reason = self.search.read(cx).error()?.to_owned();

        Some(
            div()
                .flex_none()
                .text_color(cx.theme().danger)
                .child(reason)
                .into_any_element(),
        )
    }

    fn shell(
        &self,
        title: SharedString,
        body: AnyElement,
        gutter: Pixels,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap_1()
            .child(div().pl(gutter).child(eyebrow(title, cx).pb_1()))
            .child(body)
            .into_any_element()
    }

    fn column(&self, kind: Kind, window: &Window, cx: &Context<Self>) -> AnyElement {
        let (title, only) = match kind {
            Kind::Song => (t!("search-songs"), SONGS),
            Kind::Artist => (t!("search-artists"), ARTISTS),
            Kind::Album | Kind::Playlist => (t!("search-albums-playlists"), RELEASES),
        };
        let body = self.deck(only, RAIL, None, window, cx);

        self.shell(title, body, RAIL, cx)
    }

    fn browse(&self, gutter: Pixels, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let error = self.genres.read(cx).error().map(str::to_owned);
        let found = self.genres.read(cx).genres();
        let theme = *cx.theme();
        let pad = theme.metrics.inset;
        let width = cells::content_width(window, pad * 2., cx);
        let plates = shelves::grid("genre", found, width, window, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(div().px(gutter).child(eyebrow(t!("search-browse"), cx)))
            .children(error.map(|error| {
                div()
                    .flex_none()
                    .px(gutter)
                    .text_color(theme.danger)
                    .child(SharedString::from(error))
            }))
            .child(
                Scroller::new("search-browse", &self.browsing)
                    .px(gutter)
                    .pb(pad)
                    .child(plates),
            )
            .into_any_element()
    }

    fn everything(&self, gutter: Pixels, window: &Window, cx: &Context<Self>) -> AnyElement {
        let lead = div()
            .flex()
            .flex_col()
            .gap_6()
            .children(self.best(cx))
            .child(eyebrow(t!("search-results"), cx).pb_1())
            .into_any_element();

        self.deck(&Kind::ALL, gutter, Some(lead), window, cx)
    }

    fn seats(&self, only: &[Kind], cx: &App) -> Vec<(usize, usize)> {
        let mut taken = [0; Kind::ALL.len()];

        self.search
            .read(cx)
            .hits()
            .iter()
            .enumerate()
            .filter_map(|(at, hit)| {
                let kind = hit.kind();
                let place = taken[seat(kind)];
                taken[seat(kind)] += 1;
                only.contains(&kind).then_some((at, place))
            })
            .collect()
    }

    fn deck(
        &self,
        only: &[Kind],
        gutter: Pixels,
        lead: Option<AnyElement>,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (id, bar) = match only {
            [Kind::Song] => ("search-songs", &self.songs),
            [Kind::Artist] => ("search-artists", &self.artists),
            [Kind::Album, Kind::Playlist] => ("search-albums", &self.albums),
            _ => ("search-all", &self.mixed),
        };
        let seats = self.seats(only, cx);
        if seats.is_empty() {
            let empty = vacant(t!("search-no-matches"), cx).flex_none();
            return match lead {
                Some(lead) => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px(gutter)
                    .child(lead)
                    .child(empty)
                    .into_any_element(),
                None => empty.into_any_element(),
            };
        }

        let compact = only.len() > 1;
        let column = column_of(only);
        let theme = *cx.theme();
        let row = snapped(theme.metrics.list_row, window);
        let scroll = bar.read(cx).scroll().clone();
        let me = cx.entity().downgrade();
        let measured = self.lead.clone();
        let tracked = scroll.clone();
        let deck = Deck::new(format!("{id}-deck"))
            .rows((0..seats.len()).map(|_| row))
            .gap(theme.font_size * ROW_GAP)
            .when(lead.is_some(), |deck| {
                deck.on_measure(move |top, _, _| {
                    let content = tracked.bounds().origin.y - scrolled(&tracked);
                    measured.set(top - content);
                })
            })
            .draw(move |index, _, cx| {
                let Some(view) = me.upgrade() else {
                    return div().into_any_element();
                };
                let Some(&(at, place)) = seats.get(index) else {
                    return div().into_any_element();
                };
                let this = view.read(cx);
                let Some(hit) = this.search.read(cx).hits().get(at) else {
                    return div().into_any_element();
                };

                this.row(
                    hit,
                    place,
                    compact,
                    this.cursor == Some((column, index)),
                    &view.downgrade(),
                    cx,
                )
            });

        div()
            .flex_1()
            .min_h_0()
            .child(
                Scroller::new(id, bar)
                    .px(gutter)
                    .pb(theme.metrics.inset)
                    .child(div().flex().flex_col().gap_1().children(lead).child(deck)),
            )
            .into_any_element()
    }
}

fn stacked(window: &Window, cx: &App) -> bool {
    !Chrome::room(window, cx).fits(Room::Wide)
}

fn columns(stacked: bool) -> usize {
    match stacked {
        true => 1,
        false => 3,
    }
}

fn kinds(column: usize, stacked: bool) -> &'static [Kind] {
    match (stacked, column) {
        (true, _) => &Kind::ALL,
        (false, 0) => SONGS,
        (false, 1) => ARTISTS,
        _ => RELEASES,
    }
}

fn column_of(only: &[Kind]) -> usize {
    match only {
        [Kind::Song] => 0,
        [Kind::Artist] => 1,
        [Kind::Album, Kind::Playlist] => 2,
        _ => 0,
    }
}

fn seat(kind: Kind) -> usize {
    match kind {
        Kind::Song => 0,
        Kind::Artist => 1,
        Kind::Album => 2,
        Kind::Playlist => 3,
    }
}

fn pressed(
    target: Press,
    me: &WeakEntity<SearchView>,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let me = me.clone();

    move |_, _, cx| {
        me.update(cx, |this, cx| match &target {
            Press::Song(track) => this
                .playback
                .update(cx, |playback, cx| playback.play_radio(track, cx)),
            Press::Artist(id) => navigate(Destination::Artist(id.clone().into()), cx),
            Press::Album(id) => navigate(Destination::Album(id.clone().into()), cx),
            Press::Playlist(id) => navigate(Destination::Playlist(id.clone().into()), cx),
        })
        .ok();
    }
}

fn menu(
    target: HitMenu,
    me: &WeakEntity<SearchView>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    let me = me.clone();

    move |event: &MouseDownEvent, window, cx| {
        window.prevent_default();
        me.update(cx, |this, cx| {
            this.track_menu.reset(cx);
            this.context_menu = Some((target.clone(), event.position));
            cx.notify();
        })
        .ok();
    }
}

fn album_of(hit: &AlbumHit, cx: &App) -> Album {
    Sonora::global(cx)
        .library
        .read(cx)
        .album(&hit.id)
        .cloned()
        .unwrap_or_else(|| Album {
            id: hit.id.clone(),
            name: hit.name.clone(),
            artists: hit.artists.clone(),
            artist_refs: hit.artist_refs.clone(),
            cover: hit.cover.clone(),
            cover_large: hit.cover.clone(),
            release_type: ReleaseType::Album,
            year: 0,
            track_count: 0,
            release_date: String::new(),
            label: String::new(),
            copyrights: Vec::new(),
            added_at: None,
        })
}

fn playlist_of(hit: &PlaylistHit, cx: &App) -> Playlist {
    Sonora::global(cx)
        .library
        .read(cx)
        .playlist(&hit.id)
        .cloned()
        .unwrap_or_else(|| Playlist {
            id: hit.id.clone(),
            name: hit.name.clone(),
            owner: hit.owner.clone(),
            owner_id: String::new(),
            owned: false,
            collaborative: false,
            blend: false,
            public: false,
            cover: hit.cover.clone(),
            track_count: 0,
            modified_at: None,
        })
}

fn artist_of(hit: &ArtistHit, cx: &App) -> SavedArtist {
    let id = hit.id.clone().unwrap_or_default();
    Sonora::global(cx)
        .library
        .read(cx)
        .artist(&id)
        .cloned()
        .unwrap_or_else(|| SavedArtist {
            id,
            name: hit.name.clone(),
            cover: hit.cover.clone(),
            added_at: None,
        })
}

fn cover(hit: &Hit) -> Option<String> {
    match hit {
        Hit::Song(track) => track.cover.clone(),
        Hit::Artist(artist) => artist.cover.clone(),
        Hit::Album(album) => album.cover.clone(),
        Hit::Playlist(list) => list.cover.clone(),
    }
}

fn meta(hit: &Hit, compact: bool) -> SharedString {
    match hit {
        Hit::Song(track) => SharedString::from(track.artists.clone()),
        Hit::Album(album) => SharedString::from(album.artists.clone()),
        Hit::Artist(artist) => match compact {
            true => noun(Kind::Artist),
            false => held(artist.saved),
        },
        Hit::Playlist(list) => match list.owner.is_empty() {
            true => noun(Kind::Playlist),
            false => SharedString::from(list.owner.clone()),
        },
    }
}

fn tag(kind: Kind) -> SharedString {
    let noun = noun(kind);
    t!("search-tag", kind = &noun)
}

fn held(saved: usize) -> SharedString {
    match saved {
        0 => noun(Kind::Artist),
        count => t!("search-saved", count = count),
    }
}

fn noun(kind: Kind) -> SharedString {
    match kind {
        Kind::Song => t!("kind-song"),
        Kind::Artist => t!("kind-artist"),
        Kind::Album => t!("kind-album"),
        Kind::Playlist => t!("kind-playlist"),
    }
}

impl Focusable for SearchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let pad = theme.metrics.inset;
        let width = cells::content_width(window, pad * 2., cx);
        let stacked = !Chrome::room(window, cx).fits(Room::Wide);
        let inset = match width > VAST {
            true => (width - VAST) / 2.,
            false => Pixels::ZERO,
        };
        let asked = !self.search.read(cx).query().trim().is_empty();
        let context_menu = self.context_menu.clone().map(|(target, position)| {
            let menu = match target {
                HitMenu::Song(track) => self.track_menu.for_track(&track, cx),
                HitMenu::Album(hit) => {
                    album_menu(album_of(&hit, cx), self.playback.clone(), false, cx)
                }
                HitMenu::Playlist(hit) => {
                    playlist_menu(playlist_of(&hit, cx), self.playback.clone(), false, cx)
                }
                HitMenu::Artist(hit) => {
                    artist_menu(artist_of(&hit, cx), self.playback.clone(), false, cx)
                }
            };

            Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
        });

        let gutter = pad + inset;
        let results = match (asked, stacked) {
            (false, _) => self.browse(gutter, window, cx),
            (true, true) => self.everything(gutter, window, cx),
            (true, false) => div()
                .flex()
                .flex_1()
                .min_h_0()
                .px(gutter)
                .child(self.column(Kind::Song, window, cx))
                .child(Separator::vertical())
                .child(self.column(Kind::Artist, window, cx))
                .child(Separator::vertical())
                .child(self.column(Kind::Album, window, cx))
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_6()
            .pt(pad)
            .key_context(SEARCH_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::activate_hit))
            .on_action(cx.listener(Self::deselect_hit))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .px(gutter)
                    .child(self.input.clone()),
            )
            .children(
                self.failure(cx)
                    .map(|failure| div().px(gutter).child(failure)),
            )
            .when(!stacked, |this| {
                this.children(self.best(cx).map(|best| div().px(gutter).child(best)))
            })
            .child(results)
            .when_some(context_menu, |this, menu| this.child(menu))
    }
}
