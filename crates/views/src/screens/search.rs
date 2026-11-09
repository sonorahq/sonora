//! The search page. Typing asks the store for suggestions, whose best hits show in a panel under
//! the field, and only Return or the Search button asks for the full results the page lists:
//! the best match over three columns of songs, artists and releases, or one mixed list where
//! the page is narrow. While nothing is asked, the page browses the provider's genres. The arrow
//! keys walk the panel, then the best match, then the lists.

use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};

use i18n::t;
use music::{Album, Playlist, SavedArtist, Track};
use router::Destination;
use rune::{
    Cx, Drag, El, GrowthSpec, Key, KeyEvent, ScrollSpec, TextEdit, Transition, Tx, col, el, row,
    text,
};
use state::{AlbumHit, ArtistHit, Hit, Kind, Origin, PlaylistHit, Search};
use ui::{
    Button, Card, Cover, Dropdown, Input, Room, Separator, TABULAR, Text, Theme, VAST, Weight,
    clock, spoken_eyebrow, spoken_vacant,
};

use crate::shared::cards;
use crate::shared::cells;
use crate::shared::menus::{self, Target};
use crate::shared::page::{self, rems};
use crate::shared::pins::Pinned as _;
use crate::shared::shelves;
use crate::shared::trouble;
use crate::{Ui, act, act_tx};

/// The inset of each column's rows from its rule.
const RAIL: f32 = 12.;
/// How tall the band is where results fade out as they scroll under what sits above them.
const FADE: f32 = 12.;
/// How many past searches the panel lists.
const PAST: usize = 8;
/// How many hits the panel suggests.
const SUGGESTED: usize = 6;
/// How many placeholder rows stand in while the first suggestions load.
const SUGGESTED_WAITING: usize = 3;
/// How many placeholder rows a list shows while its first results load.
const LOADING_ROWS: usize = 4;
/// The space between two rows of a list, in rems.
const ROW_GAP: f32 = 0.25;
/// The best match's art against the theme's cover size.
const BEST_ART: f32 = 0.45;
/// How far a section of results rises as it comes in, and how long each waits on the one before.
const RISE: f32 = 10.;
const STAGGER: f32 = 0.05;
/// How far a suggestion drops in from, and how long each waits on the row above.
const DROP: f32 = 4.;
const STEP: f32 = 0.025;
const SONGS: &[Kind] = &[Kind::Song];
const ARTISTS: &[Kind] = &[Kind::Artist];
const RELEASES: &[Kind] = &[Kind::Album, Kind::Playlist];
/// The clear button's side padding, tighter than a small button's own, as the field's own.
const CLEAR_INSET: f32 = 4.;
/// The element that holds the keyboard while the arrow keys walk the results.
const KEYS: &str = "search-keys";
/// The panel's anchor, whose width the panel takes.
const ANCHOR: &str = "search-field-anchor";
/// The mixed list's viewport, which standing on the best match scrolls to its top.
const MIXED: &str = "search-all";

/// What the search page keeps between frames: the field, whether its panel was put away, and
/// where the arrow keys stand.
#[derive(Default)]
pub struct State {
    pub edit: TextEdit,
    /// Whether the panel under the field was put away, by Escape, a press outside or a submit.
    /// Typing, clicking into the field or Down brings it back.
    dismissed: bool,
    /// The panel row the arrow keys are on.
    pick: Option<usize>,
    /// The key of the row `pick` is on, so the pick stays with it as the suggestions reorder.
    picked: Option<String>,
    /// Whether the arrow keys are on the best match, the stop between the field and the lists.
    on_best: bool,
    /// The list and row the arrow keys are on.
    cursor: Option<(usize, usize)>,
    /// The row each column was last left on, so Left and Right come back to it.
    rows: [usize; 3],
}

impl State {
    /// Puts the keyboard in the field as the page opens, without the panel, so the genres stay
    /// in view until the listener clicks the field, presses Down or starts typing.
    pub(crate) fn arrived(&mut self, destination: &Destination) {
        if !matches!(destination, Destination::Search) {
            return;
        }
        self.edit.focused = true;
        self.dismissed = true;
        self.pick = None;
        self.picked = None;
    }
}

/// One row of the panel under the search field: a past query while the field is empty, a
/// pasted link to open, or a suggested hit of any kind while something is typed.
#[derive(Clone)]
enum Offer {
    Past(String),
    Link(String),
    Hit(Box<Hit>),
}

impl Offer {
    /// What tells one row from another when the suggestions fill in under it.
    fn key(&self) -> String {
        match self {
            Offer::Past(query) => format!("offer-past-{query}"),
            Offer::Link(_) => "offer-link".into(),
            Offer::Hit(hit) => format!("offer-{}", hit_key(hit)),
        }
    }
}

/// Where a hit's row sits: its place among the hits of its kind, its index in the list it is
/// drawn in and that list's length.
#[derive(Clone, Copy)]
struct Seat {
    index: usize,
    total: usize,
}

/// How a hit's row is drawn: compact in the panel and the mixed list, where each row is tagged
/// with its kind; whether the arrow keys are on it; and whether pressing it also files the
/// query and puts the panel away.
#[derive(Clone, Copy)]
struct Look {
    compact: bool,
    chosen: bool,
    panel: bool,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let pad = theme.metrics.inset;
    let width = page::content_width(cx, pad * 2.);
    let stacked = !crate::content_room(cx).fits(Room::Wide);
    let inset = match width > VAST {
        true => (width - VAST) / 2.,
        false => 0.,
    };
    let gutter = pad + inset;
    let bottom = crate::chrome::player_bar::bottom(ui, cx);
    let asked = !asked(ui).is_empty();
    let generation = generation(asked_query(ui));

