//! The artist page: the hero over the artist's banner, the popular songs as a short table or a
//! deck of cards, the discography with its release filters, what the artist appears on, and the
//! About card, with the favorites dialog and the No connection page in their place when the
//! artist cannot be read.

mod banner;
mod favorites;

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use ui::Smooth as _;

use i18n::t;
use music::{Album, GenreItem, SavedArtist, Track};
use router::{Destination, LibraryTab};
use rune::{Cx, Drag, El, PointerPress, ScrollSpec, Tx, WheelEvent, col, row};
use state::{AppSettings, ArtistDetail, Origin};
use ui::{Button, Card, Mode, Pending, ScrollWatch, Text, Weight, font, spoken_heading};

use crate::chrome::toolbar::Tools;
use crate::chrome::{player_bar, sidebar_left, tools};
use crate::shared::about::{self, AboutArtist};
use crate::shared::album_grid::{CardGrid, CardLayout, card_row};
use crate::shared::cards::{self, Fit, OnItem, ReleaseFilter};
use crate::shared::confirm;
use crate::shared::hero::{Hero, Listing, MetaStrip, PlayButton, action_gap, like_button};
use crate::shared::menus::{self, Target, TrackColumns, TrackList as MenuList};
use crate::shared::page::{self, rems};
use crate::shared::picks::{Picks, column_count};
use crate::shared::pins::Pinned as _;
use crate::shared::shelves::{Rail, Rails};
use crate::shared::tracks::{TrackList, TrackRows, TrackTable, artist_columns};
use crate::shared::{cells, trouble};
use crate::{Ui, act};

/// The key the page's table layout, sort and view mode are kept under in settings.
const SECTION: &str = "artist";
/// The page's scroll viewport and the popular songs' table.
const PAGE: &str = "artist-page";
const TABLE: &str = "artist-popular-table";
/// How many rows of releases show before the grid is expanded.
const RELEASE_ROWS: usize = 2;
/// How many popular songs the table lists, and how many once it is expanded.
const LISTED: usize = 5;
const LISTED_MAX: usize = 10;
/// The space above each section, under a section heading, and between the table and its
/// toggle, in rems.
const SECTION_GAP: f32 = 1.5;
const HEADING_GAP: f32 = 0.75;
const LIST_GAP: f32 = 0.5;
/// The line height of a section's title: GPUI's default, the golden ratio.
const TITLE_LEADING: f32 = 1.618;
/// The hairline around the popular table, which Rune draws over the box, so the frame pads by
/// as much to keep it off the rows.
const RULE: f32 = 1.;

/// The artist page's own view state, reset whenever the page opens on another artist.
pub struct State {
    /// The artist the state below belongs to.
    artist: Option<String>,
    filter: ReleaseFilter,
    expanded: bool,
    /// How many popular songs the table lists.
    shown: usize,
    /// The popular deck's page, with the column count it was turned at.
    popular: (usize, usize),
    /// Whether the favorites dialog is up.
    favorites: bool,
    /// The popular table's selection, layout and sort.
    tracks: TrackList,
    rails: Rails,
    /// Where the page is scrolled, for wheel turns over the table, which hand them on to it.
    scroll: ScrollWatch,
    /// The offset the page was last seen at and the one the wheel aims for: several turns
    /// within one frame add up rather than each starting from where the page was.
    aim: Cell<(f32, f32)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            artist: None,
            filter: ReleaseFilter::All,
            expanded: false,
            shown: LISTED,
            popular: (0, 0),
            favorites: false,
            tracks: TrackList::new(),
            rails: Rails::default(),
            scroll: ScrollWatch::new(),
            aim: Cell::new((0., 0.)),
        }
    }
}

impl State {
    /// Starts over for another artist, and puts back the table's saved layout and sort, as the
    /// window arrives at `destination`.
    pub(crate) fn arrived(&mut self, destination: &Destination, settings: &AppSettings) {
        let Destination::Artist(id) = destination else {
            return;
        };
        if self.artist.as_deref() != Some(id.as_ref()) {
            *self = Self {
                artist: Some(id.to_string()),
                ..Self::default()
            };
        }
        self.tracks.restore(settings, SECTION);
    }
}

