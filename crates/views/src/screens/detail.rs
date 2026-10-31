use gpui::accesskit::{Role, Toggled};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Hsla, Pixels, Point, Render, ScrollHandle, SharedString,
    WeakEntity, Window, div, px,
};

use std::rc::Rc;

use i18n::t;
use music::{Album, AlbumDetail, Playlist, ReleaseType, Track};
use router::{Destination, navigate};
use state::{
    AppSettings, Collection, Detail, DownloadsChanged, LibraryEvent, Origin, Playback, Progress,
    Sonora,
};
use ui::{
    ActiveTheme as _, ArrowRing, Button, InlineLink, InlineLinks, Menu, Picker, Popovers, Popup,
    SortAxis,
};
use ui::{
    ColumnSpec, FilterChange, Listing as _, MIN_CONTENT, Pending, Pin, PinKind, Scrollbar,
    Scroller, TableDelegate, TableEvent, TableState, Text, runtime, table,
};

use crate::shared::menus::{album_menu, playlist_menu};
use crate::shared::trouble;

use crate::chrome::tools::{self, Sliders};
use crate::chrome::{Chrome, Searchable, Toolbar, Tooled};
use crate::shared::album_grid::CardGrid;
use crate::shared::cards;
use crate::shared::confirm::Confirm;
use crate::shared::hero::{
    HeroMetaStrip, HeroPlayButton, PageHero, copyright_notices, release_date_label,
};
use crate::shared::shelves::{Rail, RailSpec};
use crate::shared::tracks::{
    PlaybackStatus, TrackField, TrackSource, Tracks, drop_picked, playback_status, playlist_columns,
};
use crate::shared::{cells, page};

const PINNED: [&str; 3] = ["cover", "title", "name"];

enum Saveable {
    Album(Album),
    Playlist(Playlist),
}

/// Which list the recommendation rail shows: releases, or artists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RailTab {
    Albums,
    Artists,
}

impl RailTab {
    fn id(self) -> &'static str {
        match self {
            Self::Albums => "rail-tab-albums",
            Self::Artists => "rail-tab-artists",
        }
    }

    fn label(self) -> SharedString {
        match self {
            Self::Albums => t!("album-tab-albums"),
            Self::Artists => t!("album-tab-artists"),
        }
    }
}

struct DetailTracks(Entity<Detail>);

impl Tracks for DetailTracks {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        self.0.read(cx).tracks()
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.0.read(cx).is_loading()
    }

    /// As many rows as the header says the album or playlist holds, or a screenful when the
    /// page opened without a header or with a provider that reports no count.
    fn pending(&self, cx: &App) -> Option<Pending> {
        let detail = self.0.read(cx);
        if !detail.is_loading() {
            return None;
        }
        Some(
            match detail.header().map_or(0, |header| header.track_count) {
                0 => Pending::Screen,
                count => Pending::Rows(count as usize),
            },
        )
    }
}

pub(crate) struct DetailView {
    detail: Entity<Detail>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    width: Pixels,
    scrollbar: Entity<Scrollbar>,
    table: Entity<TableState<TrackSource>>,
    settings: Entity<AppSettings>,
    section: &'static str,
    sorted: Option<String>,
    shown: Option<String>,
    context_menu: Option<Point<Pixels>>,
    toolbar: Entity<Toolbar>,
    popovers: Popovers,
    sliders: Sliders,
    me: WeakEntity<Self>,
    /// The recommendation rail under the table.
    rails: Vec<Rail>,
    /// Which list the recommendation rail shows.
    rail_tab: RailTab,
}