    let results = match failure(ui, asked, gutter) {
        Some(failure) => failure,
        None => match (asked, stacked) {
            (false, _) => browse(ui, gutter, width - inset * 2., bottom),
            (true, true) => everything(ui, gutter, bottom, generation),
            (true, false) => row()
                .key("search-columns")
                .grow(1.)
                .min_h(0.)
                .w_full()
                .px(gutter)
                .child(column(ui, Kind::Song, bottom, generation, 1))
                .child(Separator::vertical().build(theme))
                .child(column(ui, Kind::Artist, bottom, generation, 2))
                .child(Separator::vertical().build(theme))
                .child(column(ui, Kind::Album, bottom, generation, 3)),
        },
    };
    let best = match stacked {
        true => None,
        false => {
            best(ui, generation).map(|best| col().key("search-best-row").px(gutter).child(best))
        }
    };
    let mut keys = col()
        .key(KEYS)
        .role("search")
        .label(t!("search-submit"))
        .w_full()
        .h_full()
        .min_h(0.)
        .gap(rems(theme, 1.5))
        .pt(pad)
        // The arrow keys walk the results from here: the field hands the keyboard over on Down,
        // and a press on the page's background lands it here as well.
        .focusable(true)
        .tab_index(-1)
        .on_key(move |ui: &mut Ui, event: &KeyEvent, tx: &mut Tx| {
            let taken = page_key(ui, event, tx, stacked);
            if taken {
                ui.settle();
            }
            taken
        })
        .child(field(ui, cx, gutter, width - inset * 2., stacked, layer))
        .children(best)
        .child(results);
    if let Some(chosen) = chosen_key(ui, stacked) {
        keys = keys.active_descendant(chosen);
    }
    col().w_full().h_full().min_h(0.).child(keys)
}

/// The search field with its panel and the button that runs the full search. Once the page is
/// too narrow for two lanes of genres, the panel spans the button too, so the suggestions get
/// the whole width.
fn field(
    ui: &Ui,
    cx: &mut Cx<Ui>,
    gutter: f32,
    width: f32,
    stacked: bool,
    layer: &mut Vec<El>,
) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.search;
    let open = panel_open(ui);
    let offers = match open {
        true => offers(ui),
        false => Vec::new(),
    };
    let pick = picked_at(ui, &offers);
    let empty = state.edit.text.trim().is_empty();

    let mut input = Input::new(
        "search-input",
        &state.edit,
        t!("search-placeholder"),
        field_access,
    )
    .icon("icons/search.svg")
    .on_change(act_tx(|ui: &mut Ui, _: &mut Tx| changed(ui)))
    .on_key(move |ui: &mut Ui, event: &KeyEvent, tx: &mut Tx| {
        let taken = field_key(ui, event, tx, stacked);
        if taken {
            ui.settle();
        }
        taken
    })
    .build(theme)
    .grow(1.)
    .min_w(0.);
    // The field's own clear button would empty it without telling the page, so the page draws
    // its own, which also takes the page back to browsing.
    if !state.edit.text.is_empty() {
        input = input.child(
            Button::new("search-clear")
                .icon("icons/x.svg")
                .tooltip(t!("common-clear"))
                .small()
                .ghost()
                .on_press_tx(act_tx(|ui: &mut Ui, tx: &mut Tx| {
                    let state = st(ui);
                    state.edit.clear();
                    state.edit.focused = true;
                    changed(ui);
                    tx.instant();
                }))
                .build(theme)
                .px(CLEAR_INSET),
        );
    }
    if let Some(at) = pick
        && let Some(offer) = offers.get(at)
    {
        input = input.active_descendant(offer.key());
    }
    let button = || {
        Button::new("search-submit")
            .icon("icons/search.svg")
            .label(t!("search-submit"))
            .primary()
            .disabled(empty)
            .on_press(act(|ui: &mut Ui| {
                let query = ui.screens.search.edit.text.clone();
                run(ui, &query);
            }))
            .build(theme)
            .h(theme.metrics.field)
    };
    let gap = rems(theme, 0.5);
    let panel_width = cx.layout_with(ANCHOR, |rect| rect[2].round()).unwrap_or(0.);
    let panel = open.then(|| panel_rows(ui, &offers, pick));
    let one_lane = shelves::lanes(width) == 1;
    let anchor = match one_lane {
        true => row()
            .w_full()
            .min_w(0.)
            .center_y()
            .gap(gap)
            .child(input)
            .child(button()),
        false => input,
    };
    let mut dropdown = Dropdown::<Ui>::new("search-field", anchor)
        .open(open)
        .on_close(act(close_panel));
    if panel_width > 0. {
        dropdown = dropdown.width(panel_width);
    }
    if let Some(panel) = panel {
        dropdown = dropdown.child(panel);
    }
    let dropdown = dropdown.build(theme, layer).grow(1.).min_w(0.);
    let field = row()
        .key("search-field-row")
        .w_full()
        .center_y()
        .gap(gap)
        .px(gutter)
        .child(dropdown);
    match one_lane {
        true => field,
        false => field.child(button()),
    }
}

/// The panel's rows, with placeholder rows while the first suggestions load. The list is named
/// for screen readers after what it holds.
fn panel_rows(ui: &Ui, offers: &[Offer], pick: Option<usize>) -> El {
    let theme = &ui.theme;
    let heading = match query(ui).is_empty() {
        true => t!("search-recent"),
        false => t!("search-results"),
    };
    let total = offers.len();
    let waiting = offers.is_empty().then(|| {
        (0..SUGGESTED_WAITING).map(|at| {
            Card::skeleton(format!("offer-waiting-{at}"))
                .tucked()
                .build(theme)
                .transition(Transition::opacity())
        })
    });
    let rows = offers.iter().enumerate().map(|(at, offer)| {
        offer_row(ui, offer, at, total, pick == Some(at))
            .transition(Transition::asymmetric(
                Transition::opacity().combined(Transition::offset(0., -DROP)),
                Transition::opacity(),
            ))
            .enter_delay(at as f32 * STEP)
    });
    col()
        .key("search-offers")
        .role("listbox")
        .label(heading)
        .w_full()
        .gap(1.)
        .children(waiting.into_iter().flatten())
        .children(rows)
}