/// Leaves the page for the library's artists when the local artist it shows is gone from the
/// files. Called as the window settles.
pub(crate) fn follow_gone(ui: &mut Ui) {
    let Destination::Artist(id) = ui.nav.current() else {
        return;
    };
    if !music::is_local_id(id.as_ref()) {
        return;
    }
    let Some(detail) = &ui.store.pages.artist else {
        return;
    };
    if detail.id().is_none() && !detail.is_loading() {
        ui.replace(Destination::Local(LibraryTab::Artists));
    }
}

/// The title bar's tools on the artist page: the switch between the table and the deck.
pub(crate) fn tools(ui: &Ui) -> Tools {
    let mode = ui.store.settings.view_or(SECTION, Mode::List);
    Tools {
        items: vec![tools::views(ui, mode, |ui: &mut Ui, mode| {
            menus::close(ui);
            let store = &mut ui.store;
            store.settings.set_view(SECTION, mode, &mut store.cx);
        })],
        search: None,
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let Some(detail) = ui.store.pages.artist.as_ref() else {
        return col().key("artist-page-empty").w_full().h_full();
    };
    if let Some(failure) = failure(ui, detail) {
        return col()
            .key("artist-failure")
            .role("alert")
            .w_full()
            .h_full()
            .child(failure);
    }
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let width = page::content_width(cx, inset * 2.);
    let mode = ui.store.settings.view_or(SECTION, Mode::List);
    let origin = playing_from(detail);
    if let Some(id) = detail.id() {
        favorites::dialog(ui, id, origin.clone(), crate::window_room(cx), layer);
    }

    let popular = match mode {
        Mode::Grid => deck(ui, detail, width, origin.clone()),
        Mode::List => listed(ui, detail, width, origin.clone()),
    };
    let mut page = col()
        .key(PAGE)
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .smooth::<Ui>(PAGE)
        .child(top(ui, detail, width, mode, origin))
        .child(
            col()
                .key("artist-popular")
                .w_full()
                .px(inset)
                .child(popular),
        );
    page = releases(ui, detail, width, page);
    page = page
        .children(appears_on(ui, detail, width))
        .children(about(ui, detail))
        .child(
            col()
                .key("artist-foot")
                .w_full()
                .h(inset + player_bar::bottom(ui, cx)),
        );
    // The workspace keys the page's root by its destination, so the viewport sits inside it to
    // keep the key the wheel over the table scrolls by.
    col()
        .key("screen-artist")
        .w_full()
        .h_full()
        .child(ui.screens.artist.scroll.vertical(page, f32::INFINITY))
}

/// The top of the page: the banner behind the hero, the favorites link under it and, over the
/// table, the popular songs' heading.
fn top(ui: &Ui, detail: &ArtistDetail, width: f32, mode: Mode, origin: Option<Origin>) -> El {
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let cover = theme.metrics.cover;
    let artist = detail.artist();
    let page = width + inset * 2.;
    let rise = banner::rise(artist, page, inset, cover);
    let banner = artist
        .and_then(|artist| banner::view(ui, artist, page, inset + rise + cover, inset + rise));
    let heading = (mode == Mode::List).then(|| {
        col()
            .key("artist-popular-head")
            .pb(rems(theme, HEADING_GAP))
            .child(section_heading(
                "artist-popular-heading",
                t!("artist-popular"),
                theme,
            ))
    });
    col()
        .key("artist-top")
        .w_full()
        .px(inset)
        .pt(inset + rise)
        .children(banner)
        .child(hero(ui, detail, origin))
        .children(detail.id().and_then(|id| favorites::link(ui, id)))
        .children(heading)
}

/// The hero: the round portrait, the name, the monthly listeners and the actions. Dragging it
/// carries the artist to the sidebar's pins.
fn hero(ui: &Ui, detail: &ArtistDetail, origin: Option<Origin>) -> El {
    let theme = &ui.theme;
    let artist = detail.artist();
    let title = artist.map(|artist| artist.name.clone()).unwrap_or_default();
    let saved = saved_artist(detail);
    let playable = detail.tracks().iter().any(|track| track.playable);
    let listing = Listing::Origin(origin.filter(|_| playable));
    let actions = row()
        .key("artist-actions")
        .center_y()
        .gap(action_gap(theme))
        .child(PlayButton::new("play-artist", listing.clone()).build(ui))
        .child(PlayButton::shuffle("shuffle-artist", listing).build(ui))
        .children(saved.clone().and_then(|artist| heart(ui, artist)))
        .children(saved.clone().map(|artist| {
            Button::new("artist-overflow")
                .icon("icons/ellipsis.svg")
                .tooltip(t!("common-more"))
                .ghost()
                .on_press(menus::below(
                    Target::Artist { artist, here: true },
                    "artist-overflow",
                ))
                .build(theme)
        }));
    let mut hero = Hero::new("artist-hero", title)
        .pending(artist.is_none() && detail.is_loading())
        .cover(artist.and_then(|artist| artist.cover_large.clone()))
        .actions(actions)
        .circle();
    if let Some(count) = artist.and_then(|artist| artist.monthly_listeners) {
        let value = cells::count(count);
        let listeners = t!("artist-monthly-listeners", count = count, value = &value);
        hero = hero.meta(MetaStrip::new().text(listeners).build(theme));
    }
    let Some(saved) = saved else {
        return hero.build(ui);
    };
    let menu = saved.clone();
    hero = hero.on_context(move |ui: &mut Ui, press: PointerPress, _: &mut Tx| {
        menus::open(
            ui,
            Target::Artist {
                artist: menu.clone(),
                here: true,
            },
            press,
        );
    });
    let built = hero.build(ui);
    match saved.pin() {
        Some(pin) => built.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            sidebar_left::carry(ui, &pin, drag, tx)
        }),
        None => built,
    }
}

