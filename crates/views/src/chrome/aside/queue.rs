//! The Queue panel: what played, what plays now and what comes next, under headings wherever
//! the songs' provenance changes. Rows are virtual, so a queue of thousands only builds what
//! shows. A song queued by hand names who queued it, an upcoming song drags to a new place
//! (its neighbours step aside as it passes) or goes with its remove button, and the list keeps
//! the playing song a quarter of the way down as it changes.

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i18n::t;
use music::{Contributor, Shape, Track};
use router::{Destination, LibraryTab};
use rune::{
    Cursor, Cx, Drag, El, GrowthSpec, Live, Phase, ScrollPos, ScrollSpec, TextStyle, Tx, col, el,
    row, text,
};
use state::{QueuePosition, QueueSections, QueueSlot, QueueView, Shelf, SideTab, Whence};
use ui::{
    Button, Card, Cover, Initials, InlineLink, InlineLinks, Menu, MenuItem, Popup, Springs, Text,
    Theme, Weight,
};

use crate::{Ui, act, act_tx};

/// The queue's scroll viewport, which the anchor and the recall button glide.
const SCROLL: &str = "queue-scroll";
/// The blank band over the first row, which the edge fade covers.
const TOP: f32 = 48.;
/// Blank rows after the last queued song.
const TAIL_ROWS: f32 = 1.;
/// Where the playing song rests, as a share of the viewport from its top.
const PINNED_SHARE: f32 = 0.25;
/// How many text heights across the picture of whoever queued a song is.
const ADDER_FACE: f32 = 1.4;
const BULLET: &str = "·";
/// How far the pointer travels before a press on a row becomes a drag.
const SLOP: f32 = 4.;
/// How long after a drop the release's press is ignored, so dropping a row never plays it.
const DROPPED: Duration = Duration::from_millis(250);
/// The gap between a row's cover and the end of its meta line, and inside the adder.
const META_GAP: f32 = 4.;
/// The space under a heading's caption.
const HEADING_FOOT: f32 = 4.;
/// The room the remove button keeps from the row's end.
const REMOVE_INSET: f32 = 4.;
/// How far a scrolled-away queue has to drift before the way back shows, in rows.
const REACH_ROWS: f32 = 3.;
/// Where the hidden recall button waits, below its perch.
const TUCKED: f32 = 64.;

/// The Queue panel's own view state.
pub struct State {
    menu: Option<RowMenu>,
    drag: Option<Dragging>,
    /// How far the dragged row is drawn from its place, following the pointer.
    carry: Live<[f32; 2]>,
    /// When a drag last ended, so the press that comes with its release does nothing.
    dropped: Option<Instant>,
    /// Whether the next poll puts the playing song back in its place, and whether it jumps there.
    anchor: bool,
    instant: bool,
    /// What the anchor last followed: how much history there was and which song played.
    past: Option<usize>,
    playing: Option<Option<String>>,
    /// The viewport's offset, furthest offset and height, from its scroll reports.
    scroll: Live<[f32; 4]>,
    /// The playing row's top in the list and the row height, for the recall button's goal.
    resting: Rc<Cell<Option<(f32, f32)>>>,
    /// The recall button's opacity and its lift off its perch.
    away: Live<f32>,
    lift: Live<[f32; 2]>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            menu: None,
            drag: None,
            carry: Live::new([0., 0.]),
            dropped: None,
            anchor: true,
            instant: true,
            past: None,
            playing: None,
            scroll: Live::new([0.; 4]),
            resting: Rc::new(Cell::new(None)),
            away: Live::new(0.),
            lift: Live::new([0., TUCKED]),
        }
    }
}

impl State {
    /// Puts the playing song back in its place, at once, as the panel comes into view.
    pub(crate) fn anchor(&mut self) {
        self.anchor = true;
        self.instant = true;
    }

    /// Closes the row's context menu.
    pub(crate) fn dismiss(&mut self) {
        self.menu = None;
    }