fn offer_row(ui: &Ui, offer: &Offer, at: usize, total: usize, chosen: bool) -> El {
    let theme = &ui.theme;
    let key = offer.key();
    let seat = Seat { index: at, total };
    let art = |icon: &'static str| {
        Cover::new(ui.artwork(), None)
            .size(row_art(theme))
            .radius(theme.radius.min(4.))
            .fallback(icon)
            .build(theme)
    };
    match offer {
        Offer::Hit(hit) => hit_row(
            ui,
            hit,
            &key,
            seat,
            Look {
                compact: true,
                chosen,
                panel: true,
            },
        ),
        Offer::Link(_) => {
            let taken = offer.clone();
            Card::new(key, t!("search-open-link"))
                .tucked()
                .art(art("icons/search.svg"))
                .chosen(chosen)
                .on_press(act(move |ui: &mut Ui| take(ui, taken.clone())))
                .build(theme)
                .position_in_set(at + 1, total)
        }
        Offer::Past(query) => {
            let taken = offer.clone();
            let forgotten = query.clone();
            Card::new(key, query.clone())
                .tucked()
                .art(art("icons/rotate-ccw-clock.svg"))
                .action(
                    Button::new(format!("search-forget-{at}"))
                        .icon("icons/x.svg")
                        .tooltip(t!("search-forget"))
                        .ghost()
                        .small()
                        .tint(theme.muted_foreground)
                        .on_press(act(move |ui: &mut Ui| forget(ui, &forgotten)))
                        .build(theme)
                        .mr(rems(theme, 0.25)),
                )
                .chosen(chosen)
                .on_press(act(move |ui: &mut Ui| take(ui, taken.clone())))
                .build(theme)
                .position_in_set(at + 1, total)
        }
    }
}

/// The best match, the first hit, drawn large over the lists with its kind over its name.
fn best(ui: &Ui, generation: u64) -> Option<El> {
    let theme = &ui.theme;
    let hit = ui.store.pages.search.as_ref()?.best()?;
    let on_best = ui.screens.search.on_best;
    let kind = hit.kind();
    let (title, artists) = match hit {
        Hit::Song(track) => (track.name.clone(), track.artist_refs.clone()),
        Hit::Artist(artist) => (artist.name.clone(), Vec::new()),
        Hit::Album(album) => (album.name.clone(), album.artist_refs.clone()),
        Hit::Playlist(list) => (list.name.clone(), Vec::new()),
    };
    let side = (theme.metrics.cover * BEST_ART).round();
    let circle = matches!(kind, Kind::Artist);
    let mut card = Card::new("search-best-card", title)
        .art(art(ui, hit, side, circle))
        .art_size(side)
        .eyebrow(noun(kind))
        .description(noun(kind))
        .size(Text::Title)
        .weight(Weight::BOLD)
        .bare_meta(
            cells::artist_links(
                "best-artist-link",
                &artists,
                meta(hit, false),
                theme.muted_foreground,
            )
            .text_size(theme.text(Text::Small))
            .truncate()
            .build(theme),
        )
        .flat();
    if circle {
        card = card.circle();
    }
    if let Some((playing, play)) = transport(ui, hit) {
        card = card.play(playing, play);
    }
    if let Some(target) = target(hit) {
        card = card.on_press(act(move |ui: &mut Ui| open_target(ui, &target)));
    }
    if let Some(menu) = menu_target(ui, hit) {
        card = card.menu(menus::at(menu));
    }
    let mut card = card
        .build(theme)
        .gap(rems(theme, 1.))
        .padding(rems(theme, 0.75))
        .fill(match on_best {
            true => theme.table_active.color(),
            false => theme.secondary.color(),
        });
    if let Some(pin) = hit.pin() {
        card = card.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            crate::chrome::sidebar_left::carry(ui, &pin, drag, tx)
        });
    }
    let label = t!("search-best-match");
    Some(
        col()
            .key(format!("search-best-{generation:x}"))
            .w_full()
            .gap(rems(theme, 0.5))
            .child(
                spoken_eyebrow("search-best-heading", label, theme)
                    .role("heading")
                    .level(2)
                    .pb(rems(theme, 0.25)),
            )
            .child(card)
            .transition(entrance())
            .enter_delay(0.),
    )
}

/// One column of the wide layout: its heading over its own scrolling list.
fn column(ui: &Ui, kind: Kind, bottom: f32, generation: u64, order: usize) -> El {
    let theme = &ui.theme;
    let (key, title, only) = match kind {
        Kind::Song => ("search-songs-heading", t!("search-songs"), SONGS),
        Kind::Artist => ("search-artists-heading", t!("search-artists"), ARTISTS),
        Kind::Album | Kind::Playlist => (
            "search-releases-heading",
            t!("search-albums-playlists"),
            RELEASES,
        ),
    };
    let body = deck(ui, only, RAIL, false, title.to_string(), bottom);
    col()
        .key(format!("{key}-{generation:x}"))
        .grow(1.)
        .shrink()
        .min_w(0.)
        .min_h(0.)
        .gap(rems(theme, 0.25))
        .child(
            spoken_eyebrow(key, title, theme)
                .role("heading")
                .level(2)
                .pl(RAIL)
                .pb(rems(theme, 0.25)),
        )
        .child(body)
        .transition(entrance())
        .enter_delay(order as f32 * STAGGER)
}

/// The narrow layout: one mixed list, headed by the best match.
fn everything(ui: &Ui, gutter: f32, bottom: f32, generation: u64) -> El {
    col()
        .key(format!("search-everything-{generation:x}"))
        .grow(1.)
        .min_h(0.)
        .w_full()
        .child(deck(
            ui,
            &Kind::ALL,
            gutter,
            true,
            t!("search-results").to_string(),
            bottom,
        ))
        .transition(entrance())
}

/// What leads the mixed list: the best match and the heading over the rest.
fn lead(ui: &Ui) -> El {
    let theme = &ui.theme;
    col()
        .key("search-lead")
        .w_full()
        .gap(rems(theme, 1.5))
        .children(best(ui, 0))
        .child(
            spoken_eyebrow("search-results-heading", t!("search-results"), theme)
                .role("heading")
                .level(2)
                .pb(rems(theme, 0.25)),
        )
}

