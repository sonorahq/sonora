use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Pixels, Point, Render, ScrollHandle, SharedString,
    WeakEntity, Window, div, px,
};

use i18n::t;
use music::{Album, Playlist, Track};
use router::{Destination, navigate};
use state::{AppSettings, Collection, Detail, LibraryEvent, Origin, Playback, Sonora};
use ui::{
    ActiveTheme as _, Button, InlineLink, InlineLinks, Menu, Picker, Popovers, Popup, SortAxis,
};
use ui::{
    ColumnSpec, Listing as _, MIN_CONTENT, Pin, PinKind, Scrollbar, Scroller, TableDelegate,
    TableEvent, TableState, Toggle, clock, table,
};

use crate::shared::menus::{album_menu, playlist_menu};

use crate::chrome::tools::{self, Sliders};
use crate::chrome::{Chrome, Searchable, Toolbar, Tooled};
use crate::shared::confirm::Confirm;
use crate::shared::hero::{HeroMetaStrip, HeroPlayButton, PageHero, release_date_label};
use crate::shared::tracks::{
    PlaybackStatus, TrackField, TrackSource, Tracks, drop_picked, playback_status, playlist_columns,
};
use crate::shared::{cells, page};

const PINNED: [&str; 3] = ["cover", "title", "name"];

enum Saveable {
    Album(Album),
    Playlist(Playlist),
}

struct DetailTracks(Entity<Detail>);

impl Tracks for DetailTracks {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        self.0.read(cx).tracks()
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.0.read(cx).is_loading()
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
    context_menu: Option<Point<Pixels>>,
    toolbar: Entity<Toolbar>,
    popovers: Popovers,
    sliders: Sliders,
    me: WeakEntity<Self>,
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
            let source = TrackSource::new(
                columns,
                DetailTracks(detail.clone()),
                playback.clone(),
                playlist_scrollbar,
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
            let mut delegate = TableDelegate::new(source, width, cx);
            delegate.set_layout(saved, cx);
            TableState::new(delegate, cx).follow(scroll)
        });

        cx.observe(&detail, |this, _, cx| {
            this.scrollbar
                .read(cx)
                .scroll()
                .set_offset(gpui::Point::default());
            this.restore_sorting(cx);
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

        Self {
            detail,
            playback,
            playback_status: current_playback,
            width,
            scrollbar,
            table,
            settings,
            section,
            sorted: None,
            context_menu: None,
            toolbar,
            popovers: Popovers::default(),
            sliders: Sliders::default(),
            me: me.downgrade(),
        }
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

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let header = self.detail.read(cx).header();
        let kind = header
            .map(|header| header.kind)
            .unwrap_or(Collection::Album);
        let title = header
            .map(|header| SharedString::from(header.title.clone()))
            .unwrap_or_default();
        let artist = header.and_then(|header| header.artist.clone());
        let artist_refs = header
            .map(|header| header.artist_refs.clone())
            .unwrap_or_default();
        let owner = header.and_then(|header| header.owner.clone());
        let release_date = header.and_then(|header| header.release_date.as_deref());
        let meta = header.map(|header| header.meta.clone()).unwrap_or_default();
        let listed = self.detail.read(cx).tracks();
        let duration: std::time::Duration = listed.iter().map(|track| track.duration).sum();
        let (eyebrow, label) = match kind {
            Collection::Playlist => (t!("detail-playlist"), t!("detail-play-playlist")),
            Collection::Album => (t!("detail-album"), t!("detail-play-album")),
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
        for item in meta {
            strip = strip.text(item);
        }
        if !duration.is_zero() {
            strip = strip.text(clock(duration));
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
            .children(self.library_button(cx))
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
                Saveable::Album(album) if saved => Confirm::albums(vec![album.clone()], cx),
                Saveable::Album(album) => {
                    library.update(cx, |library, cx| library.toggle_album(album.clone(), cx));
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

    fn menu(&self, cx: &App) -> Option<Menu> {
        let detail = self.detail.read(cx);
        let id = detail.id()?.to_owned();
        let header = detail.header()?;

        Some(match header.kind {
            Collection::Album => {
                album_menu(detail.album()?.clone(), self.playback.clone(), true, cx)
            }
            Collection::Playlist => {
                let saved = Sonora::global(cx).library.read(cx).playlist(&id).cloned();
                let playlist = saved.or_else(|| detail.playlist().cloned())?;
                playlist_menu(playlist, self.playback.clone(), true, cx)
            }
        })
    }
}

impl Render for DetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        div()
            .relative()
            .size_full()
            .child(
                Scroller::new("detail-page", &self.scrollbar)
                    .pt(inset)
                    .pb(inset)
                    .child(div().px(inset).child(self.header(cx)))
                    .child(table(&self.table)),
            )
            .when_some(context_menu, |this, menu| this.child(menu))
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

    fn toggles(&self, cx: &App) -> Vec<Toggle> {
        self.table
            .read(cx)
            .delegate()
            .toggles()
            .into_iter()
            .filter(|toggle| !PINNED.contains(&toggle.key))
            .collect()
    }

    fn toggle_column(&mut self, key: &'static str, cx: &mut Context<Self>) {
        if PINNED.contains(&key) {
            return;
        }

        let mut layout = self.table.layout(cx);
        layout.toggle(key);
        self.table.clone().set_layout(layout, cx);
        self.persist(cx);
        cx.notify();
    }
}

impl Tooled for DetailView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, cx: &App) -> Vec<AnyElement> {
        let columned = self.me.clone();
        let sifted = self.me.clone();
        let sorted = self.me.clone();

        vec![
            tools::columns(&self.popovers, self.toggles(cx), move |key, cx| {
                columned
                    .update(cx, |view, cx| view.toggle_column(key, cx))
                    .ok();
            }),
            tools::filters(
                &self.popovers,
                &self.sliders,
                self.table.filters(cx),
                move |change, cx| {
                    sifted
                        .update(cx, |view, cx| view.table.filter(change, cx))
                        .ok();
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