    /// How far the upcoming row at `index` steps aside for the row being dragged past it.
    fn shift(&self, index: usize, row: f32) -> f32 {
        let Some(drag) = self.drag.as_ref().filter(|drag| drag.moved) else {
            return 0.;
        };
        match drag.gap {
            Some(gap) if gap > drag.from && index > drag.from && index < gap => -row,
            Some(gap) if gap < drag.from && index >= gap && index < drag.from => row,
            _ => 0.,
        }
    }
}

/// The context menu of one row, open at the point it was asked for.
struct RowMenu {
    track: Track,
    uid: Option<String>,
    upcoming: bool,
    revision: u64,
    at: (f32, f32),
}

/// An upcoming song held by the pointer.
struct Dragging {
    uid: String,
    /// Its place among the upcoming songs, and its slot in the list.
    from: usize,
    slot: usize,
    /// The queue's revision when it was picked up; a drop on a changed queue does nothing.
    revision: u64,
    /// Where in its row it was grabbed, and the viewport's offset then.
    grab: f32,
    offset: f32,
    /// The gap it would land in now, if that moves it.
    gap: Option<usize>,
    moved: bool,
}

/// What a row of the list holds: the blank band at either end, a heading or a song.
enum Line {
    Top,
    Tail,
    Slot(QueueSlot),
}

pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let listing = ui.store.shown_queue();
    let sections = listing.sections.clone();
    let remote = !listing.editable();
    let state = &ui.chrome.aside.queue;

    let header = super::header(
        SideTab::Queue,
        (!remote).then(|| actions(ui, &sections, theme)),
        theme,
    );
    let body = match sections.is_empty() {
        true => ui::spoken_vacant("queue-empty", t!("queue-empty").to_string(), theme).grow(1.),
        false => {
            let row = theme.metrics.list_row.round();
            state.resting.set(Some((
                sections
                    .current_index()
                    .map_or(-1., |index| row * index as f32),
                row,
            )));
            col()
                .key("queue-body")
                .grow(1.)
                .min_h(0.)
                .w_full()
                .child(rows(ui, &listing, theme))
                .child(recall(ui, &sections, theme))
        }
    };
    if let Some(menu) = state.menu.as_ref().filter(|menu| {
        // a menu opened on a queue that has since changed would act on the wrong song
        menu.revision == listing.revision()
    }) {
        row_menu(menu, theme, layer);
    }

    col()
        .w_full()
        .h_full()
        .min_h(0.)
        .min_w(0.)
        .child(header)
        .child(body)
}

/// Follows the queue: a new playing song or a change in history puts the playing song back in
/// its place. Nothing here changes what the view shows, so it never asks for a view run.
pub(crate) fn poll(ui: &mut Ui, shown: bool, tx: &mut Tx) -> bool {
    let row = ui.theme.metrics.list_row.round();
    let listing = ui.store.shown_queue();
    let past = listing.sections.past;
    let current = listing.sections.current_index();
    let playing = listing
        .track(QueuePosition::Current)
        .and_then(|track| track.id);
    let state = &mut ui.chrome.aside.queue;
    if state.past != Some(past) || state.playing.as_ref() != Some(&playing) {
        state.past = Some(past);
        state.playing = Some(playing);
        state.anchor = true;
    }
    if !shown || !state.anchor || state.drag.is_some() {
        return false;
    }
    let Some(index) = current else {
        state.anchor = false;
        return false;
    };
    let viewport = state.scroll.get()[2];
    if viewport <= 0. {
        // not laid out yet: try again once it is
        tx.wake_after(Duration::from_millis(50));
        return false;
    }
    let goal = resting_offset(row * index as f32, row, viewport, f32::MAX);
    match std::mem::take(&mut state.instant) {
        true => tx.scroll_to_instant(SCROLL, 0., goal),
        false => tx.scroll_to_with(SCROLL, 0., goal, Springs::SLOW),
    }
    state.anchor = false;
    false
}