/// A virtual list of the hits of the `only` kinds, each row `gutter` in from the sides. The
/// mixed list leads with the best match as its first row.
fn deck(ui: &Ui, only: &[Kind], gutter: f32, leading: bool, label: String, bottom: f32) -> El {
    let theme = &ui.theme;
    let id = list_id(only);
    let column = column_of(only);
    let seats = seats(ui, only);
    let loading = ui
        .store
        .pages
        .search
        .as_ref()
        .is_some_and(Search::is_loading);
    if seats.is_empty() {
        let body = match loading {
            true => col()
                .key(format!("{id}-loading"))
                .w_full()
                .gap(rems(theme, ROW_GAP))
                .children((0..LOADING_ROWS).map(|at| {
                    Card::skeleton(format!("{id}-waiting-{at}"))
                        .build(theme)
                        .transition(Transition::opacity())
                })),
            false => spoken_vacant(format!("{id}-empty"), t!("search-no-matches"), theme)
                .role("status")
                .transition(Transition::opacity()),
        };
        return match leading {
            true => col()
                .key(id)
                .w_full()
                .gap(rems(theme, 0.25))
                .px(gutter)
                .child(lead(ui))
                .child(body),
            false => body,
        };
    }

    let compact = only.len() > 1;
    let lead_rows = usize::from(leading);
    let count = seats.len() + lead_rows;
    let estimate = theme.metrics.list_row.round() + rems(theme, ROW_GAP);
    let gap = rems(theme, ROW_GAP);
    let foot = theme.metrics.inset + bottom;
    let keyed = seats.clone();
    let list = id.to_owned();
    let rows = col()
        .key(format!("{id}-rows"))
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            estimate,
            move |ui: &Ui, at: usize| -> Cow<'_, str> {
                if at < lead_rows {
                    return Cow::Borrowed("lead");
                }
                let Some(hit) = keyed
                    .get(at - lead_rows)
                    .and_then(|&(hit, _)| hits(ui).get(hit))
                else {
                    return Cow::Owned(format!("{list}-gone-{at}"));
                };
                Cow::Owned(format!("{list}-{}", hit_key(hit)))
            },
            move |ui: &Ui, at: usize| {
                let content = match at < lead_rows {
                    true => lead(ui),
                    false => {
                        let index = at - lead_rows;
                        let hit = seats.get(index).and_then(|&(hit, _)| hits(ui).get(hit));
                        match hit {
                            Some(hit) => hit_row(
                                ui,
                                hit,
                                &format!("{id}-{}", hit_key(hit)),
                                Seat {
                                    index,
                                    total: seats.len(),
                                },
                                Look {
                                    compact,
                                    chosen: ui.screens.search.cursor == Some((column, index)),
                                    panel: false,
                                },
                            ),
                            None => el(),
                        }
                    }
                };
                let pad = match at + 1 < count {
                    true => gap,
                    false => foot,
                };
                col().w_full().pb(pad).child(content)
            },
        );
    col()
        .key(id)
        .role("listbox")
        .label(label)
        .grow(1.)
        .min_h(0.)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(FADE)
        .scroll_margin(FADE)
        .px(gutter)
        .child(rows)
}

/// The genres to browse while nothing is asked.
fn browse(ui: &Ui, gutter: f32, width: f32, bottom: f32) -> El {
    let theme = &ui.theme;
    let pad = theme.metrics.inset;
    col()
        .key("search-browse")
        .grow(1.)
        .min_h(0.)
        .w_full()
        .gap(rems(theme, 0.5))
        .child(
            col().px(gutter).child(
                spoken_eyebrow("search-browse-heading", t!("search-browse"), theme)
                    .role("heading")
                    .level(2),
            ),
        )
        .child(
            col()
                .key("search-browse-list")
                .grow(1.)
                .min_h(0.)
                .w_full()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .scroll_edge_fade(FADE)
                .px(gutter)
                .pb(pad + bottom)
                .child(shelves::grid(
                    ui,
                    "genre",
                    width - page::reserved(0.),
                    |ui: &Ui| ui.store.genres.list(),
                )),
        )
        .transition(entrance())
}

/// The page search shows in place of its results. Nothing here works without the network, so
/// the No connection state covers the whole screen the moment it is gone, and a failed search or
/// a failed browse shows its own reason otherwise. Asking again is the retry, since neither
/// loader counts a failure as served.
fn failure(ui: &Ui, asked: bool, gutter: f32) -> Option<El> {
    let offline = ui.store.network.lost();
    let (key, reason, retry): (&str, Option<String>, Retry) = match asked {
        true => {
            let reason = match offline {
                true => None,
                false => Some(ui.store.pages.search.as_ref()?.error()?.to_owned()),
            };
            let query = asked_query(ui).to_owned();
            (
                "search-lost",
                reason,
                Box::new(move |ui: &mut Ui| ui.store.search(&query)),
            )
        }
        false => {
            let reason = match offline {
                true => None,
                false => Some(ui.store.genres.error()?.to_owned()),
            };
            (
                "search-browse-lost",
                reason,
                Box::new(|ui: &mut Ui| ui.store.load_genres()),
            )
        }
    };
    Some(
        col()
            .key(format!("{key}-page"))
            .grow(1.)
            .min_h(0.)
            .w_full()
            .px(gutter)
            .center()
            .child(trouble::lost(
                ui,
                key,
                t!("trouble-not-loaded"),
                reason.as_deref(),
                move |ui: &mut Ui| retry(ui),
            ))
            .transition(Transition::opacity()),
    )
}

