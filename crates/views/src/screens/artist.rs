use std::cell::Cell;
use std::rc::Rc;

use gpui::accesskit::{Role, Toggled};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Pixels, Point, Render, ScrollHandle,
    ScrollWheelEvent, SharedString, WeakEntity, Window, div, linear_color_stop, linear_gradient,
    point, px, relative,
};

use crate::chrome::Chrome;
use crate::shared::cells;
use i18n::t;
use music::{SavedArtist, Track};
use state::{AppSettings, ArtistDetail, Origin, Playback, Sonora};
use ui::ActiveTheme as _;
use ui::Listing as _;
use ui::{
    Artwork, Button, Card, Deck, MIN_CONTENT, Mode, Pending, Picker, Pin, PinKind, Popovers, Popup,
    Scrollbar, Scroller, TableDelegate, TableEvent, TableState, Text, effects, scrolled, snapped,
    table,
};

use crate::chrome::tools;
use crate::chrome::{Toolbar, Tooled};
use crate::shared::about::{AboutArtist, about_modal};
use crate::shared::album_grid::CardGrid;
use crate::shared::cards::{self, ReleaseFilter};
use crate::shared::confirm::Confirm;
use crate::shared::hero::{HeroMetaStrip, HeroPlayButton, PageHero};
use crate::shared::menus::{ItemMenu, artist_menu};
use crate::shared::page;
use crate::shared::picks::{Picks, Shape};
use crate::shared::shelves::{Rail, RailSpec};
use crate::shared::tracks::{PlaybackStatus, TrackSource, Tracks, drop_picked, playback_status};
use crate::shared::trouble;

const SECTION: &str = "artist";
const RELEASE_ROWS: usize = 2;
const LISTED: usize = 5;
const LISTED_MAX: usize = 10;
/// The width of the `border_1` around the popular table, which pushes its rows and head below
/// the box the page measures the scroll from.
const TABLE_BORDER: Pixels = px(1.);
/// The least and the most room the banner adds above the hero, as shares of the cover's edge.
/// In between, the banner box keeps `BANNER_BOX` as its aspect, so a narrow page lifts the
/// hero less.
const BANNER_RISE_LEAST: f32 = 0.35;
const BANNER_RISE_MOST: f32 = 1.;
const BANNER_BOX: f32 = 2.7;
/// The room above the hero when only a blurred portrait stands in for the banner. The wash has
/// no proportions to keep, so it stays this tall at any width.
const BANNER_RISE_WASH: f32 = 0.6;
/// The aspect the banner picture is drawn at. Spotify's banners are this wide, and the box is
/// a little wider still, so the picture loses a strip at its top and bottom but never its sides.
const BANNER_ASPECT: f32 = 2.33;
/// How much of what the banner box cannot show is cut off above rather than below. Banners keep
/// their subject a little above the middle.
const BANNER_ANCHOR: f32 = 0.35;
/// The tallest the banner picture grows, as a share of the banner's height. On a page wide enough
/// to grow it further it keeps this size in the middle, and both its sides fade into a washed out
/// copy over the room beside it, up to `BANNER_SIDE_FADE` of its width, so faces are never blown
/// up past the box.
const BANNER_OVERFLOW: f32 = 1.25;
const BANNER_SIDE_FADE: f32 = 0.2;
/// How much wider than the page the banner is drawn, so it still reaches both edges while the
/// page width it was measured from lags a resize by a frame.
const BANNER_SLACK: f32 = 0.06;
/// The blurs the banner steps through on its way down, each with the share of the stretch from
/// the hero's top to the banner's bottom at which it has fully taken over from the one above.
/// Everything above the hero stays sharp, wherever the page width puts it, and the blur grows
/// slowly at first so it has no visible start.
const BANNER_BLURS: [(f32, Pixels); 7] = [
    (0.25, px(1.)),
    (0.375, px(2.5)),
    (0.5, px(4.5)),
    (0.625, px(7.5)),
    (0.75, px(11.5)),
    (0.875, px(17.)),
    (1., px(24.)),
];
/// How far above its own mark each banner blur starts fading in, as a share of the same
/// stretch. It is longer than the gap between marks, so each blur is still fading in when the
/// next starts.
const BANNER_RAMP: f32 = 0.25;
/// How much of the page colour lies over the whole banner, so it sits back behind the page.
const BANNER_DIM: f32 = 0.3;
/// The shade that deepens toward the banner's bottom on top of `BANNER_DIM`: where it starts, as
/// a share of the way down to the hero's top, and how much of the page colour it reaches at the
/// bottom. The banner also fades out from the hero's top and is gone at its bottom.
const BANNER_SHADE_FROM: f32 = 0.5;
const BANNER_SHADE: f32 = 0.85;
/// The blur over a portrait standing in for a missing banner, enough to leave only its colours.
const BANNER_WASH: Pixels = px(48.);

