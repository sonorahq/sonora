use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Pixels, Point, Render, ScrollHandle,
    ScrollWheelEvent, SharedString, WeakEntity, Window, div,
};

use crate::chrome::Chrome;
use crate::shared::cells;
use i18n::t;
use music::{Album, ReleaseType, SavedArtist, Track};
use state::{AppSettings, ArtistDetail, Origin, Playback, Sonora};
use ui::ActiveTheme as _;
use ui::Listing as _;
use ui::{
    Button, Card, MIN_CONTENT, Mode, Picker, Pin, PinKind, Popovers, Popup, Scrollbar, Scroller,
    Skeleton, TableDelegate, TableEvent, TableState, Text, scrolled, snapped, table,
};

use crate::chrome::tools;
use crate::chrome::{Toolbar, Tooled};
use crate::shared::about::{AboutArtist, about_modal};
use crate::shared::album_grid::{AlbumGrid, CardGrid};
use crate::shared::confirm::Confirm;
use crate::shared::hero::{HeroMetaStrip, HeroPlayButton, PageHero};
use crate::shared::menus::{ItemMenu, album_menu, artist_menu};
use crate::shared::page;
use crate::shared::picks::{Picks, Shape};
use crate::shared::tracks::{PlaybackStatus, TrackSource, Tracks, drop_picked, playback_status};

const SECTION: &str = "artist";
const RELEASE_ROWS: usize = 2;
const LISTED: usize = 5;
const LISTED_MAX: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseFilter {
    All,
    Albums,
    Singles,
    Eps,
}

impl ReleaseFilter {
    const ALL: [Self; 4] = [Self::All, Self::Singles, Self::Albums, Self::Eps];