/// A hit's row in a list or the panel: its art, its name and a line under it, its play button,
/// its menu, and the drag that pins it.
fn hit_row(ui: &Ui, hit: &Hit, key: &str, seat: Seat, look: Look) -> El {
    let theme = &ui.theme;
    let place = key.to_owned();
    let subtitle = subtitle(ui, hit, &place, look.compact);
    let mut card = match hit {
        Hit::Song(track) => {
            let (current, _) = cards::track_status(ui, track);
            let tint = match current {
                true => theme.primary,
                false => theme.foreground,
            };
            let card = Card::new(key, track.name.clone())
                .art(art(ui, hit, row_art(theme), false))
                .tint(tint)
                .trailing(
                    text(clock(track.duration).to_string())
                        .font(ui::font(theme, Text::Small, Weight::NORMAL).features(TABULAR))
                        .color(theme.muted_foreground.color())
                        .a11y_hidden(),
                )
                .hint();
            match track.explicit {
                true => card.explicit(),
                false => card,
            }
        }
        Hit::Artist(artist) => Card::new(key, artist.name.clone())
            .art(art(ui, hit, row_art(theme), true))
            .circle(),
        Hit::Album(album) => {
            Card::new(key, album.name.clone()).art(art(ui, hit, row_art(theme), false))
        }
        Hit::Playlist(list) => {
            Card::new(key, list.name.clone()).art(art(ui, hit, row_art(theme), false))
        }
    };
    card = match subtitle {
        Subtitle::Plain(line) => card.meta(line, theme),
        Subtitle::Built(line) => card.bare_meta(line),
    };
    if let Some((playing, play)) = transport(ui, hit) {
        card = card.play(playing, play);
    }
    if let Some(target) = target(hit) {
        let panel = look.panel;
        card = card.on_press(act(move |ui: &mut Ui| {
            if panel {
                let query = ui.screens.search.edit.text.clone();
                remember(ui, &query);
                close_panel(ui);
            }
            open_target(ui, &target);
        }));
    }
    if let Some(menu) = menu_target(ui, hit) {
        card = card.menu(menus::at(menu));
    }
    if look.compact {
        // The mixed list tags each row with its kind, which the card's name leaves out.
        card = card.tucked().description(noun(hit.kind()));
    }
    let mut row = card
        .chosen(look.chosen)
        .build(theme)
        .position_in_set(seat.index + 1, seat.total);
    if let Some(pin) = hit.pin() {
        row = row.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            crate::chrome::sidebar_left::carry(ui, &pin, drag, tx)
        });
    }
    row
}

/// What a failed page's button does to try again.
type Retry = Box<dyn Fn(&mut Ui)>;

/// The line under a hit's name.
enum Subtitle {
    Plain(String),
    Built(El),
}

fn subtitle(ui: &Ui, hit: &Hit, place: &str, compact: bool) -> Subtitle {
    let theme = &ui.theme;
    if let (Hit::Album(album), false) = (hit, compact) {
        return Subtitle::Built(cards::released(
            &format!("{place}-artists"),
            album.year,
            None,
            &album.artist_refs,
            album.artists.clone(),
            theme,
        ));
    }
    let (artists, fallback) = match hit {
        Hit::Song(track) => (track.artist_refs.clone(), track.artists.clone()),
        Hit::Album(album) => (album.artist_refs.clone(), album.artists.clone()),
        Hit::Playlist(list) if !list.owner.is_empty() => (Vec::new(), list.owner.clone()),
        Hit::Artist(_) | Hit::Playlist(_) => return Subtitle::Plain(meta(hit, compact)),
    };
    let links = cells::artist_links(
        format!("{place}-artists"),
        &artists,
        fallback,
        theme.muted_foreground,
    )
    .text_size(theme.text(Text::Small))
    .truncate()
    .build(theme)
    .shrink()
    .min_w(0.);
    match compact {
        true => Subtitle::Built(
            row()
                .min_w(0.)
                .center_y()
                .gap(rems(theme, 0.25))
                .child(
                    text(tag(hit.kind()))
                        .font(ui::font(theme, Text::Small, Weight::NORMAL))
                        .color(theme.muted_foreground.color())
                        .a11y_hidden(),
                )
                .child(links),
        ),
        false => Subtitle::Built(links),
    }
}

/// A hit's art at `side`, round for an artist.
fn art(ui: &Ui, hit: &Hit, side: f32, circle: bool) -> El {
    let theme = &ui.theme;
    let mut cover = Cover::new(ui.artwork(), cover(hit)).size(side);
    cover = match circle {
        true => cover.circle().fallback("icons/user.svg"),
        false => cover.radius(theme.radius.min(4.)),
    };
    cover.build(theme)
}

/// The play button of a hit: a song pauses and resumes while it is the one playing and starts
/// its radio otherwise; anything else plays from itself.
fn transport(ui: &Ui, hit: &Hit) -> Option<(bool, impl Fn(&mut Ui, &mut Tx) + 'static)> {
    enum Play {
        Song(Box<Track>, bool),
        From(Origin),
    }
    let (playing, play) = match hit {
        Hit::Song(track) => {
            let (current, playing) = cards::track_status(ui, track);
            (playing, Play::Song(Box::new(track.clone()), current))
        }
        other => {
            let origin = match other {
                Hit::Artist(artist) => {
                    Origin::artist(artist.id.clone()?).named(artist.name.clone())
                }
                Hit::Album(album) => Origin::album(album.id.clone()).named(album.name.clone()),
                Hit::Playlist(list) => Origin::playlist(list.id.clone()).named(list.name.clone()),
                Hit::Song(_) => return None,
            };
            let playing = ui.store.playback.playing_from(&origin, &ui.store.queue) == Some(true);
            (playing, Play::From(origin))
        }
    };
    Some((playing, move |ui: &mut Ui, _: &mut Tx| {
        match &play {
            Play::Song(track, true) => {
                let _ = track;
                ui.store.toggle_play();
            }
            Play::Song(track, false) => ui.store.play_radio(track),
            Play::From(origin) => ui.store.toggle_collection(origin),
        }
        ui.settle();
    }))
}

/// What pressing a hit does.
#[derive(Clone)]
enum Press {
    Song(Box<Track>),
    Page(Destination),
}

fn target(hit: &Hit) -> Option<Press> {
    match hit {
        Hit::Song(track) => Some(Press::Song(Box::new(track.clone()))),
        Hit::Artist(artist) => artist
            .id
            .as_ref()
            .map(|id| Press::Page(Destination::Artist(id.as_str().into()))),
        Hit::Album(album) => Some(Press::Page(Destination::Album(album.id.as_str().into()))),
        Hit::Playlist(list) => Some(Press::Page(Destination::Playlist(list.id.as_str().into()))),
    }
}

/// A press on a hit's row: a song starts its radio, anything else opens its page.
fn open_target(ui: &mut Ui, target: &Press) {
    match target {
        Press::Song(track) => ui.store.play_radio(track),
        Press::Page(destination) => ui.go(destination.clone()),
    }
}

/// What choosing a hit from the keyboard does: a song plays its radio, or pauses and resumes
/// when it is the one playing, and anything else opens its page.
fn open_hit(ui: &mut Ui, hit: &Hit) {
    if let Hit::Song(track) = hit
        && cards::track_status(ui, track).0
    {
        ui.store.toggle_play();
        return;
    }
    if let Some(target) = target(hit) {
        open_target(ui, &target);
    }
}