struct ArtistTracks {
    detail: Entity<ArtistDetail>,
    shown: Rc<Cell<usize>>,
}

impl Tracks for ArtistTracks {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        let tracks = self.detail.read(cx).tracks();
        &tracks[..self.shown.get().min(tracks.len())]
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.detail.read(cx).is_loading()
    }

    /// As many rows as the list shows before it is expanded.
    fn pending(&self, cx: &App) -> Option<Pending> {
        self.is_loading(cx).then_some(Pending::Rows(LISTED))
    }
}

pub(crate) struct ArtistView {
    detail: Entity<ArtistDetail>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    artist_id: Option<String>,
    release_filter: ReleaseFilter,
    releases_expanded: bool,
    release_padding: Pixels,
    release_padding_offset: Pixels,
    width: Pixels,
    scrollbar: Entity<Scrollbar>,
    about_bar: Entity<Scrollbar>,
    about_open: bool,
    table: Entity<TableState<TrackSource>>,
    shown: Rc<Cell<usize>>,
    mode: Mode,
    popular: Rc<Vec<Track>>,
    popular_page: usize,
    popular_columns: usize,
    track_menu: ItemMenu,
    track_context: Option<(usize, Point<Pixels>)>,
    settings: Entity<AppSettings>,
    toolbar: Entity<Toolbar>,
    me: WeakEntity<Self>,
    popovers: Popovers,
    /// Where the releases grid's leading edge landed last frame, in window coordinates, or
    /// none until it has been laid out once. `hold_releases` needs it to tell how deep the
    /// page is scrolled into the grid.
    release_lead: Rc<Cell<Option<Pixels>>>,
    /// The packing the grid was laid out with last frame, so a change of width can hold its
    /// place rather than slide the cards under the user.
    release_columns: usize,
    release_tile: Pixels,
    /// The recommendation rails under the releases, in page order: appears-on.
    rails: Vec<Rail>,
}