    fn id(self) -> &'static str {
        match self {
            Self::All => "release-filter-all",
            Self::Albums => "release-filter-albums",
            Self::Singles => "release-filter-singles",
            Self::Eps => "release-filter-eps",
        }
    }

    fn label(self) -> SharedString {
        match self {
            Self::All => t!("artist-filter-all"),
            Self::Albums => t!("artist-filter-albums"),
            Self::Singles => t!("artist-filter-singles"),
            Self::Eps => t!("artist-filter-eps"),
        }
    }

    fn matches(self, kind: ReleaseType) -> bool {
        self == Self::All
            || matches!(
                (self, kind),
                (Self::Albums, ReleaseType::Album)
                    | (Self::Singles, ReleaseType::Single)
                    | (Self::Eps, ReleaseType::Ep)
            )
    }
}

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
    release_menu: Option<(Album, Point<Pixels>)>,
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
        let columns =
            crate::shared::tracks::artist_columns(Sonora::global(cx).session.read(cx).playcounts());
        let scroll = scrollbar.read(cx).scroll().clone();
        let shown = Rc::new(Cell::new(LISTED));
        let table = cx.new(|cx| {
            let menu_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
            let source = TrackSource::new(
                columns,
                ArtistTracks {
                    detail: detail.clone(),
                    shown: shown.clone(),
                },
                playback.clone(),
                menu_scrollbar,
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

        cx.observe(&detail, |this, detail, cx| {
            let artist_id = detail.read(cx).id().map(str::to_owned);
            if this.artist_id != artist_id {
                this.artist_id = artist_id;
                this.release_filter = ReleaseFilter::All;
                this.releases_expanded = false;
                this.release_padding = Pixels::ZERO;
                this.release_padding_offset = Pixels::ZERO;
                this.about_open = false;
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
            track_menu: ItemMenu::new(playlist_scrollbar),
            track_context: None,
            settings,
            toolbar,
            me: me.downgrade(),
            popovers: Popovers::default(),
            release_menu: None,
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
            .children(self.follow_button(cx))
            .children(overflow);

        let cover = artist.and_then(|artist| artist.cover_large.clone());
        let pin = self
            .detail
            .read(cx)
            .id()
            .map(|id| Pin::new(PinKind::Artist, id, title.clone()).cover(cover.clone()));

        PageHero::new("artist-hero", title)
            .pin(pin)
            .cover(cover)
            .eyebrow(t!("artist-eyebrow"))
            .when_some(listeners, |hero, listeners| {
                hero.meta(HeroMetaStrip::new().text(listeners))
            })
            .actions(actions)
            .circle()
            .into_any_element()
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

    fn follow_button(&self, cx: &App) -> Option<Button> {
        let theme = *cx.theme();
        let library = Sonora::global(cx).library.clone();
        let target = self.saved_artist(cx)?;
        if music::is_local_id(&target.id) {
            return None;
        }
        let followed = library.read(cx).saved_artist(&target.id);

        let heart = Button::new("artist-toggle-library")
            .outline()
            .icon(match followed {
                true => "icons/heart-filled.svg",
                false => "icons/heart.svg",
            })
            .tooltip(match followed {
                true => "artist-unfollow",
                false => "artist-follow",
            })
            .disabled(library.read(cx).pending_artist(&target.id));

        Some(
            match followed {
                true => heart.tint(theme.primary),
                false => heart,
            }
            .on_click(move |_, _, cx| match followed {
                true => Confirm::artists(vec![target.clone()], cx),
                false => {
                    library.update(cx, |library, cx| library.toggle_artist(target.clone(), cx));
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

    fn releases(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = *cx.theme();
        let detail = self.detail.read(cx);
        let loading = detail.is_loading();
        let albums = detail.albums();
        if albums.is_empty() && !loading {
            return None;
        }
        let local = detail.id().is_some_and(music::is_local_id);
        let filters = match loading {
            true => Vec::new(),
            false => release_filters(local, albums.iter().map(|album| album.release_type)),
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
                let matching: Vec<&Album> = albums
                    .iter()
                    .filter(|album| self.release_filter.matches(album.release_type))
                    .collect();
                let shown: Vec<(usize, Album)> = matching
                    .iter()
                    .take(match self.releases_expanded {
                        true => matching.len(),
                        false => grid.columns * RELEASE_ROWS,
                    })
                    .map(|album| (*album).clone())
                    .enumerate()
                    .collect();
                let opened = cx.entity().downgrade();

                div()
                    .flex()
                    .flex_col()
                    .gap(gap)
                    .children(shown.chunks(grid.columns.max(1)).map(|row| {
                        let opened = opened.clone();

                        AlbumGrid::new(
                            "artist-release",
                            self.width,
                            row.to_vec(),
                            self.playback.clone(),
                        )
                        .on_context(move |album, position, cx| {
                            let Some(view) = opened.upgrade() else {
                                return;
                            };
                            view.update(cx, |this, cx| {
                                this.release_menu = Some((album.clone(), position));
                                cx.notify();
                            });
                        })
                        .into_any_element()
                    }))
                    .into_any_element()
            }
        };

        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_6()
                .child(
                    div()
                        .text_size(theme.text(Text::Title))
                        .font_weight(FontWeight::BOLD)
                        .child(t!("artist-releases")),
                )
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
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_release_view(this.release_filter, !expanded, columns, window, cx);
                })),
        )
    }

    fn tracks_loading(&self, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let line = || Skeleton::new().w_full().h(theme.metrics.pad);

        div()
            .w_full()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(theme.metrics.header)
                    .px(theme.metrics.pad)
                    .bg(theme.table_head)
                    .child(line()),
            )
            .children((0..5).map(|_| {
                div()
                    .flex()
                    .items_center()
                    .h(theme.metrics.row)
                    .px(theme.metrics.pad)
                    .border_t_1()
                    .border_color(theme.table_row_border)
                    .child(line())
            }))
            .into_any_element()
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
            .child(match self.detail.read(cx).is_loading() {
                true => self.tracks_loading(cx),
                false => table(&self.table)
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .into_any_element(),
            })
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
            self.playback_status.0.clone(),
            self.width,
            self.popular_page,
        )
        .title("artist-popular")
        .eyebrow("artist-popular-eyebrow")
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

    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let error = self.detail.read(cx).error()?.to_owned();
        Some(
            div()
                .pb_4()
                .text_color(cx.theme().danger)
                .child(error)
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
        let theme = *cx.theme();
        let inset = theme.metrics.inset;
        let previous = self.width;
        page::resize(&self.table, &mut self.width, inset, window, cx);

        let scroll = self.scrollbar.read(cx).scroll().clone();
        self.settle_release_padding(scrolled(&scroll));
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
            let viewport = page::viewport(&scroll, inset, window);
            self.table
                .update(cx, |table, _| table.set_viewport(viewport));
        }

        let release_menu = self.release_menu.clone().map(|(album, position)| {
            let menu = album_menu(album, self.playback.clone(), false, cx);
            Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                this.release_menu = None;
                cx.notify();
            }))
        });
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
        let page = Scroller::new("artist-page", &self.scrollbar)
            .px(inset)
            .pt(inset)
            .pb(inset)
            .on_scroll_wheel(cx.listener(Self::release_scroll))
            .child(
                div()
                    .child(self.header(cx))
                    .children(self.failure(cx))
                    .when(listed, |this| {
                        this.child(
                            div()
                                .pb_3()
                                .text_size(theme.text(Text::Title))
                                .font_weight(FontWeight::BOLD)
                                .child(t!("artist-popular")),
                        )
                    }),
            )
            .child(match self.mode {
                Mode::Grid => self.popular(cx),
                Mode::List => self.listed(cx),
            })
            .children(self.releases(window, cx))
            .children(self.about(cx))
            .when(release_padding > Pixels::ZERO, |this| {
                this.child(div().h(release_padding).flex_none())
            });

        div()
            .relative()
            .size_full()
            .child(page)
            .when_some(release_menu, |this, menu| this.child(menu))
            .when_some(track_menu, |this, menu| this.child(menu))
            .children(self.about_dialog(cx))
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

fn release_filters(
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
    fn local_artists_have_no_release_filters() {
        assert!(release_filters(true, [ReleaseType::Album]).is_empty());
    }

    #[test]
    fn streamed_artists_only_show_populated_release_filters() {
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