/// The context menu's target for a hit, from the library's copy when it holds one.
fn menu_target(ui: &Ui, hit: &Hit) -> Option<Target> {
    match hit {
        Hit::Song(track) => Some(Target::track(track.clone())),
        Hit::Album(album) => Some(Target::Album(album_of(ui, album))),
        Hit::Playlist(list) => Some(Target::Playlist(playlist_of(ui, list))),
        Hit::Artist(artist) if artist.id.is_some() => Some(Target::artist(artist_of(ui, artist))),
        Hit::Artist(_) => None,
    }
}

fn album_of(ui: &Ui, hit: &AlbumHit) -> Album {
    ui.store
        .library
        .album(&hit.id)
        .cloned()
        .unwrap_or_else(|| Album {
            id: hit.id.clone(),
            name: hit.name.clone(),
            artists: hit.artists.clone(),
            artist_refs: hit.artist_refs.clone(),
            cover: hit.cover.clone(),
            cover_large: hit.cover.clone(),
            ..Default::default()
        })
}

fn playlist_of(ui: &Ui, hit: &PlaylistHit) -> Playlist {
    ui.store
        .library
        .playlist(&hit.id)
        .cloned()
        .unwrap_or_else(|| Playlist {
            id: hit.id.clone(),
            name: hit.name.clone(),
            owner: hit.owner.clone(),
            cover: hit.cover.clone(),
            ..Default::default()
        })
}

fn artist_of(ui: &Ui, hit: &ArtistHit) -> SavedArtist {
    let id = hit.id.clone().unwrap_or_default();
    ui.store
        .library
        .artist(&id)
        .cloned()
        .unwrap_or_else(|| SavedArtist {
            id,
            name: hit.name.clone(),
            cover: hit.cover.clone(),
            ..Default::default()
        })
}

// Handlers.

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.search
}

/// Where the field writes its edits back. A write while the field had no keyboard is the field
/// taking it, by a click or Tab, which brings the panel back; a focus the page moves there
/// itself sets `focused` first and so stays quiet.
fn field_access(ui: &mut Ui) -> &mut TextEdit {
    let state = st(ui);
    if !state.edit.focused {
        state.dismissed = false;
        state.pick = None;
        state.picked = None;
    }
    &mut state.edit
}

/// The text changed: suggest for it, and go back to browsing once it is emptied.
fn changed(ui: &mut Ui) {
    let typed = ui.screens.search.edit.text.clone();
    let state = st(ui);
    state.dismissed = false;
    state.pick = None;
    state.picked = None;
    ui.store.suggest(&typed);
    if typed.trim().is_empty() {
        ui.store.search("");
        reset_cursor(ui);
    }
}

/// Keys in the field: the arrows walk the panel and step down into the results, Return takes
/// the picked row or searches, and Escape puts the panel away and then lets go of the field.
fn field_key(ui: &mut Ui, event: &KeyEvent, tx: &mut Tx, stacked: bool) -> bool {
    let mods = event.mods;
    if mods.cmd || mods.alt || mods.ctrl || mods.shift {
        return false;
    }
    match event.key {
        Key::Down => select_next(ui, tx, stacked),
        Key::Up => panel_open(ui) && select_previous(ui, tx, stacked),
        Key::Enter => {
            submit(ui);
            true
        }
        Key::Escape => {
            if panel_open(ui) {
                close_panel(ui);
                return true;
            }
            tx.focus(KEYS, false);
            true
        }
        _ => false,
    }
}

/// Keys on the page while the field does not have them: the arrows walk the best match and the
/// lists, Return opens what they are on, and Escape goes back up to the field.
fn page_key(ui: &mut Ui, event: &KeyEvent, tx: &mut Tx, stacked: bool) -> bool {
    if ui.screens.search.edit.focused {
        return false;
    }
    let mods = event.mods;
    if mods.cmd || mods.alt || mods.ctrl || mods.shift {
        return false;
    }
    match event.key {
        Key::Down => select_next(ui, tx, stacked),
        Key::Up => select_previous(ui, tx, stacked),
        Key::Left => select_side(ui, tx, stacked, false),
        Key::Right => select_side(ui, tx, stacked, true),
        Key::Enter => activate(ui, stacked),
        Key::Escape => deselect(ui),
        _ => false,
    }
}

fn select_next(ui: &mut Ui, tx: &mut Tx, stacked: bool) -> bool {
    if panel_open(ui) {
        let offers = offers(ui);
        if let Some(last) = offers.len().checked_sub(1) {
            let at = picked_at(ui, &offers).map_or(0, |at| (at + 1).min(last));
            choose(ui, &offers, Some(at));
        }
        return true;
    }
    if asked(ui).is_empty() {
        // With no results to step into, Down asks for the panel the page opened without.
        let state = st(ui);
        if state.edit.focused && state.dismissed {
            state.dismissed = false;
            return true;
        }
        return false;
    }
    let best = ui
        .store
        .pages
        .search
        .as_ref()
        .and_then(Search::best)
        .is_some();
    let state = &ui.screens.search;
    match state.cursor {
        None if best && !state.on_best => st(ui).on_best = true,
        None => {
            let Some(column) = first_filled(ui, 0, stacked) else {
                return false;
            };
            st(ui).on_best = false;
            place(ui, column, 0);
        }
        Some((column, row)) => {
            let last = seats(ui, kinds(column, stacked)).len().saturating_sub(1);
            if row < last {
                place(ui, column, row + 1);
            }
        }
    }
    tx.focus(KEYS, false);
    reveal(ui, tx, stacked);
    true
}

fn select_previous(ui: &mut Ui, tx: &mut Tx, stacked: bool) -> bool {
    if panel_open(ui) {
        let offers = offers(ui);
        if let Some(at) = picked_at(ui, &offers) {
            choose(ui, &offers, at.checked_sub(1));
        }
        return true;
    }
    let Some((column, row)) = ui.screens.search.cursor else {
        if std::mem::take(&mut st(ui).on_best) {
            return_to_field(ui);
            return true;
        }
        return false;
    };
    if row == 0 {
        st(ui).cursor = None;
        match ui
            .store
            .pages
            .search
            .as_ref()
            .and_then(Search::best)
            .is_some()
        {
            true => {
                st(ui).on_best = true;
                reveal(ui, tx, stacked);
            }
            false => return_to_field(ui),
        }
        return true;
    }
    place(ui, column, row - 1);
    reveal(ui, tx, stacked);
    true
}