/// The controls in the header over this app's own queue: autoplay where a station can keep the
/// queue going, putting the queue back in its order, and clearing what is queued.
fn actions(ui: &Ui, sections: &QueueSections, theme: &Theme) -> El {
    let radio = ui.store.playback.radio();
    let muted = theme.muted_foreground;
    let mut group = row().center_y().gap(theme.metrics.pad / 2.);
    if ui.store.session.capabilities().radio {
        group = group.child(
            Button::new("toggle-radio")
                .ghost()
                .small()
                .icon("icons/radio.svg")
                .tooltip(t!("queue-radio").to_string())
                .toggled(radio)
                .tint(match radio {
                    true => theme.primary,
                    false => muted,
                })
                .on_press(act(|ui: &mut Ui| ui.store.toggle_radio()))
                .build(theme),
        );
    }
    group
        .child(
            Button::new("reset-queue")
                .ghost()
                .small()
                .label(t!("queue-reset").to_string())
                .tint(muted)
                .disabled(!ui.store.queue.reordered())
                .on_press(act(|ui: &mut Ui| {
                    let state::Store { queue, cx, .. } = &mut ui.store;
                    queue.reset(cx);
                }))
                .build(theme),
        )
        .child(
            Button::new("clear-queue")
                .ghost()
                .small()
                .label(t!("queue-clear").to_string())
                .tint(muted)
                .disabled(sections.upcoming == 0)
                .on_press(act(|ui: &mut Ui| {
                    let state::Store { queue, cx, .. } = &mut ui.store;
                    queue.clear_upcoming(cx);
                }))
                .build(theme),
        )
}

/// The list: a blank band, every heading and song, and a blank row, built a window at a time.
fn rows(ui: &Ui, listing: &QueueView<'_>, theme: &Theme) -> El {
    let row = theme.metrics.list_row.round();
    let count = listing.sections.len() + 2;
    let mut stack = col()
        .key("queue-rows")
        .w_full()
        .growth(GrowthSpec {
            spring: Springs::BASE,
            edge: rune::edge::NO_MASK,
        })
        .virtual_rows(
            count,
            row,
            |ui: &Ui, index| line_key(ui, index),
            |ui: &Ui, index| line(ui, index),
        );
    if listing.editable() {
        let sections = &listing.sections;
        stack = stack.virtual_rows_version((
            ui.store.queue.revision(),
            count,
            sections.past,
            sections.current,
        ));
    }
    let scroll = ui.chrome.aside.queue.scroll.clone();
    let resting = ui.chrome.aside.queue.resting.clone();
    let (away, lift) = (
        ui.chrome.aside.queue.away.clone(),
        ui.chrome.aside.queue.lift.clone(),
    );
    let reach = theme.metrics.list_row * REACH_ROWS;
    let mut viewport = col()
        .key(SCROLL)
        .role("list")
        .label(t!("queue-title").to_string())
        .grow(1.)
        .min_h(0.)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(move |at: ScrollPos| {
            let [offset, max, height] = [at.offset[1], at.max[1], at.viewport[1]];
            scroll.set([offset, max, height, 0.]);
            let goal = match resting.get() {
                Some((top, row)) if top >= 0. => resting_offset(top, row, height, max),
                _ => 0.,
            };
            let far = (offset - goal).abs() > reach.min(height / 2.);
            away.set(match far {
                true => 1.,
                false => 0.,
            });
            lift.set(match far {
                true => [0., 0.],
                false => [0., TUCKED],
            });
        })
        .child(stack);
    if ui::effects() {
        viewport = viewport.scroll_edge_fade(TOP / 2.);
    }
    viewport
}