/// The heart that follows or unfollows the artist, for a provider that keeps followed artists.
fn heart(ui: &Ui, artist: SavedArtist) -> Option<El> {
    if !ui.store.session.capabilities().follow_artists {
        return None;
    }
    let library = &ui.store.library;
    let saved = library.saved_artist(&artist.id);
    let pending = library.pending_artist(&artist.id);
    Some(like_button(
        "artist-toggle-library",
        saved,
        pending,
        act(move |ui: &mut Ui| {
            let artist = artist.clone();
            match saved && !confirm::unstarring(&ui.store.library, &artist.id) {
                true => confirm::artists(ui, vec![artist]),
                false => {
                    let store = &mut ui.store;
                    store
                        .library
                        .heart_artist(artist, &store.session, &mut store.cx);
                }
            }
        }),
        &ui.theme,
    ))
}

/// The popular songs as a framed table of five rows, or ten once expanded, with the toggle
/// under it. The table is its own scroll viewport that never scrolls, so it hands the wheel on
/// to the page.
fn listed(ui: &Ui, detail: &ArtistDetail, width: f32, origin: Option<Origin>) -> El {
    let theme = &ui.theme;
    let playcounts = ui.store.session.capabilities().playcounts;
    let rows = TrackRows::new(
        shown_tracks,
        |ui: &Ui| &ui.screens.artist.tracks,
        |ui: &mut Ui| &mut ui.screens.artist.tracks,
        artist_columns(playcounts),
    )
    .origin(origin)
    .liked();
    let loading = detail.is_loading() && detail.tracks().is_empty();
    let count = match loading {
        true => LISTED,
        false => shown_tracks(ui).len(),
    };
    let height = theme.metrics.header + theme.metrics.row * count as f32 + RULE * 2.;
    let table = TrackTable::new(TABLE, &rows)
        .width(width - RULE * 2.)
        .pending(loading.then_some(Pending::Rows(LISTED)))
        .label(t!("artist-popular"))
        .remember(SECTION)
        .on_context(|ui: &mut Ui, picked, press: PointerPress, _: &mut Tx| {
            let columns = TrackColumns {
                album: picked.album,
                artists: picked.artists,
            };
            menus::open_tracks(ui, picked.tracks, MenuList::Plain, columns, press);
        })
        .on_row_drag(
            |ui: &mut Ui, tracks: &[Track], drag: &mut Drag, tx: &mut Tx| {
                if let Some(pin) = tracks.first().and_then(|track| track.pin()) {
                    sidebar_left::carry(ui, &pin, drag, tx);
                }
            },
        )
        .build(ui)
        .on_wheel(hand_on);
    let frame = col()
        .key("artist-popular-frame")
        .w_full()
        .h(height)
        .padding(RULE)
        .radius(theme.radius)
        .border(RULE, theme.border.color())
        .clip_rounded(theme.radius)
        .child(table);
    let expanded = ui.screens.artist.shown > LISTED;
    let more = (detail.tracks().len() > LISTED).then(|| {
        Button::new("artist-popular-more")
            .label(match expanded {
                true => t!("artist-popular-less"),
                false => t!("artist-popular-more"),
            })
            .trailing(chevron(expanded))
            .small()
            .ghost()
            .expanded(expanded)
            .on_press(move |ui: &mut Ui| {
                ui.screens.artist.shown = match expanded {
                    true => LISTED,
                    false => LISTED_MAX,
                };
            })
            .build(theme)
            .w_full()
    });
    col()
        .key("artist-popular-list")
        .w_full()
        .gap(rems(theme, LIST_GAP))
        .child(frame)
        .children(more)
}