impl DetailView {
    pub(crate) fn new(
        detail: Entity<Detail>,
        playback: Entity<Playback>,
        columns: &'static [ColumnSpec<TrackField>],
        show_liked: bool,
        section: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let saved = settings.read(cx).table(section);
        let width = MIN_CONTENT;

        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        let scroll = scrollbar.read(cx).scroll().clone();

        let table = cx.new(|cx| {
            let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
            crate::shared::tracks::follow_downloads(cx);
            let source = TrackSource::new(
                columns,
                DetailTracks(detail.clone()),
                playback.clone(),
                playlist_scrollbar,
                cx,
            );
            let source = match show_liked {
                true => source.with_liked(Sonora::global(cx).library.clone()),
                false => source,
            };
            let source = match section == "playlist" {
                true => source.with_playlist(detail.clone()),
                false => source,
            };
            let source = match section == "album" {
                true => source.with_album(detail.clone()),
                false => source,
            };
            let source = source.from({
                let detail = detail.clone();
                move |cx: &App| {
                    let detail = detail.read(cx);
                    match (detail.album(), detail.playlist()) {
                        (Some(album), _) => {
                            Some(Origin::album(album.id.clone()).named(album.name.clone()))
                        }
                        (_, Some(list)) => {
                            Some(Origin::playlist(list.id.clone()).named(list.name.clone()))
                        }
                        _ => None,
                    }
                }
            });
            let source = source.table(cx.weak_entity());
            let mut delegate = TableDelegate::new(source, width, cx).with_column_menu(&PINNED);
            delegate.set_layout(saved, cx);
            TableState::new(delegate, cx).follow(scroll)
        });

        cx.observe(&detail, |this, detail, cx| {
            let shown = detail.read(cx).id().map(str::to_owned);
            if this.shown != shown {
                this.shown = shown;
                this.rails.clear();
                this.rail_tab = RailTab::Albums;
                this.scrollbar
                    .read(cx)
                    .scroll()
                    .set_offset(gpui::Point::default());
                this.restore_sorting(cx);
                // The rows belong to another detail now, so its filters must not leak across.
                this.table.filter(FilterChange::Reset, cx);
                this.restore_filters(cx);
            }
            this.retune(cx);
            this.rebuild(cx);
            cx.notify();
        })
        .detach();

        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, move |this, _, cx| {
            if show_liked {
                this.table.update(cx, |table, cx| table.refresh(cx));
            }
            cx.notify();
        })
        .detach();

        if section == "playlist" {
            let library = Sonora::global(cx).library.clone();
            cx.subscribe(&library, |_, _, event, cx| {
                let LibraryEvent::PlaylistGone(id) = event else {
                    return;
                };
                let trail = router::trail(cx);
                if trail.read(cx).current() != router::Destination::Playlist(id.clone().into()) {
                    return;
                }
                match trail.read(cx).can_go_back() {
                    true => router::back(cx),
                    false => router::navigate(
                        router::Destination::Library(router::LibraryTab::Playlists),
                        cx,
                    ),
                }
            })
            .detach();
        }

        // Progress only redraws the download button. A filtered table rebuilds when what it
        // shows may have changed, since a rebuild drops the open menu and the selection.
        let downloads = Sonora::global(cx).downloads.clone();
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&downloads, |this, _, _: &DownloadsChanged, cx| {
            if this.table.narrowed(cx) {
                this.rebuild(cx);
            }
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
        let toolbar = Toolbar::searchable(&me, cx);

        let mut view = Self {
            detail,
            playback,
            playback_status: current_playback,
            width,
            scrollbar,
            table,
            settings,
            section,
            sorted: None,
            shown: None,
            context_menu: None,
            toolbar,
            popovers: Popovers::default(),
            sliders: Sliders::default(),
            me: me.downgrade(),
            rails: Vec::new(),
            rail_tab: RailTab::Albums,
        };
        view.restore_filters(cx);
        view
    }