impl ArtistView {
    pub(crate) fn new(
        detail: Entity<ArtistDetail>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let width = MIN_CONTENT;
        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
        let settings = Sonora::global(cx).settings.clone();
        let saved = settings.read(cx).table(SECTION);
        let sorting = settings.read(cx).sorting(SECTION);
        let mode = settings.read(cx).view_or(SECTION, Mode::List);
        let columns = crate::shared::tracks::artist_columns(
            Sonora::global(cx)
                .session
                .read(cx)
                .capabilities()
                .playcounts,
        );
        let scroll = scrollbar.read(cx).scroll().clone();
        let shown = Rc::new(Cell::new(LISTED));
        let table = cx.new(|cx| {
            let menu_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
            crate::shared::tracks::follow_downloads(cx);
            let source = TrackSource::new(
                columns,
                ArtistTracks {
                    detail: detail.clone(),
                    shown: shown.clone(),
                },
                playback.clone(),
                menu_scrollbar,
                cx,
            )
            .from({
                let detail = detail.clone();
                move |cx: &App| {
                    let detail = detail.read(cx);
                    let name = detail.artist()?.name.clone();
                    Some(Origin::artist(detail.id()?).named(name))
                }
            })
            .with_liked(Sonora::global(cx).library.clone());
            let source = source.table(cx.weak_entity());
            let mut delegate = TableDelegate::new(source, width, cx);
            delegate.set_layout(saved, cx);
            delegate.set_sorting(sorting.flatten(), cx);
            TableState::new(delegate, cx).follow(scroll)
        });

        cx.subscribe(&detail, |_, _, event, cx| {
            let state::ArtistDetailEvent::Gone(id) = event;
            if router::trail(cx).read(cx).current()
                == router::Destination::Artist(id.clone().into())
            {
                router::replace(router::Destination::Local(router::LibraryTab::Artists), cx);
            }
        })
        .detach();

        cx.observe(&detail, |this, detail, cx| {
            let artist_id = detail.read(cx).id().map(str::to_owned);
            if this.artist_id != artist_id {
                this.artist_id = artist_id;
                this.release_filter = ReleaseFilter::All;
                this.releases_expanded = false;
                this.release_padding = Pixels::ZERO;
                this.release_padding_offset = Pixels::ZERO;
                this.about_open = false;
                this.rails.clear();
                this.shown.set(LISTED);
                this.scrollbar.update(cx, |bar, cx| {
                    bar.set_max_offset(None, cx);
                    bar.scroll().set_offset(gpui::Point::default());
                });
            }
            this.popular = Rc::new(detail.read(cx).tracks().to_vec());
            this.popular_page = 0;
            this.track_menu.reset(cx);
            this.track_context = None;
            this.rebuild(cx);
            cx.notify();
        })
        .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |this, _, cx| {
            this.table.update(cx, |table, cx| table.refresh(cx));
            cx.notify();
        })
        .detach();
        let current_playback = playback_status(&playback, cx);
        let artist_id = detail.read(cx).id().map(str::to_owned);
        cx.observe(&playback, |this, playback, cx| {
            let current = playback_status(&playback, cx);
            if this.playback_status == current {
                return;
            }
            this.playback_status = current;
            this.table.update(cx, |table, cx| table.refresh(cx));
            cx.notify();
        })
        .detach();
        cx.subscribe(&table, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) => {
                page::play(&this.table, &this.playback, *display, cx)
            }
            TableEvent::Activated(display) => {
                page::play_or_toggle(&this.table, &this.playback, *display, cx)
            }
            TableEvent::Removed => drop_picked(&this.table, cx),
            _ => this.persist(cx),
        })
        .detach();

        let me = cx.entity();
        let toolbar = Toolbar::tooled(&me, cx);

        Self {
            popular: Rc::new(detail.read(cx).tracks().to_vec()),
            detail,
            playback,
            playback_status: current_playback,
            artist_id,
            release_filter: ReleaseFilter::All,
            releases_expanded: false,
            release_padding: Pixels::ZERO,
            release_padding_offset: Pixels::ZERO,
            width,
            scrollbar,
            about_bar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
            about_open: false,
            table,
            shown,
            mode,
            popular_page: 0,
            popular_columns: 0,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            track_context: None,
            settings,
            toolbar,
            me: me.downgrade(),
            popovers: Popovers::default(),
            release_lead: Rc::new(Cell::new(None)),
            release_columns: 0,
            release_tile: Pixels::ZERO,
            rails: Vec::new(),
        }
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        page::store(
            &self.settings.clone(),
            &self.table.clone(),
            SECTION,
            SECTION,
            cx,
        );
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.rebuild(cx);
        });
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == Mode::List {
            self.table.clone().set_width(self.width, cx);
        }
        self.track_context = None;
        self.settings
            .update(cx, |settings, cx| settings.set_view(SECTION, mode, cx));
        cx.notify();
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let artist = self.detail.read(cx).artist();
        let title = artist
            .map(|artist| SharedString::from(artist.name.clone()))
            .unwrap_or_default();
        let listeners = artist
            .and_then(|artist| artist.monthly_listeners)
            .map(|count| {
                let value = cells::count(count);
                t!("artist-monthly-listeners", count = count, value = &value)
            });
        let overflow = self.saved_artist(cx).map(|artist| {
            Picker::icon("artist-overflow", &self.popovers, "icons/ellipsis.svg")
                .tooltip("common-more")
                .large()
                .left()
                .menu(artist_menu(artist, self.playback.clone(), true, cx))
        });
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                HeroPlayButton::new(
                    "play-artist",
                    t!("artist-play"),
                    self.popular.as_ref().clone(),
                    self.playback.clone(),
                )
                .from(self.playing_from(cx)),
            )
            .child(
                HeroPlayButton::shuffle(
                    "shuffle-artist",
                    self.popular.as_ref().clone(),
                    self.playback.clone(),
                )
                .from(self.playing_from(cx)),
            )
            .children(self.favorite_button(cx))
            .children(overflow);

        let cover = artist.and_then(|artist| artist.cover_large.clone());
        let pin = self
            .detail
            .read(cx)
            .id()
            .map(|id| Pin::new(PinKind::Artist, id, title.clone()).cover(cover.clone()));

        PageHero::new("artist-hero", title)
            .pending(artist.is_none() && self.detail.read(cx).is_loading())
            .pin(pin)
            .cover(cover)
            .when_some(listeners, |hero, listeners| {
                hero.meta(HeroMetaStrip::new().text(listeners))
            })
            .actions(actions)
            .circle()
            .into_any_element()
    }

    /// The artist's banner behind the hero, `height` tall and bled past the page's `inset` to
    /// its edges, with the hero's top `hero` below its own. It is dimmed throughout and stays sharp
    /// above the hero, then blurs, darkens and fades harder until it is gone at its bottom.
    /// Without a banner the portrait stands in, blurred to a wash of its colours so it does not
    /// repeat the cover beside it. None when the artist has neither, or has only a portrait
    /// while blurs are off.
    fn banner(&self, inset: Pixels, height: Pixels, hero: Pixels, cx: &App) -> Option<AnyElement> {
        let artist = self.detail.read(cx).artist()?;
        let (url, wash) = match &artist.banner {
            Some(banner) => (banner.clone(), None),
            None if effects() => (artist.cover_large.clone()?, Some(BANNER_WASH)),
            None => return None,
        };
        // The box stretches to the page's edges at once, while the width measured here arrives a
        // frame later, so a window being resized would show the box's edge past the picture.
        // Everything is drawn a little wider than the box, centred in it, and the box clips the
        // rest.
        let width = (self.width + inset * 2.) * (1. + BANNER_SLACK);
        let theme = cx.theme();
        let shade = div()
            .absolute()
            .inset_0()
            .bg(theme.background.opacity(BANNER_DIM))
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(
                    theme.background.opacity(0.),
                    hero / height * BANNER_SHADE_FROM,
                ),
                linear_color_stop(theme.background.opacity(BANNER_SHADE), 1.),
            )));
        // Every copy reaches past the box by twice the widest blur, so no blur pulls in the
        // transparent edge, and they all share one geometry so they line up.
        let bleed = wash.unwrap_or(BANNER_BLURS[BANNER_BLURS.len() - 1].1) * 2.;
        let natural = width / BANNER_ASPECT;
        let tall = match wash {
            Some(_) => natural,
            None => natural.min(height * BANNER_OVERFLOW),
        }
        .max(height);
        let across = (tall * BANNER_ASPECT).min(width);
        // The fade grows from nothing with the room either side, so a page whose width sits near
        // the point where the picture stops filling it does not flick the fade on and off.
        let side = bleed + ((width - across) / 2.).min(across * BANNER_SIDE_FADE);
        let lifted = (tall - height) * BANNER_ANCHOR;
        // The same picture lies under the banner across the whole box, washed out like a missing
        // banner's portrait, so a picture narrower than the page has its colours either side.
        let spread = BANNER_WASH * 2.;
        let backdrop = Artwork::new(Some(url.clone()))
            .size(width + spread * 2.)
            .wide(height + spread * 2.)
            .overlay()
            .absolute()
            .left(relative(0.5))
            .ml(-(width / 2. + spread))
            .top(-spread)
            .blur(BANNER_WASH);
        let picture = |top: Pixels| {
            Artwork::new(Some(url.clone()))
                .size(across + bleed * 2.)
                .wide(tall + bleed * 2.)
                .overlay()
                .absolute()
                .left(relative(0.5))
                .ml(-(across / 2. + bleed))
                .top(-top - lifted - bleed)
                .fade_sides(side, side)
        };
        let below = height - hero;
        let steps = BANNER_BLURS.iter().map(|&(mark, blur)| {
            let top = hero + below * (mark - BANNER_RAMP);
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(top)
                .bottom_0()
                .overflow_hidden()
                .fade_edges(below * BANNER_RAMP, Pixels::ZERO)
                .child(picture(top).blur(blur))
        });

        Some(
            div()
                .absolute()
                .top(-inset)
                .left(-inset)
                .right(-inset)
                .h(height)
                .overflow_hidden()
                .fade_edges(Pixels::ZERO, below)
                .map(|this| match wash {
                    Some(wash) => this.child(picture(Pixels::ZERO).blur(wash)),
                    None => this
                        .when(effects(), |this| this.child(backdrop))
                        .child(picture(Pixels::ZERO))
                        .when(effects(), |this| this.children(steps)),
                })
                .child(shade)
                .into_any_element(),
        )
    }

    fn saved_artist(&self, cx: &App) -> Option<SavedArtist> {
        let detail = self.detail.read(cx);
        let artist = detail.artist()?;

        Some(SavedArtist {
            id: detail.id()?.to_owned(),
            name: artist.name.clone(),
            cover: artist.cover_large.clone(),
            added_at: None,
        })
    }

    fn favorite_button(&self, cx: &App) -> Option<Button> {
        let theme = *cx.theme();
        // A provider with no followed artists has nothing for this to toggle.
        if !Sonora::global(cx)
            .session
            .read(cx)
            .capabilities()
            .follow_artists
        {
            return None;
        }
        let library = Sonora::global(cx).library.clone();
        let target = self.saved_artist(cx)?;
        let saved = library.read(cx).saved_artist(&target.id);

        let heart = Button::new("artist-toggle-library")
            .outline()
            .icon(match saved {
                true => "icons/heart-filled.svg",
                false => "icons/heart.svg",
            })
            .tooltip(match saved {
                true => "menu-remove-from-library",
                false => "menu-add-to-library",
            })
            .disabled(library.read(cx).pending_artist(&target.id));

        Some(
            match saved {
                true => heart.tint(theme.primary),
                false => heart,
            }
            .on_click(move |_, _, cx| {
                match saved && !Confirm::unstarring(&target.id, cx) {
                    true => Confirm::artists(vec![target.clone()], cx),
                    false => {
                        library.update(cx, |library, cx| library.heart_artist(target.clone(), cx));
                    }
                }
            }),
        )
    }

    fn release_height(
        &self,
        filter: ReleaseFilter,
        expanded: bool,
        columns: usize,
        window: &Window,
        cx: &App,
    ) -> Pixels {
        let count = self
            .detail
            .read(cx)
            .albums()
            .iter()
            .filter(|album| filter.matches(album.release_type))
            .count();
        let visible = match expanded {
            true => count,
            false => count.min(columns * RELEASE_ROWS),
        };
        let rows = visible.div_ceil(columns.max(1));
        if rows == 0 {
            return Pixels::ZERO;
        }

        let grid = CardGrid::layout(self.width);
        let card = Card::tile_height(grid.card, window, cx);
        card * rows as f32 + release_gap(window) * rows.saturating_sub(1) as f32
    }

    fn set_release_view(
        &mut self,
        filter: ReleaseFilter,
        expanded: bool,
        columns: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let old = self.release_height(
            self.release_filter,
            self.releases_expanded,
            columns,
            window,
            cx,
        );
        let new = self.release_height(filter, expanded, columns, window, cx);
        let offset = scrolled(self.scrollbar.read(cx).scroll());
        self.release_padding = match old > new {
            true => self.release_padding + (old - new).min(offset),
            false => (self.release_padding - (new - old)).max(Pixels::ZERO),
        };
        self.release_padding_offset = offset;
        self.release_filter = filter;
        self.releases_expanded = expanded;
        cx.notify();
    }

    fn settle_release_padding(&mut self, offset: Pixels) {
        if offset < self.release_padding_offset {
            self.release_padding =
                (self.release_padding - (self.release_padding_offset - offset)).max(Pixels::ZERO);
        }
        self.release_padding_offset = offset;
    }

    fn release_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let upward = event.delta.pixel_delta(window.line_height()).y;
        if upward <= Pixels::ZERO || self.release_padding <= Pixels::ZERO {
            return;
        }
        cx.notify();
    }

    /// How many releases the grid lists at a given packing: every one that passes the filter
    /// while it is expanded, and the first two rows of them while it is not.
    fn shown_releases(&self, columns: usize, cx: &App) -> usize {
        let count = self
            .detail
            .read(cx)
            .albums()
            .iter()
            .filter(|album| self.release_filter.matches(album.release_type))
            .count();

        match self.releases_expanded {
            true => count,
            false => count.min(columns * RELEASE_ROWS),
        }
    }

    /// Holds the releases grid still when the page changes width. A narrower page packs the
    /// cards into fewer columns and more rows, which slides whatever the user was looking at
    /// down the page; this moves the scroll by as much as the row under the top edge moved,
    /// so the same releases stay where they were. The row heights are uniform here, so the
    /// anchor is the first card of that row and nothing but the packing has to be remembered.
    fn hold_releases(&mut self, window: &Window, cx: &mut Context<Self>) {
        let grid = CardGrid::layout(self.width);
        let columns = grid.columns.max(1);
        let tile = Card::tile_height(grid.card, window, cx);
        let (was_columns, was_tile) = (self.release_columns, self.release_tile);
        self.release_columns = columns;
        self.release_tile = tile;
        if was_columns == 0 || (was_columns == columns && (was_tile - tile).abs() < px(0.5)) {
            return;
        }
        let Some(lead) = self.release_lead.get() else {
            return;
        };

        let scroll = self.scrollbar.read(cx).scroll().clone();
        let depth = scroll.bounds().origin.y - lead;
        if depth <= Pixels::ZERO {
            return;
        }
        let gap = release_gap(window);
        let before = self.shown_releases(was_columns, cx).div_ceil(was_columns);
        let (row, into) = Deck::at(&vec![was_tile; before], gap, depth);
        let share = match was_tile > Pixels::ZERO {
            true => into / was_tile,
            false => 0.,
        };

        let after = self.shown_releases(columns, cx).div_ceil(columns);
        let landed = (row * was_columns / columns).min(after.saturating_sub(1));
        let top = Deck::tops(&vec![tile; after], gap)
            .get(landed)
            .copied()
            .unwrap_or_default();
        let moved = top + tile * share - depth;
        if moved.abs() < px(0.5) {
            return;
        }

        let at = (-scroll.offset().y + moved).max(Pixels::ZERO);
        scroll.set_offset(point(Pixels::ZERO, -at));
    }

    /// The Releases grid. The rows go into a `Deck`, so an artist with a long discography
    /// builds only the rows on screen once the list is expanded, and the section still takes
    /// the height `release_height` reports, which is what the collapse padding is measured
    /// against.
    fn releases(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = *cx.theme();
        let detail = self.detail.read(cx);
        let albums = detail.albums();
        // A discography still on its way reads as loading only while nothing of it is up.
        let loading = detail.is_loading() || (albums.is_empty() && detail.is_filling());
        if albums.is_empty() && !loading {
            return None;
        }
        let local = detail.id().is_some_and(music::is_local_id);
        let filters = match loading {
            true => Vec::new(),
            false => cards::release_filters(local, albums.iter().map(|album| album.release_type)),
        };

        let grid = CardGrid::layout(self.width);
        let gap = release_gap(window);
        let releases = match loading {
            true => div()
                .flex()
                .flex_col()
                .gap(gap)
                .children((0..RELEASE_ROWS).map(|row| {
                    CardGrid::new(self.width).children((0..grid.columns).map(move |column| {
                        let index = row * grid.columns + column;
                        Card::skeleton(("artist-release-skeleton", index))
                            .tile(grid.card)
                            .into_any_element()
                    }))
                }))
                .into_any_element(),
            false => {
                let columns = grid.columns.max(1);
                let shown: Rc<[usize]> = albums
                    .iter()
                    .enumerate()
                    .filter(|(_, album)| self.release_filter.matches(album.release_type))
                    .map(|(index, _)| index)
                    .take(match self.releases_expanded {
                        true => usize::MAX,
                        false => columns * RELEASE_ROWS,
                    })
                    .collect();
                let rows = shown.len().div_ceil(columns);
                let total = shown.len();
                let height = Card::tile_height(grid.card, window, cx);
                let width = self.width;
                let card = grid.card;
                let opened = cx.entity().downgrade();
                let lead = self.release_lead.clone();

                Deck::new("artist-releases-deck")
                    .rows(std::iter::repeat_n(height, rows))
                    .gap(gap)
                    .on_measure(move |top, _, _| lead.set(Some(top)))
                    .draw(move |row, _, cx| {
                        let Some(view) = opened.upgrade() else {
                            return div().into_any_element();
                        };
                        let start = row * columns;
                        let end = (start + columns).min(shown.len());
                        // The cards are built straight off the shelf's albums. A row is
                        // rebuilt on every frame a scroll asks for, so cloning a release,
                        // its credits and all of its strings to get here would be the whole
                        // cost of scrolling a long discography.
                        let held = view.read(cx);
                        let listed = held.detail.read(cx).albums();
                        let cards = (start..end).filter_map(|seat| {
                            let album = listed.get(shown[seat])?;
                            let id = SharedString::from(format!("artist-release-{}", album.id));

                            Some(
                                cards::album_card(id, album, &held.playback, cx)
                                    .tile(card)
                                    .flat()
                                    .aria_position_in_set(seat + 1)
                                    .aria_size_of_set(total)
                                    .into_any_element(),
                            )
                        });

                        CardGrid::new(width).children(cards).into_any_element()
                    })
                    .into_any_element()
            }
        };

        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_6()
                .child({
                    let label = t!("artist-releases");
                    div()
                        .id("artist-releases-heading")
                        .role(Role::Heading)
                        .aria_level(2)
                        .aria_label(label.clone())
                        .text_size(theme.text(Text::Title))
                        .font_weight(FontWeight::BOLD)
                        .child(label)
                })
                .when(!filters.is_empty(), |this| {
                    this.child(
                        div()
                            .flex()
                            .gap_1()
                            .children(filters.into_iter().map(|filter| {
                                Button::new(filter.id())
                                    .label(filter.label())
                                    .small()
                                    .outline()
                                    .selected(self.release_filter == filter)
                                    .aria_toggled(Toggled::from(self.release_filter == filter))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.release_filter == filter {
                                            return;
                                        }
                                        this.set_release_view(
                                            filter,
                                            false,
                                            grid.columns,
                                            window,
                                            cx,
                                        );
                                    }))
                            })),
                    )
                })
                .child(releases)
                .children(self.release_toggle(grid.columns, cx))
                .into_any_element(),
        )
    }

    fn release_toggle(&self, columns: usize, cx: &mut Context<Self>) -> Option<Button> {
        let count = self
            .detail
            .read(cx)
            .albums()
            .iter()
            .filter(|album| self.release_filter.matches(album.release_type))
            .count();
        if count <= columns * RELEASE_ROWS {
            return None;
        }
        let expanded = self.releases_expanded;

        Some(
            Button::new("artist-releases-more")
                .label(match expanded {
                    true => t!("artist-releases-less"),
                    false => t!("artist-releases-more"),
                })
                .trailing(chevron(expanded))
                .small()
                .ghost()
                .aria_expanded(expanded)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_release_view(this.release_filter, !expanded, columns, window, cx);
                })),
        )
    }

    /// The recommendation rail under the releases: what the artist guests on. A rail
    /// still on its way reads as skeletons only while nothing of it is up.
    fn recommended(
        &self,
        window: &Window,
        notify: &Rc<dyn Fn(&mut App)>,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let grid = CardGrid::layout(self.width);
        let card = grid.card;
        let columns = grid.columns.max(1);
        let detail = self.detail.read(cx);
        let filling = detail.is_filling();
        let appears = detail.appears_on().len();

        vec![match appears {
            0 if filling => Some(Rail::pending(
                t!("artist-appears-on"),
                card,
                columns,
                window,
                cx,
            )),
            0 => None,
            _ => {
                let opened = self.me.clone();
                Some(self.rails[0].render(
                    RailSpec {
                        tag: "artist-appears",
                        place: 0,
                        title: t!("artist-appears-on"),
                        count: appears,
                        tile: card,
                        columns,
                        tabs: None,
                    },
                    window,
                    cx,
                    notify,
                    move |index, _, cx| {
                        let Some(view) = opened.upgrade() else {
                            return div().into_any_element();
                        };
                        let held = view.read(cx);
                        let detail = held.detail.read(cx);
                        let Some(album) = detail.appears_on().get(index) else {
                            return div().into_any_element();
                        };
                        let id = SharedString::from(format!("artist-appears-{}", album.id));
                        cards::album_card(id, album, &held.playback, cx)
                            .tile(card)
                            .flat()
                            .aria_position_in_set(index + 1)
                            .aria_size_of_set(appears)
                            .into_any_element()
                    },
                ))
            }
        }]
        .into_iter()
        .flatten()
        .map(|rail| div().pt_6().child(rail).into_any_element())
        .collect()
    }

    fn listed(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let expanded = self.shown.get() > LISTED;
        let more = (self.popular.len() > LISTED).then(|| {
            Button::new("artist-popular-more")
                .label(match expanded {
                    true => t!("artist-popular-less"),
                    false => t!("artist-popular-more"),
                })
                .trailing(chevron(expanded))
                .small()
                .ghost()
                .aria_expanded(expanded)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shown.set(match expanded {
                        true => LISTED,
                        false => LISTED_MAX,
                    });
                    this.rebuild(cx);
                    cx.notify();
                }))
        });

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                table(&self.table)
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border),
            )
            .children(more)
            .into_any_element()
    }

    fn playing_from(&self, cx: &App) -> Option<Origin> {
        let detail = self.detail.read(cx);
        let name = detail.artist()?.name.clone();

        Some(Origin::artist(detail.id()?).named(name))
    }

    fn popular(&self, cx: &mut Context<Self>) -> AnyElement {
        let tracks = self.popular.clone();
        let from = self.playing_from(cx);
        let pages = Shape::new(self.width, tracks.len()).pages;
        let queued = tracks.clone();
        let playback = self.playback.clone();
        let opened = cx.entity().downgrade();

        Picks::new(
            "artist-popular",
            tracks,
            self.playback.clone(),
            self.width,
            self.popular_page,
        )
        .title("artist-popular")
        .eyebrow(i18n::lookup("artist-popular-eyebrow", None))
        .vacancy("artist-popular-empty")
        .detailed()
        .loading(self.detail.read(cx).is_loading())
        .on_previous(cx.listener(|this, _, _, cx| {
            this.popular_page = this.popular_page.saturating_sub(1);
            this.track_context = None;
            cx.notify();
        }))
        .on_next(cx.listener(move |this, _, _, cx| {
            this.popular_page = (this.popular_page + 1).min(pages.saturating_sub(1));
            this.track_context = None;
            cx.notify();
        }))
        .on_context_menu(move |place, event, _, cx| {
            let Some(view) = opened.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| {
                this.track_menu.reset(cx);
                this.track_context = Some((place, event.position));
                cx.notify();
            });
        })
        .on_start(move |place, cx| {
            playback.update(cx, |playback, cx| {
                playback.start(queued.as_ref().clone(), place, from.clone(), cx)
            });
        })
        .into_any_element()
    }

    fn about(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let artist = self.detail.read(cx).artist()?;
        let card = AboutArtist::new("artist-about", artist.name.clone())
            .cover(artist.cover_large.clone())
            .biography(artist.biography.clone())
            .on_open(cx.listener(|this, _, _, cx| {
                this.about_open = true;
                cx.notify();
            }));

        Some(div().pt_6().child(card).into_any_element())
    }

    /// Closes the About the artist dialog when it is open, and says whether it was.
    pub(crate) fn escape(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.about_open {
            return false;
        }
        self.about_open = false;
        cx.notify();
        true
    }

    fn about_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.about_open {
            return None;
        }
        let artist = self.detail.read(cx).artist()?;

        Some(
            about_modal(
                artist.name.clone().into(),
                artist.biography.clone(),
                None,
                &self.about_bar,
                cx,
            )
            .action(
                Button::new("artist-about-close")
                    .label(t!("common-dismiss"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.about_open = false;
                        cx.notify();
                    })),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| {
                this.about_open = false;
                cx.notify();
            }))
            .into_any_element(),
        )
    }

    /// The page an artist shows instead of its hero and its tables when it cannot be read: the
    /// No connection state as soon as the network is gone, whatever this page happens to hold,
    /// and the failure of its own load otherwise. Opening the same artist again is the retry.
    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.detail.read(cx).id()?.to_owned();
        let reason = match trouble::unreachable(&id, cx) {
            true => None,
            false => Some(self.detail.read(cx).error()?.to_owned()),
        };
        let detail = self.detail.clone();

        Some(
            trouble::lost(
                "artist-lost",
                t!("trouble-not-loaded"),
                reason.as_deref(),
                move |_, _, cx| {
                    let id = id.clone();
                    detail.update(cx, |detail, cx| detail.open(&id, cx));
                },
                cx,
            )
            .size_full()
            .into_any_element(),
        )
    }
}