/// Where the list rests with the row whose top is `top` a quarter of the way down a viewport
/// `viewport` tall, clamped to the list's ends.
fn resting_offset(top: f32, row: f32, viewport: f32, max: f32) -> f32 {
    let above = (viewport * PINNED_SHARE / row).round() * row;
    (top - above).clamp(0., max.max(0.))
}

/// The button that glides back to the playing song, or to the top while nothing plays, once the
/// list has drifted far enough from it. It slides in and out on live values, without a view run.
fn recall(ui: &Ui, sections: &QueueSections, theme: &Theme) -> El {
    let state = &ui.chrome.aside.queue;
    let tooltip = match sections.current {
        true => t!("queue-return-playing"),
        false => t!("nav-return-top"),
    };
    let scroll = state.scroll.clone();
    let resting = state.resting.clone();
    let mut button = Button::new("queue-return-top")
        .secondary()
        .small()
        .icon("icons/undo-2.svg")
        .tooltip(tooltip.to_string())
        .on_press_tx(move |_: &mut Ui, tx: &mut Tx| {
            let [_, max, height, _] = scroll.get();
            let goal = match resting.get() {
                Some((top, row)) if top >= 0. => resting_offset(top, row, height, max),
                _ => 0.,
            };
            tx.scroll_to_with(SCROLL, 0., goal, Springs::SLOW);
        })
        .build(theme);
    if ui::blurring(theme) {
        button = ui::glass(button, theme);
    }
    ui::perched(
        el().live_opacity(state.away.smooth(Springs::BASE))
            .live_offset(state.lift.smooth(Springs::BASE))
            .child(button),
    )
}

/// What row `index` of the list holds.
fn line_at(sections: &QueueSections, index: usize) -> Line {
    match index.checked_sub(1) {
        None => Line::Top,
        Some(slot) => match sections.slot(slot) {
            Some(slot) => Line::Slot(slot),
            None => Line::Tail,
        },
    }
}

/// A row's key: a song's comes from its place in play order and its queue entry, which it keeps
/// while it moves, and a heading's from the song under it.
fn line_key(ui: &Ui, index: usize) -> Cow<'_, str> {
    let listing = ui.store.shown_queue();
    match line_at(&listing.sections, index) {
        Line::Top => "queue-top".into(),
        Line::Tail => "queue-tail".into(),
        Line::Slot(QueueSlot::Track(position)) => track_key(ui, &listing, position, index).into(),
        Line::Slot(QueueSlot::Header(heading)) => match listing.sections.slot(index) {
            Some(QueueSlot::Track(position @ QueuePosition::Upcoming(_))) => {
                format!("head-{}", track_key(ui, &listing, position, index + 1)).into()
            }
            _ => format!("head-{heading}").into(),
        },
    }
}

fn track_key(ui: &Ui, listing: &QueueView<'_>, position: QueuePosition, index: usize) -> String {
    let uid = listing
        .editable()
        .then(|| ui.store.queue.uid(position))
        .flatten();
    match (uid, position) {
        (Some(uid), QueuePosition::Past(_)) => format!("past-{uid}"),
        (Some(uid), QueuePosition::Current) => format!("now-{uid}"),
        (Some(uid), QueuePosition::Upcoming(_)) => format!("next-{uid}"),
        (None, _) => format!("slot-{index}"),
    }
}

/// Builds row `index` of the list.
fn line(ui: &Ui, index: usize) -> El {
    let theme = &ui.theme;
    let listing = ui.store.shown_queue();
    match line_at(&listing.sections, index) {
        Line::Top => el().w_full().h(TOP).a11y_hidden(),
        Line::Tail => el()
            .w_full()
            .h(theme.metrics.list_row.round() * TAIL_ROWS)
            .a11y_hidden(),
        Line::Slot(QueueSlot::Header(heading)) => section(ui, &listing, heading, index, theme),
        Line::Slot(QueueSlot::Track(position)) => track_row(ui, &listing, position, index, theme),
    }
}