    fn retune(&mut self, cx: &mut Context<Self>) {
        if self.section != "playlist" {
            return;
        }

        let detail = self.detail.read(cx);
        let blend = detail.playlist().is_some_and(|list| list.blend);
        let shared = detail.tracks().iter().any(|track| track.added_by.is_some());
        let columns = playlist_columns(blend, shared);

        self.table.update(cx, |table, cx| {
            if table.delegate_mut().source_mut().set_columns(columns) {
                table.rebuild(cx);
            }
        });
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().clear_selection();
            table.rebuild(cx);
        });
    }

    fn sort_key(&self, cx: &App) -> String {
        match self.detail.read(cx).id() {
            Some(id) if self.section == "playlist" => format!("{}:{id}", self.section),
            _ => self.section.to_owned(),
        }
    }

    fn restore_sorting(&mut self, cx: &mut Context<Self>) {
        let key = self.sort_key(cx);
        if self.sorted.as_deref() == Some(key.as_str()) {
            return;
        }
        self.sorted = Some(key.clone());

        let sorting = self.settings.read(cx).sorting(&key);
        self.table.clone().set_sorting(sorting.flatten(), cx);
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        let key = self.sort_key(cx);
        page::store(
            &self.settings.clone(),
            &self.table.clone(),
            self.section,
            &key,
            cx,
        );
    }

    fn sift(&mut self, change: FilterChange, cx: &mut Context<Self>) {
        self.table.filter(change, cx);
        self.persist(cx);
        cx.notify();
    }

    /// Fills filter axes the storage names for the detail on show. See `LibraryView::restore`.
    fn restore_filters(&mut self, cx: &mut Context<Self>) {
        let key = self.sort_key(cx);
        page::restore(&self.settings.clone(), &self.table, &key, cx);
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let header = self.detail.read(cx).header();
        let kind = header
            .map(|header| header.kind)
            .or(self.detail.read(cx).kind())
            .unwrap_or(Collection::Album);
        let release = header
            .and_then(|header| header.release)
            .unwrap_or(ReleaseType::Album);
        let title = header
            .map(|header| SharedString::from(header.title.clone()))
            .unwrap_or_default();
        let artist = header.and_then(|header| header.artist.clone());
        let artist_refs = header
            .map(|header| header.artist_refs.clone())
            .unwrap_or_default();
        let owner = header.and_then(|header| header.owner.clone());
        let release_date = header.and_then(|header| header.release_date.as_deref());
        let owner_name = header.and_then(|header| header.owner_name.clone());
        let track_count = header.map(|header| header.track_count).unwrap_or(0);
        let listed = self.detail.read(cx).tracks();
        let duration: std::time::Duration = listed.iter().map(|track| track.duration).sum();
        let (eyebrow, label) = match kind {
            Collection::Playlist => (t!("detail-playlist"), t!("detail-play-playlist")),
            Collection::Album => (
                i18n::lookup(cards::release_key(release), None),
                t!("detail-play-album"),
            ),
        };

        let mut strip = HeroMetaStrip::new();
        if let Some(artist) = artist {
            strip = strip.item(cells::artist_links(
                "detail-artist",
                artist_refs,
                artist,
                muted,
            ));
        }
        if let Some(owner) = owner {
            strip = strip.item(
                InlineLinks::new(
                    SharedString::new_static("detail-owner"),
                    [InlineLink::new(owner.name.clone(), Some(owner.id.into()))],
                    owner.name,
                    muted,
                )
                .on_click(|id, cx| navigate(Destination::User(id), cx))
                .truncate(),
            );
        }
        if let Some(release_date) = release_date {
            strip = strip.text(release_date_label(release_date));
        }
        if let Some(owner_name) = owner_name {
            strip = strip.text(owner_name);
        }
        if track_count > 0 {
            strip = strip.text(t!("count-songs", count = track_count));
        }
        if !duration.is_zero() {
            strip = strip.text(runtime(duration));
        }

        let overflow = self.menu(cx).map(|menu| {
            Picker::icon("detail-overflow", &self.popovers, "icons/ellipsis.svg")
                .tooltip("common-more")
                .large()
                .left()
                .menu(menu)
        });
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(HeroPlayButton::listed(
                "play-detail",
                label,
                &self.table,
                self.playback.clone(),
            ))
            .child(HeroPlayButton::shuffle_listed(
                "shuffle-detail",
                &self.table,
                self.playback.clone(),
            ))
            .children(self.library_button(cx))
            .children(self.download_button(cx))
            .children(overflow);

        let cover = header.and_then(|header| header.cover.clone());
        let pin = self.detail.read(cx).id().map(|id| {
            let kind = match kind {
                Collection::Album => PinKind::Album,
                Collection::Playlist => PinKind::Playlist,
            };
            Pin::new(kind, id, title.clone()).cover(cover.clone())
        });

        let view = self.me.clone();
        PageHero::new("detail-hero", title)
            .pending(header.is_none() && self.detail.read(cx).is_loading())
            .pin(pin)
            .cover(cover)
            .eyebrow(eyebrow)
            .meta(strip)
            .actions(actions)
            .drag_start(move |event, window, cx| {
                window.prevent_default();
                view.update(cx, |this, cx| {
                    this.context_menu = Some(event.position);
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }

    fn library_button(&self, cx: &App) -> Option<Button> {
        let theme = *cx.theme();
        let library = Sonora::global(cx).library.clone();
        let detail = self.detail.read(cx);
        let id = detail.id()?.to_owned();

        let (target, saved, busy) = match detail.header()?.kind {
            Collection::Album => (
                Saveable::Album(detail.album()?.clone()),
                library.read(cx).saved_album(&id),
                library.read(cx).pending_album(&id),
            ),
            Collection::Playlist => {
                let known = library.read(cx).playlist(&id).cloned();
                let playlist = known.clone().or_else(|| detail.playlist().cloned())?;
                if playlist.owned {
                    return None;
                }
                (Saveable::Playlist(playlist), known.is_some(), false)
            }
        };

        let heart = Button::new("detail-toggle-library")
            .outline()
            .icon(match saved {
                true => "icons/heart-filled.svg",
                false => "icons/heart.svg",
            })
            .tooltip(match saved {
                true => "menu-remove-from-library",
                false => "menu-add-to-library",
            })
            .disabled(busy);

        Some(
            match saved {
                true => heart.tint(theme.primary),
                false => heart,
            }
            .on_click(move |_, _, cx| match &target {
                Saveable::Album(album) if saved && !Confirm::unstarring(&album.id, cx) => {
                    Confirm::albums(vec![album.clone()], cx)
                }
                Saveable::Album(album) => {
                    library.update(cx, |library, cx| library.heart_album(album.clone(), cx));
                }
                Saveable::Playlist(playlist) if saved => {
                    Confirm::playlists(vec![playlist.id.clone()], cx)
                }
                Saveable::Playlist(playlist) => {
                    library.update(cx, |library, cx| {
                        library.add_playlist_to_library(playlist.clone(), cx)
                    });
                }
            }),
        )
    }

    /// Downloads the album or playlist for offline play, or removes its download. It shows for a
    /// provider whose tracks can be downloaded, and for one that is offline with this page
    /// downloaded.
    fn download_button(&self, cx: &App) -> Option<Button> {
        let theme = *cx.theme();
        let sonora = Sonora::global(cx);
        let downloads = sonora.downloads.clone();
        let detail = self.detail.read(cx);
        let id = detail.id()?.to_owned();
        if music::is_local_id(&id) {
            return None;
        }
        let kind = detail.header()?.kind;
        let progress = downloads.read(cx).set(kind, &id);
        if progress.is_none() && !sonora.session.read(cx).capabilities().downloads {
            return None;
        }
        let album = detail.album().cloned();
        let playlist = detail.playlist().cloned();
        let tracks = detail.tracks().to_vec();
        let share = downloads.read(cx).share(kind, &id);
        let ring = |done: f32, color: Hsla| {
            ArrowRing::new("detail-download-ring", Some(done)).color(color)
        };
        let button = Button::new("detail-download").outline();
        Some(
            match progress {
                None => button.icon("icons/download.svg").tooltip("menu-download"),
                Some(Progress::Failed) => button
                    .leading(ring(share, theme.danger))
                    .tooltip("menu-download"),
                Some(Progress::Waiting | Progress::Running) => button
                    .leading(ring(share, theme.primary))
                    .tooltip("menu-downloading"),
                Some(Progress::Done) => button
                    .leading(ring(1., theme.primary))
                    .tooltip("menu-remove-download"),
            }
            .on_click(move |_, _, cx| {
                downloads.update(cx, |downloads, cx| match (progress, &album, &playlist) {
                    (None | Some(Progress::Failed), Some(album), _) => {
                        downloads.download_album(
                            AlbumDetail {
                                album: album.clone(),
                                tracks: tracks.clone(),
                            },
                            cx,
                        );
                    }
                    (None | Some(Progress::Failed), None, Some(playlist)) => {
                        downloads.download_playlist_by_id(playlist, cx);
                    }
                    (None | Some(Progress::Failed), None, None) => {}
                    _ => downloads.remove_set(kind, &id, cx),
                })
            }),
        )
    }

    /// The recommendation rail under the table, on album pages only: related releases
    /// under the albums tab, similar artists under the artists tab. The tabs show while
    /// both lists hold something. A rail still on its way reads as skeletons only while
    /// nothing of it is up.
    fn recommended(
        &self,
        window: &Window,
        notify: &Rc<dyn Fn(&mut App)>,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let grid = CardGrid::layout(self.width - cx.theme().metrics.inset * 2.);
        let card = grid.card;
        let columns = grid.columns.max(1);
        let detail = self.detail.read(cx);
        if detail.album().is_none() {
            return Vec::new();
        }
        let filling = detail.is_filling();
        let albums = detail.also_like().len();
        let artists = detail.similar().len();
        if albums == 0 && artists == 0 {
            return match filling {
                true => vec![Rail::pending(
                    t!("album-also-like"),
                    card,
                    columns,
                    window,
                    cx,
                )],
                false => Vec::new(),
            };
        }
        // A tab without a list behind it is not one the page can be on.
        let tab = match self.rail_tab {
            RailTab::Albums if albums == 0 => RailTab::Artists,
            RailTab::Artists if artists == 0 => RailTab::Albums,
            tab => tab,
        };

        let opened = self.me.clone();
        let tabs = match albums > 0 && artists > 0 {
            false => None,
            true => {
                let opened = opened.clone();
                Some(
                    div()
                        .flex()
                        .gap_1()
                        .children([RailTab::Albums, RailTab::Artists].into_iter().map(|tab| {
                            let opened = opened.clone();
                            Button::new(tab.id())
                                .label(tab.label())
                                .small()
                                .outline()
                                .selected(tab == self.rail_tab)
                                .aria_toggled(Toggled::from(tab == self.rail_tab))
                                .on_click(move |_, _, cx| {
                                    opened
                                        .update(cx, |this, cx| {
                                            if this.rail_tab == tab {
                                                return;
                                            }
                                            this.rail_tab = tab;
                                            for rail in &this.rails {
                                                rail.rewind();
                                            }
                                            cx.notify();
                                        })
                                        .ok();
                                })
                        }))
                        .into_any_element(),
                )
            }
        };
        let count = match tab {
            RailTab::Albums => albums,
            RailTab::Artists => artists,
        };
        vec![self.rails[0].render(
            RailSpec {
                tag: "detail-also-like",
                place: 0,
                title: t!("album-also-like"),
                count,
                tile: card,
                columns,
                tabs,
            },
            window,
            cx,
            notify,
            move |position, _, cx| {
                let Some(view) = opened.upgrade() else {
                    return div().into_any_element();
                };
                let held = view.read(cx);
                let detail = held.detail.read(cx);
                match tab {
                    RailTab::Albums => {
                        let Some(album) = detail.also_like().get(position) else {
                            return div().into_any_element();
                        };
                        let id = SharedString::from(format!("detail-also-like-{}", album.id));
                        cards::album_card(id, album, &held.playback, cx)
                            .tile(card)
                            .flat()
                            .aria_position_in_set(position + 1)
                            .aria_size_of_set(count)
                            .into_any_element()
                    }
                    RailTab::Artists => {
                        let Some(artist) = detail.similar().get(position) else {
                            return div().into_any_element();
                        };
                        let id = SharedString::from(format!("detail-similar-{}", artist.id));
                        cards::artist_card(id, artist, &held.playback, cx)
                            .tile(card)
                            .flat()
                            .aria_position_in_set(position + 1)
                            .aria_size_of_set(count)
                            .into_any_element()
                    }
                }
            },
        )]
    }

    /// The album's copyright and label lines under the table, or nothing on a playlist and on
    /// an album whose provider names neither.
    fn notices(&self, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        let notices = copyright_notices(self.detail.read(cx).album()?);
        if notices.is_empty() {
            return None;
        }
        Some(
            div()
                .px(theme.metrics.pad * 2.)
                .pt_2()
                .flex()
                .flex_col()
                .gap_1()
                .min_w_0()
                .text_size(theme.text(Text::Tiny))
                .text_color(theme.muted_foreground)
                .children(
                    notices
                        .into_iter()
                        .enumerate()
                        .map(|(index, notice)| ui::spoken(("detail-notice", index), notice)),
                )
                .into_any_element(),
        )
    }

    fn menu(&self, cx: &App) -> Option<Menu> {
        let detail = self.detail.read(cx);
        let id = detail.id()?.to_owned();
        let header = detail.header()?;

        Some(match header.kind {
            Collection::Album => {
                let menus = self.table.read(cx).delegate().source().menu();
                album_menu(detail.album()?.clone(), self.playback.clone(), menus, cx)
            }
            Collection::Playlist => {
                let saved = Sonora::global(cx).library.read(cx).playlist(&id).cloned();
                let playlist = saved.or_else(|| detail.playlist().cloned())?;
                playlist_menu(playlist, self.playback.clone(), cx)
            }
        })
    }
}

impl Render for DetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(failure) = self.failure(cx) {
            return div()
                .id("detail-failure")
                .role(Role::Alert)
                .relative()
                .size_full()
                .child(failure)
                .into_any_element();
        }

        self.table.claim(cx);
        let inset = cx.theme().metrics.inset;
        let width = cells::content_width(window, Pixels::ZERO, cx);
        if (width - self.width).abs() >= px(0.5) {
            self.width = width;
            self.table.set_width(width, cx);
        }

        let scroll = self.scrollbar.read(cx).scroll().clone();
        let viewport = page::viewport(&scroll, inset, window);
        self.table
            .update(cx, |table, _| table.set_viewport(viewport));

        let context_menu = self.context_menu.and_then(|position| {
            let menu = self.menu(cx)?;
            Some(
                Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                })),
            )
        });

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

        div()
            .relative()
            .size_full()
            .child(
                Scroller::new("detail-page", &self.scrollbar)
                    .bottom_inset(crate::chrome::Chrome::bottom(cx))
                    .pt(inset)
                    .pb(inset)
                    .child(div().px(inset).child(self.header(cx)))
                    .child(table(&self.table))
                    .children(self.notices(cx))
                    .child(div().px(inset).pt_10().children(rails)),
            )
            .when_some(context_menu, |this, menu| this.child(menu))
            .into_any_element()
    }
}