/// Left and Right hop to the next list that has rows, back to the row it was last left on.
fn select_side(ui: &mut Ui, tx: &mut Tx, stacked: bool, right: bool) -> bool {
    let Some((column, _)) = ui.screens.search.cursor else {
        return false;
    };
    let next = match right {
        true => first_filled(ui, column + 1, stacked),
        false => (0..column)
            .rev()
            .find(|&at| !seats(ui, kinds(at, stacked)).is_empty()),
    };
    let Some(next) = next else {
        return false;
    };
    let last = seats(ui, kinds(next, stacked)).len().saturating_sub(1);
    let row = ui
        .screens
        .search
        .rows
        .get(next)
        .copied()
        .unwrap_or(0)
        .min(last);
    place(ui, next, row);
    reveal(ui, tx, stacked);
    true
}

fn activate(ui: &mut Ui, stacked: bool) -> bool {
    if ui.screens.search.on_best {
        if let Some(best) = ui
            .store
            .pages
            .search
            .as_ref()
            .and_then(Search::best)
            .cloned()
        {
            open_hit(ui, &best);
        }
        return true;
    }
    let Some((column, row)) = ui.screens.search.cursor else {
        return false;
    };
    let Some(&(at, _)) = seats(ui, kinds(column, stacked)).get(row) else {
        return false;
    };
    let Some(hit) = hits(ui).get(at).cloned() else {
        return false;
    };
    open_hit(ui, &hit);
    true
}

/// Escape on the results: back up to the field. With nothing chosen it goes on to the window.
fn deselect(ui: &mut Ui) -> bool {
    let state = st(ui);
    let on_best = std::mem::take(&mut state.on_best);
    if state.cursor.take().is_none() && !on_best {
        return false;
    }
    return_to_field(ui);
    true
}

/// Scrolls the list the arrow keys are in so their row shows. The best match heads the mixed
/// list, so standing on it scrolls that list to its top.
fn reveal(ui: &Ui, tx: &mut Tx, stacked: bool) {
    let state = &ui.screens.search;
    if state.on_best && stacked {
        tx.scroll_to_y(MIXED, 0.);
        return;
    }
    let Some((column, row)) = state.cursor else {
        return;
    };
    let only = kinds(column, stacked);
    let Some(hit) = seats(ui, only)
        .get(row)
        .and_then(|&(at, _)| hits(ui).get(at))
    else {
        return;
    };
    let list = list_id(only);
    tx.scroll_into_view_in(list, format!("{list}-{}", hit_key(hit)));
}

/// Moves the keyboard back to the field without opening the panel.
fn return_to_field(ui: &mut Ui) {
    let state = st(ui);
    state.dismissed = true;
    state.edit.focused = true;
}

fn place(ui: &mut Ui, column: usize, row: usize) {
    let state = st(ui);
    if let Some(slot) = state.rows.get_mut(column) {
        *slot = row;
    }
    state.cursor = Some((column, row));
}

fn reset_cursor(ui: &mut Ui) {
    let state = st(ui);
    state.cursor = None;
    state.on_best = false;
    state.rows = [0; 3];
}

/// Puts the arrow keys on panel row `at` and notes which row that is.
fn choose(ui: &mut Ui, offers: &[Offer], at: Option<usize>) {
    let picked = at.and_then(|at| offers.get(at).map(Offer::key));
    let state = st(ui);
    state.pick = at;
    state.picked = picked;
}

fn close_panel(ui: &mut Ui) {
    let state = st(ui);
    state.dismissed = true;
    state.pick = None;
    state.picked = None;
}

/// Return in the field: the panel row the arrow keys are on, or else a full search for what is
/// typed.
fn submit(ui: &mut Ui) {
    let picked = match panel_open(ui) {
        true => {
            let offers = offers(ui);
            picked_at(ui, &offers).and_then(|at| offers.into_iter().nth(at))
        }
        false => None,
    };
    match picked {
        Some(offer) => take(ui, offer),
        None => {
            let query = ui.screens.search.edit.text.clone();
            run(ui, &query);
        }
    }
}

/// Searches by the resolved name of a pasted link, or for the query as typed.
fn run(ui: &mut Ui, query: &str) {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return;
    }
    let linked = ui
        .store
        .pages
        .suggest
        .as_ref()
        .filter(|suggest| suggest.query() == query)
        .and_then(Search::linked_query)
        .map(str::to_owned);
    search_text(ui, linked.as_deref().unwrap_or(&query));
}

/// Runs a full search and files the query in the history.
fn search_text(ui: &mut Ui, query: &str) {
    remember(ui, query);
    ui.store.search(query);
    close_panel(ui);
    reset_cursor(ui);
}

/// Acts on a panel row. A past query fills the field and runs again, a link opens, and a hit
/// opens the way it does in the results, filing what was typed as a search would.
fn take(ui: &mut Ui, offer: Offer) {
    match offer {
        Offer::Link(query) => {
            ui.store.open_link(&query);
            close_panel(ui);
        }
        Offer::Past(query) => {
            st(ui).edit.replace_text(query.clone());
            ui.store.suggest(&query);
            run(ui, &query);
        }
        Offer::Hit(hit) => {
            let typed = ui.screens.search.edit.text.clone();
            remember(ui, &typed);
            open_hit(ui, &hit);
            close_panel(ui);
        }
    }
}

fn forget(ui: &mut Ui, query: &str) {
    ui.store.settings.forget_search(query, &mut ui.store.cx);
    let state = st(ui);
    state.pick = None;
    state.picked = None;
}

/// Files `query` at the top of the search history.
fn remember(ui: &mut Ui, query: &str) {
    ui.store.settings.remember_search(query, &mut ui.store.cx);
}

// Reading the page's state.

/// What the field holds, trimmed.
fn query(ui: &Ui) -> &str {
    ui.screens.search.edit.text.trim()
}

/// The query the results answer, untrimmed as the store keeps it.
fn asked_query(ui: &Ui) -> &str {
    ui.store.pages.search.as_ref().map_or("", Search::query)
}