/// A section's heading. Over the playing song it also names the collection the song was queued
/// from, which opens its page.
fn section(
    ui: &Ui,
    listing: &QueueView<'_>,
    heading: &'static str,
    index: usize,
    theme: &Theme,
) -> El {
    let label = i18n::lookup(heading, None).to_string();
    let mut head = row()
        .key(format!("queue-section-{heading}-{index}"))
        .role("heading")
        .level(2)
        .label(label.clone())
        .w_full()
        .min_w(0.)
        .h(theme.metrics.list_row.round())
        .items_end()
        .gap(theme.metrics.pad / 2.)
        .px(theme.metrics.pad)
        .pb(HEADING_FOOT)
        .child(ui::eyebrow(&label, theme).a11y_hidden());
    let from = match heading {
        "queue-now-playing" => listing.origin().and_then(|origin| playing_from(ui, origin)),
        _ => None,
    };
    if let Some((name, place)) = from {
        let small = ui::font(theme, Text::Small, Weight::SEMIBOLD);
        let quoted = t!("queue-from").to_string();
        head = head
            .child(
                text(BULLET)
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .a11y_hidden(),
            )
            .child(
                text(quoted.clone())
                    .key("queue-from")
                    .role("text")
                    .label(quoted)
                    .font(small)
                    .color(theme.muted_foreground.color()),
            )
            .child(
                text(name.clone())
                    .key("queue-source")
                    .role("link")
                    .label(name)
                    .font(small)
                    .color(theme.muted_foreground.color())
                    .truncate()
                    .shrink()
                    .min_w(0.)
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .on_press(act(move |ui: &mut Ui| ui.go(place.clone()))),
            );
    }
    head
}

/// The name and page of the collection the playing song was queued from. `None` for a song
/// added on its own or an origin without a name, so the heading names no source.
fn playing_from(ui: &Ui, origin: &state::Origin) -> Option<(String, Destination)> {
    let id: router::SharedString = origin.id.as_str().into();
    let place = match origin.whence {
        Whence::Album => Destination::Album(id),
        Whence::Playlist => Destination::Playlist(id),
        Whence::Artist => Destination::Artist(id),
        Whence::Radio => Destination::Song(id),
        Whence::Saved => Destination::Library(LibraryTab::Songs),
        Whence::Local => Destination::Local(LibraryTab::Songs),
    };
    let name = match origin.whence {
        Whence::Saved => match ui.store.library.shape(Shelf::Streaming) {
            Shape::Saved => t!("library-liked-songs").to_string(),
            Shape::Catalog(_) => t!("nav-songs").to_string(),
        },
        Whence::Local => t!("nav-local").to_string(),
        Whence::Radio => t!("queue-from-radio", name = origin.name.as_deref()?).to_string(),
        Whence::Album | Whence::Playlist | Whence::Artist => origin.name.clone()?,
    };
    Some((name, place))
}