impl Tooled for ArtistView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, _cx: &App) -> Vec<AnyElement> {
        let viewed = self.me.clone();

        vec![tools::views(&self.popovers, self.mode, move |mode, cx| {
            viewed.update(cx, |view, cx| view.set_mode(mode, cx)).ok();
        })]
    }
}

impl Render for ArtistView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(failure) = self.failure(cx) {
            return div()
                .id("artist-failure")
                .role(Role::Alert)
                .relative()
                .size_full()
                .child(failure)
                .into_any_element();
        }

        let theme = *cx.theme();
        let inset = theme.metrics.inset;
        let previous = self.width;
        page::resize(&self.table, &mut self.width, inset, window, cx);

        let scroll = self.scrollbar.read(cx).scroll().clone();
        self.settle_release_padding(scrolled(&scroll));
        self.hold_releases(window, cx);
        if self.width != previous {
            self.scrollbar
                .update(cx, |bar, cx| bar.set_max_offset(None, cx));
        }
        let columns = Shape::new(self.width, self.popular.len()).columns;
        if self.popular_columns != columns {
            self.popular_columns = columns;
            self.popular_page = 0;
        }
        if self.mode == Mode::List {
            self.table.claim(cx);
            let mut viewport = page::viewport(&scroll, inset, window);
            viewport.top = (viewport.top - TABLE_BORDER).max(Pixels::ZERO);
            self.table
                .update(cx, |table, _| table.set_viewport(viewport));
        }

        let picked = self.track_context.and_then(|(place, position)| {
            self.popular
                .get(place)
                .cloned()
                .map(|track| (track, position))
        });
        let track_menu = picked.map(|(track, position)| {
            Popup::new(position, self.track_menu.for_track(&track, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.track_context = None;
                    cx.notify();
                },
            ))
        });

        let listed = self.mode == Mode::List;
        let release_padding = self.release_padding;
        let head = self.header(cx);
        let cover = theme.metrics.cover;
        let banner_shown = self
            .detail
            .read(cx)
            .artist()
            .is_some_and(|artist| artist.banner.is_some());
        let rise = match banner_shown {
            true => ((self.width + inset * 2.) / BANNER_BOX - inset - cover)
                .max(cover * BANNER_RISE_LEAST)
                .min(cover * BANNER_RISE_MOST),
            false => cover * BANNER_RISE_WASH,
        };
        let rise = snapped(rise, window);
        let banner = self.banner(inset, inset + rise + cover, inset + rise, cx);
        let tracks = match self.mode {
            Mode::Grid => self.popular(cx),
            Mode::List => self.listed(cx),
        };
        let grid = self.releases(window, cx);
        let about = self.about(cx);
        while self.rails.is_empty() {
            self.rails.push(Rail::new(cx.entity_id()));
        }
        for rail in &self.rails {
            rail.sync();
        }
        let weak = cx.entity().downgrade();
        let notify: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            weak.update(cx, |_, cx| cx.notify()).ok();
        });
        let rails = self.recommended(window, &notify, cx);
        let page =
            Scroller::new("artist-page", &self.scrollbar)
                .bottom_inset(crate::chrome::Chrome::bottom(cx))
                .px(inset)
                .pt(inset)
                .pb(inset)
                .on_scroll_wheel(cx.listener(Self::release_scroll))
                .child(div().relative().pt(rise).children(banner).child(head).when(
                    listed,
                    |this| {
                        let label = t!("artist-popular");
                        this.child(
                            div()
                                .id("artist-popular-heading")
                                .role(Role::Heading)
                                .aria_level(2)
                                .aria_label(label.clone())
                                .pb_3()
                                .text_size(theme.text(Text::Title))
                                .font_weight(FontWeight::BOLD)
                                .child(label),
                        )
                    },
                ))
                .child(tracks)
                .children(grid)
                .children(rails)
                .children(about)
                .when(release_padding > Pixels::ZERO, |this| {
                    this.child(div().h(release_padding).flex_none())
                });

        div()
            .relative()
            .size_full()
            .child(page)
            .when_some(track_menu, |this, menu| this.child(menu))
            .children(self.about_dialog(cx))
            .into_any_element()
    }
}

fn release_gap(window: &Window) -> Pixels {
    snapped(window.rem_size() * 1.5, window)
}

fn chevron(expanded: bool) -> &'static str {
    match expanded {
        true => "icons/chevron-up.svg",
        false => "icons/chevron-down.svg",
    }
}