/// The popular songs as a paged deck of cards, each starting the songs from itself.
fn deck(ui: &Ui, detail: &ArtistDetail, width: f32, origin: Option<Origin>) -> El {
    let columns = column_count(width);
    let page = match ui.screens.artist.popular {
        (turned, page) if turned == columns => page,
        _ => 0,
    };
    Picks::new("artist-popular", detail.tracks(), width, page)
        .title("artist-popular")
        .eyebrow(i18n::lookup("artist-popular-eyebrow", None))
        .vacancy("artist-popular-empty")
        .detailed()
        .loading(detail.is_loading())
        .on_page(move |ui: &mut Ui, page| {
            ui.screens.artist.popular = (columns, page);
            menus::close(ui);
        })
        .on_context(|ui: &mut Ui, place, press: PointerPress, _: &mut Tx| {
            let Some(track) = popular_tracks(ui).get(place).cloned() else {
                return;
            };
            menus::open(ui, Target::track(track), press);
        })
        .on_start(move |ui: &mut Ui, place| {
            let tracks = popular_tracks(ui).to_vec();
            ui.store.play_tracks(tracks, place, origin.clone());
        })
        .build(ui)
}

/// Adds the Releases section to `page`: its heading and filters, the grid of releases (two rows
/// of them until expanded) as a direct child of the scroll viewport so only the rows on screen
/// are built, and the toggle under it. Nothing while the artist has no releases.
fn releases(ui: &Ui, detail: &ArtistDetail, width: f32, page: El) -> El {
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let albums = detail.albums();
    // A discography still on its way reads as loading only while nothing of it is up.
    let loading = detail.is_loading() || (albums.is_empty() && detail.is_filling());
    if albums.is_empty() && !loading {
        return page;
    }
    let state = &ui.screens.artist;
    let local = detail.id().is_some_and(music::is_local_id);
    let filters = match loading {
        true => Vec::new(),
        false => cards::release_filters(local, albums.iter().map(|album| album.release_type)),
    };
    let layout = CardLayout::new(width);
    let columns = layout.columns.max(1);
    let filter = state.filter;
    let matching: Vec<usize> = albums
        .iter()
        .enumerate()
        .filter(|(_, album)| filter.matches(album.release_type))
        .map(|(index, _)| index)
        .collect();
    let crowded = matching.len() > columns * RELEASE_ROWS;
    let expanded = state.expanded;

    let pills = (!filters.is_empty()).then(|| {
        row()
            .key("artist-release-filters")
            .gap(rems(theme, 0.25))
            .children(filters.into_iter().map(|pill| {
                let chosen = filter == pill;
                Button::new(pill.id())
                    .label(pill.label())
                    .small()
                    .outline()
                    .selected(chosen)
                    .toggled(chosen)
                    .on_press(move |ui: &mut Ui| {
                        let state = &mut ui.screens.artist;
                        if state.filter != pill {
                            state.filter = pill;
                            state.expanded = false;
                        }
                    })
                    .build(theme)
            }))
    });
    let head = col()
        .key("artist-releases-head")
        .w_full()
        .px(inset)
        .pt(rems(theme, SECTION_GAP))
        .pb(rems(theme, HEADING_GAP))
        .gap(rems(theme, HEADING_GAP))
        .child(section_heading(
            "artist-releases-heading",
            t!("artist-releases"),
            theme,
        ))
        .children(pills);
    let page = page.child(head);

    let grid = match loading {
        true => {
            let gap = rems(theme, SECTION_GAP);
            col()
                .key("artist-releases-pending")
                .w_full()
                .px(inset)
                .gap(gap)
                .children((0..RELEASE_ROWS).map(|line| {
                    card_row(
                        layout,
                        (0..layout.columns).map(|column| {
                            Card::skeleton(format!(
                                "artist-release-skeleton-{}",
                                line * layout.columns + column
                            ))
                            .tile(layout.card)
                            .build(theme)
                        }),
                    )
                }))
                .a11y_hidden()
        }
        false => {
            let shown: Rc<Vec<usize>> = Rc::new(match expanded {
                true => matching,
                false => matching.into_iter().take(columns * RELEASE_ROWS).collect(),
            });
            let keyed = shown.clone();
            let on_item = item_menu();
            // One key whatever the filter, so a change of filter or of the expansion eases rows
            // in and out of the same grid rather than swapping grids.
            CardGrid::new("artist-releases", width, shown.len())
                .inset(inset)
                .build(
                    theme,
                    move |ui: &Ui, at: usize| {
                        keyed
                            .get(at)
                            .and_then(|&index| releases_of(ui).get(index))
                            .map_or(Cow::Borrowed(""), |album| Cow::Borrowed(album.id.as_str()))
                    },
                    move |ui: &Ui, at: usize, tile: f32| {
                        let Some(album) =
                            shown.get(at).and_then(|&index| releases_of(ui).get(index))
                        else {
                            return Card::skeleton(format!("artist-release-{at}"))
                                .tile(tile)
                                .build(&ui.theme);
                        };
                        cards::album_card(
                            ui,
                            format!("artist-release-{}", album.id),
                            album,
                            Fit::Tile(tile),
                            Some(&on_item),
                        )
                        .flat()
                        .build(&ui.theme)
                    },
                )
        }
    };
    let page = page.child(grid);
    let toggle = (!loading && crowded).then(|| {
        row()
            .key("artist-releases-toggle")
            .w_full()
            .px(inset)
            .pt(rems(theme, HEADING_GAP))
            .child(
                Button::new("artist-releases-more")
                    .label(match expanded {
                        true => t!("artist-releases-less"),
                        false => t!("artist-releases-more"),
                    })
                    .trailing(chevron(expanded))
                    .small()
                    .ghost()
                    .expanded(expanded)
                    .on_press_tx(|ui: &mut Ui, tx: &mut Tx| {
                        let state = &mut ui.screens.artist;
                        state.expanded = !state.expanded;
                        // Collapsing a long discography would leave the page far below what
                        // is left of it: the toggle stays under the pointer instead.
                        tx.keep_on_screen("artist-releases-more");
                    })
                    .build(theme)
                    .grow(1.),
            )
    });
    page.children(toggle)
}