impl DetailView {
    /// The page an album or a playlist shows instead of its header and its table when it cannot
    /// be read: the No connection state as soon as the network is gone, whatever rows this page
    /// happens to hold, and the failure of its own load otherwise. A downloaded album or
    /// playlist opens from its kept copy instead, network or not.
    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.read(cx);
        let id = detail.id()?.to_owned();
        let kept = detail.kind().is_some_and(|kind| {
            Sonora::global(cx)
                .downloads
                .read(cx)
                .set(kind, &id)
                .is_some()
        });
        if kept && detail.error().is_none() {
            return None;
        }
        let reason = match trouble::unreachable(&id, cx) {
            true => None,
            false => Some(self.detail.read(cx).error()?.to_owned()),
        };
        let detail = self.detail.clone();

        Some(
            trouble::lost(
                "detail-lost",
                t!("trouble-not-loaded"),
                reason.as_deref(),
                move |_, _, cx| {
                    detail.update(cx, |detail, cx| detail.reload(cx));
                },
                cx,
            )
            .size_full()
            .into_any_element(),
        )
    }
}

impl Searchable for DetailView {
    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_query(query, cx);
            table.refresh(cx);
        });
        cx.notify();
    }

    fn hint() -> SharedString {
        "filter-album".into()
    }
}

impl DetailView {
    fn sorts(&self, cx: &App) -> Vec<SortAxis> {
        self.table.sortables(cx)
    }

    fn set_sort(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.table.clone().cycle_sort(key, cx);
        cx.notify();
    }
}

impl Tooled for DetailView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, cx: &App) -> Vec<AnyElement> {
        let sifted = self.me.clone();
        let sorted = self.me.clone();

        vec![
            tools::filters(
                &self.popovers,
                &self.sliders,
                self.table.filters(cx),
                move |change, cx| {
                    sifted.update(cx, |view, cx| view.sift(change, cx)).ok();
                },
                cx,
            ),
            tools::sorts(
                &self.popovers,
                self.sorts(cx),
                move |key, cx| {
                    sorted.update(cx, |view, cx| view.set_sort(key, cx)).ok();
                },
                cx,
            ),
        ]
    }
}