/// One song: its cover with a play button, its title tinted by where it stands in play order,
/// its artists and who queued it. An upcoming song of this app's own queue can be removed and
/// dragged to a new place; another device's queue shows read only.
fn track_row(
    ui: &Ui,
    listing: &QueueView<'_>,
    position: QueuePosition,
    index: usize,
    theme: &Theme,
) -> El {
    let Some(track) = listing.track(position) else {
        return el().w_full().h(theme.metrics.list_row.round());
    };
    let sections = &listing.sections;
    let remote = !listing.editable();
    let revision = listing.revision();
    let current = position == QueuePosition::Current;
    let playing = listing.playing && current;
    let upcoming = position.upcoming().filter(|_| !remote);
    let uid = (!remote)
        .then(|| ui.store.queue.uid(position).map(str::to_owned))
        .flatten();
    let key = track_key(ui, listing, position, index);
    let state = &ui.chrome.aside.queue;
    let row_height = theme.metrics.list_row.round();

    let tint = match position {
        QueuePosition::Past(_) => theme.muted_foreground,
        QueuePosition::Current => theme.primary,
        QueuePosition::Upcoming(_) => theme.foreground,
    };
    let queued = sections.kind(position) == Some(state::QueueKind::Queued);
    let added = track
        .added_by
        .as_deref()
        .filter(|added| queued && !added.name.is_empty() && added.name != added.id);
    let about = match added {
        Some(added) => format!(
            "{}, {}",
            track.artists,
            t!("queue-added-by", name = added.name.as_str())
        ),
        None => track.artists.clone(),
    };

    let art_side = (row_height - theme.metrics.pad * 2.).round();
    let art = Cover::new(ui.artwork(), track.cover.as_deref())
        .size(art_side)
        .radius(theme.radius.min(4.))
        .fallback("icons/music.svg")
        .build(theme);
    let artists = InlineLinks::<Ui>::new(
        format!("queue-track-artist-{key}"),
        track
            .artist_refs
            .iter()
            .map(|artist| InlineLink::new(artist.name.clone(), artist.id.clone())),
        track.artists.clone(),
        theme.muted_foreground,
    )
    .text_size(theme.text(Text::Small))
    .truncate()
    .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| ui.go(Destination::Artist(id.into())))
    .build(theme)
    .a11y_hidden();
    let meta = row()
        .center_y()
        .gap(META_GAP)
        .min_w(0.)
        .child(artists)
        .children(added.map(|added| adder(ui, added, theme)));

    let mut card = Card::new(format!("queue-track-{key}"), track.name.clone())
        .description(about)
        .art(art)
        .bare_meta(meta)
        .tint(tint);
    if track.explicit {
        card = card.explicit();
    }
    if !remote || current {
        let uid = uid.clone();
        card = card.play(
            playing,
            act_tx(
                move |ui: &mut Ui, _: &mut Tx| match (remote, uid.as_deref()) {
                    (true, _) => ui.store.toggle_play(),
                    (false, Some(uid)) => ui.store.play_queued(uid),
                    _ => {}
                },
            ),
        );
    }
    {
        let track = track.clone();
        let uid = uid.clone();
        card = card.menu(move |ui: &mut Ui, press: rune::PointerPress, _: &mut Tx| {
            ui.chrome.aside.queue.menu = Some(RowMenu {
                track: track.clone(),
                uid: uid.clone(),
                upcoming: upcoming.is_some(),
                revision,
                at: (press.x, press.y),
            });
        });
    }
    if !remote
        && !current
        && let Some(uid) = uid.clone()
    {
        card = card.on_press(act(move |ui: &mut Ui| {
            let dropped = ui.chrome.aside.queue.dropped.take();
            if dropped.is_some_and(|at| at.elapsed() < DROPPED) {
                return;
            }
            ui.store.play_queued(&uid);
        }));
    }
    if upcoming.is_some()
        && let Some(uid) = uid.clone()
    {
        card = card.action(
            Button::new(format!("remove-queued-track-{key}"))
                .ghost()
                .small()
                .icon("icons/x.svg")
                .tooltip(t!("menu-remove-from-queue").to_string())
                .tint(theme.muted_foreground)
                .on_press(act(move |ui: &mut Ui| ui.store.remove_queued(&uid)))
                .build(theme)
                .mr(REMOVE_INSET),
        );
    }
    let mut card = card.build(theme);

    let dragged = upcoming.is_some()
        && state
            .drag
            .as_ref()
            .is_some_and(|drag| drag.moved && Some(&drag.uid) == uid.as_ref());
    if let (Some(_), Some(uid)) = (upcoming, uid) {
        card = card
            .on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| drag_row(ui, &uid, drag, tx));
    }
    if dragged {
        card = card
            .fill(theme.secondary.color())
            .shadow(4., 16., ui::black().opacity(0.3).color());
    }

    let (place, total) = track_place(sections, position);
    let mut line = el()
        .role("listitem")
        .position_in_set(place, total)
        .w_full()
        .px(theme.metrics.pad)
        .child(card);
    if dragged {
        line = line.live_offset(&state.carry).raise();
    } else if let Some(at) = upcoming {
        line = line
            .offset(0., state.shift(at, row_height))
            .offset_motion(Springs::QUICK);
    }
    line
}