/// The rail of releases the artist appears on, skeletons while the page is still filling.
fn appears_on(ui: &Ui, detail: &ArtistDetail, width: f32) -> Option<El> {
    let theme = &ui.theme;
    let layout = CardLayout::new(width);
    let appears = detail.appears_on();
    let title = t!("artist-appears-on").into_owned();
    let rail = match appears.len() {
        0 if detail.is_filling() => Rail::pending("artist-appears", title, layout, theme),
        0 => return None,
        count => {
            let on_item = item_menu();
            Rail::new("artist-appears", title, &ui.screens.artist.rails, layout)
                .cards(appears.iter().enumerate().map(|(index, album)| {
                    cards::album_card(
                        ui,
                        format!("artist-appears-{}", album.id),
                        album,
                        Fit::Tile(layout.card),
                        Some(&on_item),
                    )
                    .flat()
                    .build(theme)
                    .position_in_set(index + 1, count)
                }))
                .build(theme)
        }
    };
    Some(
        col()
            .key("artist-appears-section")
            .w_full()
            .px(theme.metrics.inset)
            .pt(rems(theme, SECTION_GAP))
            .child(rail),
    )
}

/// The About card, which opens the whole biography.
fn about(ui: &Ui, detail: &ArtistDetail) -> Option<El> {
    let artist = detail.artist()?;
    let theme = &ui.theme;
    let (name, biography) = (artist.name.clone(), artist.biography.clone());
    let card = AboutArtist::new("artist-about", artist.name.clone())
        .cover(artist.cover_large.clone())
        .biography(artist.biography.clone())
        .on_open(move |ui: &mut Ui| about::open(ui, name.clone(), biography.clone(), None))
        .build(ui);
    Some(
        col()
            .key("artist-about-section")
            .w_full()
            .px(theme.metrics.inset)
            .pt(rems(theme, SECTION_GAP))
            .child(card),
    )
}