fn asked(ui: &Ui) -> &str {
    asked_query(ui).trim()
}

fn hits(ui: &Ui) -> &[Hit] {
    ui.store.pages.search.as_ref().map_or(&[], Search::hits)
}

/// The rows of the panel: past queries while the field is empty, a pasted link to open, and
/// the best suggested hits once something is typed.
fn offers(ui: &Ui) -> Vec<Offer> {
    let typed = query(ui);
    if typed.is_empty() {
        return ui
            .store
            .settings
            .searches()
            .iter()
            .take(PAST)
            .cloned()
            .map(Offer::Past)
            .collect();
    }
    let suggest = ui.store.pages.suggest.as_ref();
    let mut offers = Vec::new();
    if ui.store.links.recognizes(typed, &ui.store.session) {
        offers.push(Offer::Link(typed.to_owned()));
        if suggest.and_then(Search::linked_query).is_none() {
            return offers;
        }
    }
    if let Some(suggest) = suggest {
        offers.extend(
            suggest
                .hits()
                .iter()
                .take(SUGGESTED)
                .map(|hit| Offer::Hit(Box::new(hit.clone()))),
        );
    }
    offers
}

/// Whether the panel shows: the field has the keyboard, nothing put the panel away, and it has
/// rows or is still waiting on the first suggestions.
fn panel_open(ui: &Ui) -> bool {
    let state = &ui.screens.search;
    if state.dismissed || !state.edit.focused {
        return false;
    }
    let waiting = !query(ui).is_empty()
        && ui
            .store
            .pages
            .suggest
            .as_ref()
            .is_some_and(Search::is_loading);
    waiting || !offers(ui).is_empty()
}

/// The panel row the arrow keys are on: the row they were put on, followed as the suggestions
/// reorder, or the same place when that row is gone.
fn picked_at(ui: &Ui, offers: &[Offer]) -> Option<usize> {
    let state = &ui.screens.search;
    let at = state.pick?;
    let found = state
        .picked
        .as_ref()
        .and_then(|key| offers.iter().position(|offer| offer.key() == *key));
    found.or_else(|| offers.len().checked_sub(1).map(|last| at.min(last)))
}

/// The key of the row the arrow keys are on in the results, which the page names as its active
/// descendant for screen readers.
fn chosen_key(ui: &Ui, stacked: bool) -> Option<String> {
    let state = &ui.screens.search;
    if state.on_best {
        return Some("search-best-card".into());
    }
    let (column, row) = state.cursor?;
    let only = kinds(column, stacked);
    let (at, _) = *seats(ui, only).get(row)?;
    let hit = hits(ui).get(at)?;
    Some(format!("{}-{}", list_id(only), hit_key(hit)))
}

/// Where each hit of the `only` kinds sits: its index among all hits and its place among the
/// hits of its kind. The first hit is the best match drawn above the lists, so the lists leave
/// it out.
fn seats(ui: &Ui, only: &[Kind]) -> Vec<(usize, usize)> {
    let mut taken = [0; Kind::ALL.len()];
    hits(ui)
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(at, hit)| {
            let kind = hit.kind();
            let place = taken[seat(kind)];
            taken[seat(kind)] += 1;
            only.contains(&kind).then_some((at, place))
        })
        .collect()
}

fn first_filled(ui: &Ui, start: usize, stacked: bool) -> Option<usize> {
    (start..columns(stacked)).find(|&column| !seats(ui, kinds(column, stacked)).is_empty())
}

// Free helpers.

/// The entrance of a section of results: up from a little below, fading in. It leaves by
/// fading where it stood, while the next set comes in over it.
fn entrance() -> Transition {
    Transition::asymmetric(
        Transition::offset(0., RISE).combined(Transition::opacity()),
        Transition::opacity(),
    )
}

/// A short tag for a query, so each new set of results enters afresh.
fn generation(query: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.trim().hash(&mut hasher);
    hasher.finish()
}

fn row_art(theme: &Theme) -> f32 {
    (theme.metrics.list_row.round() - theme.metrics.pad * 2.).round()
}

/// A stable key for a hit, from its id where it has one.
fn hit_key(hit: &Hit) -> String {
    match hit {
        Hit::Song(track) => format!("song-{}", track.id.as_deref().unwrap_or(&track.name)),
        Hit::Artist(artist) => {
            format!("artist-{}", artist.id.as_deref().unwrap_or(&artist.name))
        }
        Hit::Album(album) => format!("album-{}", album.id),
        Hit::Playlist(list) => format!("playlist-{}", list.id),
    }
}

fn list_id(only: &[Kind]) -> &'static str {
    match only {
        [Kind::Song] => "search-songs",
        [Kind::Artist] => "search-artists",
        [Kind::Album, Kind::Playlist] => "search-albums",
        _ => MIXED,
    }
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

fn cover(hit: &Hit) -> Option<&str> {
    match hit {
        Hit::Song(track) => track.cover.as_deref(),
        Hit::Artist(artist) => artist.cover.as_deref(),
        Hit::Album(album) => album.cover.as_deref(),
        Hit::Playlist(list) => list.cover.as_deref(),
    }
}

fn meta(hit: &Hit, compact: bool) -> String {
    match hit {
        Hit::Song(track) => track.artists.clone(),
        Hit::Album(album) => album.artists.clone(),
        Hit::Artist(artist) => match compact {
            true => noun(Kind::Artist),
            false => held(artist.saved),
        },
        Hit::Playlist(list) => match list.owner.is_empty() {
            true => noun(Kind::Playlist),
            false => list.owner.clone(),
        },
    }
}

fn tag(kind: Kind) -> String {
    let noun = noun(kind);
    t!("search-tag", kind = &noun).into_owned()
}

fn held(saved: usize) -> String {
    match saved {
        0 => noun(Kind::Artist),
        count => t!("search-saved", count = count).into_owned(),
    }
}

fn noun(kind: Kind) -> String {
    match kind {
        Kind::Song => t!("kind-song"),
        Kind::Artist => t!("kind-artist"),
        Kind::Album => t!("kind-album"),
        Kind::Playlist => t!("kind-playlist"),
    }
    .into_owned()
}