/// The part of a row that names who queued the song: a bullet, their round picture or initials,
/// and "Added by NAME".
fn adder(ui: &Ui, added: &Contributor, theme: &Theme) -> El {
    let face = theme.text(Text::Small) * ADDER_FACE;
    let style = TextStyle::new(theme.text(Text::Small), Weight::NORMAL);
    let muted = theme.muted_foreground.color();
    let picture = match &added.avatar {
        Some(url) => ui::Avatar::new(ui.artwork(), Some(url.as_str()))
            .size(face)
            .build(theme),
        None => Initials::new(added.name.clone(), face).build(theme),
    };
    row()
        .center_y()
        .gap(META_GAP)
        .shrink()
        .min_w(0.)
        .child(text(BULLET).font(style).color(muted).a11y_hidden())
        .child(picture)
        .child(
            text(t!("queue-added-by", name = added.name.as_str()).to_string())
                .font(style)
                .color(muted)
                .truncate()
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        )
}

/// Where the song at `position` stands among every song the queue lists, counted from one and
/// leaving out the headings, and how many songs that is.
fn track_place(sections: &QueueSections, position: QueuePosition) -> (usize, usize) {
    let current = usize::from(sections.current);
    let before = match position {
        QueuePosition::Past(at) => at,
        QueuePosition::Current => sections.past,
        QueuePosition::Upcoming(at) => sections.past + current + at,
    };
    (before + 1, sections.past + current + sections.upcoming)
}

/// The slot `position` sits in.
fn slot_of(sections: &QueueSections, position: QueuePosition) -> Option<usize> {
    (0..sections.len()).find(|slot| sections.slot(*slot) == Some(QueueSlot::Track(position)))
}

/// Follows a drag of the upcoming song `uid`. The row rides the pointer on a live offset, so a
/// move only runs the view when the gap it would land in changes, and then the rows it passes
/// spring aside to open that gap. A release moves the song there.
fn drag_row(ui: &mut Ui, uid: &str, drag: &mut Drag, tx: &mut Tx) {
    let row = ui.theme.metrics.list_row.round();
    match drag.phase {
        Phase::Down => {
            let Some(QueuePosition::Upcoming(from)) = ui.store.queue.position(uid) else {
                return;
            };
            let sections = ui.store.shown_queue().sections;
            let Some(slot) = slot_of(&sections, QueuePosition::Upcoming(from)) else {
                return;
            };
            let state = &mut ui.chrome.aside.queue;
            state.carry.set([0., 0.]);
            state.drag = Some(Dragging {
                uid: uid.to_owned(),
                from,
                slot,
                revision: ui.store.queue.revision(),
                grab: drag.y,
                offset: state.scroll.get()[0],
                gap: None,
                moved: false,
            });
            tx.live_only();
        }
        Phase::Move => {
            let sections = ui.store.shown_queue().sections;
            let state = &mut ui.chrome.aside.queue;
            let scrolled = state.scroll.get()[0];
            let Some(held) = state.drag.as_mut() else {
                return;
            };
            if !held.moved && drag.dx.abs().max(drag.dy.abs()) < SLOP {
                tx.live_only();
                return;
            }
            let travel = drag.dy + scrolled - held.offset;
            state.carry.set([0., travel]);
            let gap = gap_at(&sections, held, travel, row);
            match (held.moved, gap == held.gap) {
                (true, true) => tx.live_only(),
                _ => {
                    held.moved = true;
                    held.gap = gap;
                    tx.animated();
                }
            }
        }
        Phase::Up => {
            let state = &mut ui.chrome.aside.queue;
            state.carry.set([0., 0.]);
            let Some(held) = state.drag.take() else {
                return;
            };
            if !held.moved {
                return;
            }
            state.dropped = Some(Instant::now());
            if !drag.cancelled()
                && let Some(gap) = held.gap
            {
                ui.store.move_queued(held.revision, held.from, gap);
                ui.settle();
            }
            // the rows that stepped aside are where the move puts them, so nothing travels
            tx.instant();
        }
    }
}