/// The page an artist shows instead of its hero and its lists when it cannot be read: the No
/// connection state as soon as the network is gone, whatever this page happens to hold, and
/// the failure of its own load otherwise. Opening the same artist again is the retry.
fn failure(ui: &Ui, detail: &ArtistDetail) -> Option<El> {
    let id = detail.id()?.to_owned();
    let reason = match trouble::unreachable(ui, &id) {
        true => None,
        false => Some(detail.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "artist-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        move |ui: &mut Ui| ui.store.open_artist(&id),
    ))
}

/// A section's title, as a level two heading.
fn section_heading(key: &str, label: impl Into<String>, theme: &ui::Theme) -> El {
    let size = theme.text(Text::Title);
    spoken_heading(key, label, 2, theme)
        .font(font(theme, Text::Title, Weight::BOLD).lh((size * TITLE_LEADING).round()))
}

/// The artist as the library and the menus know one.
fn saved_artist(detail: &ArtistDetail) -> Option<SavedArtist> {
    let artist = detail.artist()?;
    Some(SavedArtist {
        id: detail.id()?.to_owned(),
        name: artist.name.clone(),
        cover: artist.cover_large.clone(),
        ..Default::default()
    })
}

/// The origin playback names for songs started from the page.
fn playing_from(detail: &ArtistDetail) -> Option<Origin> {
    let name = detail.artist()?.name.clone();
    Some(Origin::artist(detail.id()?).named(name))
}

/// Every popular song of the artist on show.
fn popular_tracks(ui: &Ui) -> &[Track] {
    ui.store
        .pages
        .artist
        .as_ref()
        .map(ArtistDetail::tracks)
        .unwrap_or_default()
}

/// The popular songs the table lists: the first five, or ten once expanded.
fn shown_tracks(ui: &Ui) -> &[Track] {
    let tracks = popular_tracks(ui);
    &tracks[..ui.screens.artist.shown.min(tracks.len())]
}

/// The artist's releases, in the store's order.
fn releases_of(ui: &Ui) -> &[Album] {
    ui.store
        .pages
        .artist
        .as_ref()
        .map(ArtistDetail::albums)
        .unwrap_or_default()
}

/// Opens the context menu of a card where the pointer pressed.
fn item_menu() -> OnItem {
    Rc::new(
        |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
            if let Some(target) = Target::of(&item) {
                menus::open(ui, target, press);
            }
        },
    )
}

/// Scrolls the page by a wheel turn over the popular table, whose own viewport never scrolls
/// and would otherwise keep it. Turns that land before the page reports its new offset add up.
fn hand_on(ui: &mut Ui, event: WheelEvent, tx: &mut Tx) -> bool {
    if event.dy.abs() < event.dx.abs() {
        return false;
    }
    let state = &ui.screens.artist;
    let at = state.scroll.offset();
    let (seen, aimed) = state.aim.get();
    let from = match seen == at {
        true => aimed,
        false => at,
    };
    let to = (from - event.dy).clamp(0., state.scroll.max().max(0.));
    state.aim.set((at, to));
    tx.scroll_to_instant(PAGE, 0., to);
    true
}

fn chevron(expanded: bool) -> &'static str {
    match expanded {
        true => "icons/chevron-up.svg",
        false => "icons/chevron-down.svg",
    }
}