/// The gap among the upcoming songs a row picked up from `held` lands in after travelling
/// `travel` points: before the song under the pointer's upper half, after one under its lower
/// half, before the first song of a section whose heading is under it. `None` where the song
/// would stay where it is.
fn gap_at(sections: &QueueSections, held: &Dragging, travel: f32, row: f32) -> Option<usize> {
    let along = (held.grab + travel) / row;
    let slot = held.slot as f32 + along.floor();
    let lower = along - along.floor() >= 0.5;
    let gap = match slot < 0. {
        true => 0,
        false => {
            let slot = slot as usize;
            match sections.slot(slot) {
                None => sections.upcoming,
                Some(QueueSlot::Track(QueuePosition::Upcoming(at))) => at + usize::from(lower),
                Some(QueueSlot::Track(_)) => 0,
                Some(QueueSlot::Header(_)) => (slot + 1..sections.len())
                    .find_map(|next| match sections.slot(next) {
                        Some(QueueSlot::Track(QueuePosition::Upcoming(at))) => Some(at),
                        _ => None,
                    })
                    .unwrap_or(sections.upcoming),
            }
        }
    };
    (gap != held.from && gap != held.from + 1).then_some(gap)
}

/// A row's context menu: play it next or at the end, open its album or artist, take it out of
/// the queue.
fn row_menu(menu: &RowMenu, theme: &Theme, layer: &mut Vec<El>) {
    let track = &menu.track;
    let mut items = Vec::new();
    {
        let track = track.clone();
        items.push(
            MenuItem::new("queue-menu-play-next", t!("menu-play-next").to_string())
                .icon("icons/list-start.svg")
                .on_press(act(move |ui: &mut Ui| ui.store.play_next(track.clone()))),
        );
    }
    {
        let track = track.clone();
        items.push(
            MenuItem::new("queue-menu-add", t!("menu-add-to-queue").to_string())
                .icon("icons/list-plus.svg")
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.enqueue_track(track.clone())
                })),
        );
    }
    if let Some(album) = track.album_id.clone() {
        items.push(
            MenuItem::new("queue-menu-album", t!("menu-go-to-album").to_string())
                .icon("icons/disc-3.svg")
                .on_press(act(move |ui: &mut Ui| {
                    ui.go(Destination::Album(album.as_str().into()))
                })),
        );
    }
    if let Some(artist) = track
        .artist_refs
        .iter()
        .find_map(|artist| artist.id.clone())
    {
        items.push(
            MenuItem::new("queue-menu-artist", t!("menu-go-to-artist").to_string())
                .icon("icons/user.svg")
                .on_press(act(move |ui: &mut Ui| {
                    ui.go(Destination::Artist(artist.as_str().into()))
                })),
        );
    }
    if menu.upcoming
        && let Some(uid) = menu.uid.clone()
    {
        items.push(MenuItem::separator("queue-menu-gap"));
        items.push(
            MenuItem::new(
                "queue-menu-remove",
                t!("menu-remove-from-queue").to_string(),
            )
            .icon("icons/x.svg")
            .on_press(act(move |ui: &mut Ui| ui.store.remove_queued(&uid))),
        );
    }
    Popup::new(
        "queue-menu",
        menu.at,
        Menu::<Ui>::new("queue-menu")
            .name(track.name.clone())
            .items(items),
    )
    .on_close(|ui: &mut Ui| ui.chrome.aside.queue.dismiss())
    .build(theme, layer);
}
