//! The player bar along the foot of the workspace: the playing cover, title and artists with the
//! heart, the transport over the seek bar, and the panel buttons, the volume and fullscreen on
//! the right. The layout follows the `player_bar_layout` setting and stacks into two rows in a
//! narrow window.
//!
//! Playback's position never runs this view: the seek bar draws from `ui.live.seek` and the
//! clocks from `ui.live.elapsed` and `ui.live.total`, which `Ui::poll` feeds.

use i18n::t;
use music::Track;
use router::Destination;
use rune::{Cx, El, SpringPair, Transition, Tx, WheelEvent, col, el, icon, row, text};
use state::{PlayerBarLayout, Repeat, SideTab, Store};
use ui::{
    Button, Cover, ExplicitBadge, InlineLink, InlineLinks, Room, Scrubber, ScrubberState, Springs,
    Text, Theme, Weight,
};

use crate::chrome::{sessions_button, sidebar_right};
use crate::shared::menus;
use crate::{Ui, act};

/// The widest the column of transport and seek bar grows.
const SEEK_MAX: f32 = 560.;
/// The volume slider's width, and its width in a window too narrow for the full one.
const VOLUME_WIDTH: f32 = 110.;
const VOLUME_TIGHT: f32 = 72.;
/// How wide a clock is, in multiples of its text size, for times under an hour and over.
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// How far one notch of the wheel over the volume moves it.
const NOTCH: f32 = 0.05;
/// How far the pointer has to move over a slider before its bubble follows.
const STEP: f32 = 0.004;
/// The level unmuting returns to when nothing was muted from.
const RESTORE: f32 = 0.7;
/// A button's icon side, which the cross-fading glyphs keep.
const GLYPH: f32 = 16.;
/// How small a glyph fading out shrinks to.
const GLYPH_SHRINK: f32 = 0.6;
/// How much a disabled button without a fill fades its ink, as `ui::Button` does.
const FADED: f32 = 0.55;
/// The bouncy spring a heart pops in on, and how small it starts.
const POP: rune::Spring = rune::Spring::new(0.42, 0.5);
const POP_FROM: f32 = 0.4;

/// The player bar's own view state: the sliders and what the pointer points at on them.
pub struct State {
    pub volume: ScrubberState,
    /// The fraction a drag on the seek bar holds, which the seek lands on once let go. While it
    /// is set `Ui::poll` leaves the bar where the drag put it.
    pub seeking: Option<f32>,
    over_seek: Option<f32>,
    over_volume: Option<f32>,
    volume_held: bool,
    /// The level before muting, which unmuting brings back.
    muted: Option<f32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            volume: ScrubberState::new(),
            seeking: None,
            over_seek: None,
            over_volume: None,
            volume_held: false,
            muted: None,
        }
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let layout = settings.player_bar_layout();
    let room = crate::window_room(cx);
    let stacked = stacked(room, layout);
    let translucent = settings.player_bar_translucent();
    let height = height(ui, cx);
    let show_track = room.fits(Room::Snug);
    let centered_wing = theme.metrics.control * 4. + VOLUME_WIDTH + theme.metrics.pad * 3.;

    let surface = |base: El| {
        let base = base
            .role("toolbar")
            .label(t!("settings-page-player-bar"))
            .w_full()
            .h(height)
            .px(rems(theme, 1.25))
            .border_sides([1., 0., 0., 0.], theme.border.color());
        match (translucent, theme.transparent) {
            (true, _) => ui::bar_surface(base, theme).color(theme.popover_foreground.color()),
            (false, false) => base.fill(theme.secondary.color()),
            (false, true) => base,
        }
    };

    let mut right = |volume: f32| {
        [
            buttons(ui, cx, layer),
            sound(ui, volume),
            fullscreen_button(theme),
        ]
    };
    let content = match (stacked, layout) {
        (true, _) => surface(col())
            .justify_center()
            .py(rems(theme, 0.5))
            .gap(rems(theme, 0.5))
            .child(
                row()
                    .center_y()
                    .gap(rems(theme, 0.5))
                    .w_full()
                    .child(now_playing(ui, show_track, layout, stacked))
                    .child(transport(ui, false)),
            )
            .child(
                row()
                    .center_y()
                    .gap(rems(theme, 0.75))
                    .w_full()
                    .child(col().grow(1.).min_w(0.).child(seek(ui)))
                    .children(right(volume_width(room))),
            ),
        (false, PlayerBarLayout::Centered) => surface(row())
            .center_y()
            .py(theme.metrics.pad)
            .gap(rems(theme, 0.75))
            .child(
                row()
                    .grow(1.)
                    .min_w(centered_wing)
                    .h_full()
                    .center_y()
                    .gap(rems(theme, 0.75))
                    .child(artwork(ui, layout, stacked))
                    .child(transport(ui, false)),
            )
            .child(
                col()
                    .grow(1.)
                    .min_w(0.)
                    .max_w(SEEK_MAX)
                    .h_full()
                    .justify_center()
                    .child(track_details(ui, true))
                    .child(seek(ui)),
            )
            .child(
                row()
                    .grow(1.)
                    .min_w(centered_wing)
                    .h_full()
                    .center_y()
                    .justify_end()
                    .gap(rems(theme, 0.5))
                    .children(right(VOLUME_WIDTH)),
            ),
        (false, PlayerBarLayout::Compact) => surface(row())
            .center_y()
            .gap(rems(theme, 0.75))
            .child(now_playing(ui, show_track, layout, stacked).max_w(theme.metrics.cover * 1.6))
            .child(transport(ui, false))
            .child(col().grow(1.).min_w(0.).child(seek(ui)))
            .children(right(VOLUME_WIDTH)),
        (false, PlayerBarLayout::Classic) => surface(row())
            .center_y()
            .gap(rems(theme, 1.))
            .child(now_playing(ui, show_track, layout, stacked))
            .child(
                col()
                    .items_center()
                    .gap(rems(theme, 0.25))
                    .grow(1.)
                    .min_w(0.)
                    .max_w(SEEK_MAX)
                    .child(transport(ui, false))
                    .child(seek(ui)),
            )
            .child(
                // never narrower than its buttons, or they spill over the seek bar's clock
                row()
                    .center_y()
                    .justify_end()
                    .gap(rems(theme, 0.5))
                    .grow(1.)
                    .children(right(VOLUME_WIDTH)),
            ),
    };

    content
}

/// The height the workspace gives the bar: two rows when stacked, a row's height in the compact
/// layout, the theme's player bar height otherwise.
pub(crate) fn height(ui: &Ui, cx: &Cx<Ui>) -> f32 {
    let theme = &ui.theme;
    let layout = ui.store.settings.player_bar_layout();
    match (stacked(crate::window_room(cx), layout), layout) {
        (true, _) => theme.metrics.player_bar + theme.metrics.pad * 3.,
        (false, PlayerBarLayout::Compact) => theme.metrics.row + theme.metrics.pad * 2.,
        _ => theme.metrics.player_bar,
    }
    .round()
}

/// How much of a page the bar covers: its height while it is translucent and floats over the
/// page, nothing while it stands under it. A page adds it to its bottom padding so its last row
/// scrolls clear of the bar.
pub(crate) fn bottom(ui: &Ui, cx: &Cx<Ui>) -> f32 {
    match ui.store.settings.player_bar_translucent() {
        true => height(ui, cx),
        false => 0.,
    }
}

/// `n` rems, which GPUI's spacing steps were and which follow the text size.
pub(crate) fn rems(theme: &Theme, n: f32) -> f32 {
    theme.font_size * n
}

/// Centered needs room for its three columns; smaller windows keep every control reachable in
/// the shared responsive layout. Only the whole window's width decides.
fn stacked(room: Room, layout: PlayerBarLayout) -> bool {
    let needs = match layout {
        PlayerBarLayout::Centered => Room::Wide,
        _ => Room::Roomy,
    };
    !room.fits(needs)
}

/// The volume slider keeps its full width until the window is as narrow as the one that stacks
/// the classic bar.
fn volume_width(room: Room) -> f32 {
    match room.fits(Room::Roomy) {
        true => VOLUME_WIDTH,
        false => VOLUME_TIGHT,
    }
}

/// The cover and, with room for it, the title and artists.
fn now_playing(ui: &Ui, room: bool, layout: PlayerBarLayout, stacked: bool) -> El {
    let theme = &ui.theme;
    let label = match ui.store.playback.shown_track(&ui.store.devices) {
        Some(track) => format!("{} — {}", track.name, track.artists),
        None => t!("player-nothing-playing").into_owned(),
    };
    let mut group = row()
        .key("now-playing")
        .role("group")
        .label(label)
        .center_y()
        .gap(rems(theme, 0.75))
        .grow(1.)
        .min_w(0.)
        .child(artwork(ui, layout, stacked));
    if room {
        group = group.child(track_details(ui, false));
    }
    group
}

/// The playing cover, which opens its album and the track's context menu in every layout.
fn artwork(ui: &Ui, layout: PlayerBarLayout, stacked: bool) -> El {
    let theme = &ui.theme;
    let size = match layout == PlayerBarLayout::Compact && !stacked {
        true => theme.metrics.thumb,
        false => theme.metrics.row,
    };
    let track = ui.store.playback.shown_track(&ui.store.devices);
    let cover = track.as_ref().and_then(|track| track.cover.as_deref());
    let art = Cover::new(ui.artwork(), cover)
        .size(size)
        .fallback("icons/disc-3.svg")
        .build(theme);
    // the fullscreen cover carries the same match, so the cover flies between the two
    let mut face = el()
        .key("now-playing-artwork")
        .matched(crate::shared::transport::COVER_FLIGHT)
        .child(art);
    let Some(track) = track else {
        return face;
    };
    let album = match track.album.is_empty() {
        true => t!("song-view-album").into_owned(),
        false => track.album.clone(),
    };
    face = face
        .label(album)
        .on_secondary_press(open_menu)
        .press_scale(ui::PRESS_SCALE_CARD);
    match track.album_id.clone() {
        Some(id) => face
            .role("link")
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(act(move |ui: &mut Ui| {
                ui.go(Destination::Album(id.clone().into()))
            })),
        None => face.role("img"),
    }
}

/// The title and artist links, the explicit badge and the heart, optionally centred without
/// moving the cover or the transport with them.
fn track_details(ui: &Ui, centered: bool) -> El {
    let theme = &ui.theme;
    let muted = theme.muted_foreground;
    let track = ui.store.playback.shown_track(&ui.store.devices);
    let explicit = track.as_ref().is_some_and(|track| track.explicit);
    let title = match &track {
        Some(track) => {
            let mut title = InlineLinks::<Ui>::new(
                "now-playing-track",
                [InlineLink::new(track.name.clone(), track.album_id.clone())],
                track.name.clone(),
                theme.foreground,
            )
            .truncate();
            if track.album_id.is_some() {
                title = title.on_link(|ui: &mut Ui, id: &str, _: &mut Tx| {
                    ui.go(Destination::Album(id.to_owned().into()));
                    ui.settle();
                });
            }
            title.build(theme).on_secondary_press(open_menu)
        }
        None => text(t!("player-nothing-playing"))
            .key("now-playing-album")
            .role("status")
            .label(t!("player-nothing-playing"))
            .font(ui::font(theme, Text::Body, Weight::NORMAL))
            .color(muted.color())
            .truncate()
            .min_w(0.),
    };
    let mut actions = row().center_y().gap(rems(theme, 0.25));
    if explicit {
        actions = actions.child(ExplicitBadge::new().build(theme));
    }
    actions = actions.child(like(ui, track.as_ref()));
    let headline = match centered {
        // equal clearance on both sides keeps the title itself centred; the actions sit
        // outside its measured width and stay reachable even for a truncated title
        true => {
            let clearance = theme.metrics.control_small
                * match explicit {
                    true => 2.,
                    false => 1.,
                }
                + theme.metrics.pad;
            row().w_full().justify_center().px(clearance).child(
                row()
                    .center_y()
                    .min_w(0.)
                    .min_h(theme.text(Text::Body) * ui::LEADING)
                    .child(title)
                    // a box of no width after the title, which the actions hang from
                    .child(el().w(0.).h_full().child(actions.pin(
                        Some(theme.metrics.pad * 0.5),
                        Some(0.),
                        None,
                        Some(0.),
                    ))),
            )
        }
        false => row()
            .w_full()
            .min_w(0.)
            .center_y()
            .gap(rems(theme, 0.25))
            .child(title)
            .child(actions),
    };
    let mut details = col().justify_center().grow(1.).min_w(0.).child(headline);
    if centered {
        details = details.items_center().w_full();
    }
    if let Some(track) = &track {
        let artists = InlineLinks::<Ui>::new(
            "now-playing-artist",
            track
                .artist_refs
                .iter()
                .map(|artist| InlineLink::new(artist.name.clone(), artist.id.clone())),
            track.artists.clone(),
            muted,
        )
        .text_size(theme.text(Text::Small))
        .truncate()
        .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| {
            ui.go(Destination::Artist(id.to_owned().into()));
            ui.settle();
        })
        .build(theme);
        details = details.child(artists);
    }
    details
}

/// The heart that saves or drops the shown track. Liking pops the filled heart in on a bouncy
/// spring; unliking fades it back to the outline.
pub(crate) fn like(ui: &Ui, track: Option<&Track>) -> El {
    let theme = &ui.theme;
    let id = track.and_then(|track| track.id.as_deref());
    let saved = id.is_some_and(|id| ui.store.library.saved(id));
    let ink = match (id.is_some(), saved) {
        (false, _) => theme.muted_foreground.opacity(FADED),
        (true, true) => theme.primary,
        (true, false) => theme.muted_foreground,
    };
    let mut glyph = el().key("heart").size(GLYPH, GLYPH).a11y_hidden().child(
        icon(icons::get("icons/heart.svg"), GLYPH)
            .key("heart-outline")
            .absolute(0., 0.)
            .color(ink.color())
            .opacity(match saved {
                true => 0.,
                false => 1.,
            }),
    );
    if saved {
        glyph = glyph.child(
            icon(icons::get("icons/heart-filled.svg"), GLYPH)
                .key("heart-filled")
                .absolute(0., 0.)
                .color(ink.color())
                .transition(
                    Transition::asymmetric(
                        Transition::scale(POP_FROM).allow_small_scale(),
                        Transition::opacity(),
                    )
                    .springs(SpringPair::new(POP, Springs::QUICK)),
                ),
        );
    }
    Button::new("toggle-liked-track")
        .ghost()
        .backgroundless()
        .small()
        .leading(glyph)
        .toggled(saved)
        .tooltip_above(match saved {
            true => t!("menu-remove-from-library"),
            false => t!("menu-add-to-library"),
        })
        .disabled(id.is_none())
        .on_press(act(|ui: &mut Ui| toggle_liked(&mut ui.store)))
        .build(theme)
}

/// Flips the heart on the track playback holds, which is the one the bar shows.
fn toggle_liked(store: &mut Store) {
    let Some(track) = store.playback.track().cloned() else {
        return;
    };
    let Store {
        library,
        session,
        cx,
        ..
    } = store;
    library.toggle(track, session, cx);
}

/// Shuffle, previous, play or pause, next and repeat. `big` is only true in fullscreen, where
/// the buttons float over the ambient background and frost their hover fills.
pub(crate) fn transport(ui: &Ui, big: bool) -> El {
    let theme = &ui.theme;
    row()
        .key("transport")
        .center_y()
        .gap(rems(theme, 0.5))
        .child(shuffle(ui, big))
        .child(previous(ui, big))
        .child(toggle(ui, big))
        .child(next(ui, big))
        .child(repeat(ui, big))
}

/// The ghost button every transport button starts from.
fn transport_button(key: &str, big: bool) -> Button {
    let button = Button::new(key).ghost();
    match big {
        true => button.frosted(),
        false => button.small(),
    }
}

/// Play or pause. The two glyphs cross-fade and swap sizes on a soft spring as playback starts
/// and stops, rather than the icon being swapped in place.
fn toggle(ui: &Ui, big: bool) -> El {
    let theme = &ui.theme;
    let store = &ui.store;
    let steered = store.playback.steered(&store.devices);
    let playing = steered.as_ref().map_or_else(
        || store.playback.wants_playing(),
        |remote| remote.state.playing,
    );
    let idle = store.playback.track().is_none() && steered.is_none();
    let ink = match idle {
        true => theme.muted_foreground.opacity(FADED),
        false => theme.foreground,
    };
    let glyph = |key: &str, path: &str, shown: bool| {
        icon(icons::get(path), GLYPH)
            .key(key)
            .absolute(0., 0.)
            .color(ink.color())
            .opacity(match shown {
                true => 1.,
                false => 0.,
            })
            .scale(match shown {
                true => 1.,
                false => GLYPH_SHRINK,
            })
            .motion(Springs::SETTLE)
    };
    let face = el()
        .key("play-glyph")
        .size(GLYPH, GLYPH)
        .a11y_hidden()
        .child(glyph("play", "icons/play.svg", !playing))
        .child(glyph("pause", "icons/pause.svg", playing));
    transport_button("play-pause", big)
        .leading(face)
        .tooltip_above(match playing {
            true => t!("play-pause"),
            false => t!("play-resume"),
        })
        .disabled(idle)
        .on_press(act(|ui: &mut Ui| ui.store.toggle_play()))
        .build(theme)
}

fn shuffle(ui: &Ui, big: bool) -> El {
    let theme = &ui.theme;
    let on = ui
        .store
        .playback
        .shown_shuffle(&ui.store.queue, &ui.store.devices);
    transport_button("shuffle", big)
        .icon("icons/shuffle.svg")
        .indicator(on)
        .toggled(on)
        .tooltip_above(t!("player-shuffle"))
        .tint(match on {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .on_press(act(|ui: &mut Ui| ui.store.toggle_shuffle()))
        .build(theme)
}

fn repeat(ui: &Ui, big: bool) -> El {
    let theme = &ui.theme;
    let repeat = ui.store.playback.shown_repeat(&ui.store.devices);
    transport_button("repeat", big)
        .icon(match repeat {
            Repeat::One => "icons/repeat-one.svg",
            _ => "icons/repeat.svg",
        })
        .indicator(repeat != Repeat::Off)
        .tooltip_above(match repeat {
            Repeat::Off => t!("player-repeat"),
            Repeat::All => t!("player-repeat-all"),
            Repeat::One => t!("player-repeat-one"),
        })
        .tint(match repeat {
            Repeat::Off => theme.muted_foreground,
            _ => theme.primary,
        })
        .on_press(act(|ui: &mut Ui| ui.store.cycle_repeat()))
        .build(theme)
}

fn previous(ui: &Ui, big: bool) -> El {
    let queue = ui.store.shown_queue();
    let enabled = !queue.editable() || queue.sections.current || queue.sections.past > 0;
    transport_button("previous", big)
        .icon("icons/skip-back.svg")
        .tooltip_above(t!("player-previous"))
        .disabled(!enabled)
        .on_press(act(|ui: &mut Ui| ui.store.previous_track()))
        .build(&ui.theme)
}

fn next(ui: &Ui, big: bool) -> El {
    let queue = ui.store.shown_queue();
    let enabled = !queue.editable() || queue.sections.upcoming > 0;
    transport_button("next", big)
        .icon("icons/skip-forward.svg")
        .tooltip_above(t!("player-next"))
        .disabled(!enabled)
        .on_press(act(|ui: &mut Ui| ui.store.next_track()))
        .build(&ui.theme)
}

/// The elapsed time, the seek bar and the length. The bar and both clocks follow playback
/// through live values, so a playing track never runs the view.
fn seek(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = &ui.chrome.player_bar;
    let (_, total) = ui.store.playback.shown_time(&ui.store.devices);
    let seekable = ui.store.playback.shown_track(&ui.store.devices).is_some();
    let size = theme.text(Text::Tiny);
    let width = size
        * match total.as_secs() >= 3600 {
            true => CLOCK_LONG,
            false => CLOCK_SHORT,
        };
    let clock = |key: &str, live: &rune::LiveText, end: bool| {
        text("0:00")
            .key(key)
            .role("text")
            .live_text(live)
            .font(ui::font(theme, Text::Tiny, Weight::NORMAL).features(ui::TABULAR))
            .color(theme.muted_foreground.color())
            .align(match end {
                true => rune::Align::End,
                false => rune::Align::Start,
            })
            .w(width)
    };
    let mut bar = Scrubber::<Ui>::live("player-seek-bar", &ui.live.seek)
        .name(t!("song-position"))
        .colors(
            theme.progress_bar,
            theme.muted_foreground.opacity(0.3),
            theme.foreground,
        )
        .enabled(seekable)
        .on_move(|ui: &mut Ui, fraction: f32, _: &mut Tx| {
            ui.chrome.player_bar.seeking = Some(fraction);
        })
        .on_release(|ui: &mut Ui, _: &mut Tx| {
            if let Some(fraction) = ui.chrome.player_bar.seeking.take() {
                ui.store.seek_fraction(fraction);
                ui.settle();
            }
        })
        .on_hover(|ui: &mut Ui, at: Option<f32>, tx: &mut Tx| {
            let state = &mut ui.chrome.player_bar;
            match moved(state.over_seek, at) {
                true => state.over_seek = at,
                false => tx.live_only(),
            }
        });
    if let Some(at) = state.over_seek.or(state.seeking) {
        bar = bar.bubble(at, ui::clock(total.mul_f32(at)));
    }
    row()
        .key("player-seek")
        .role("group")
        .label(t!("song-position"))
        .center_y()
        .gap(rems(theme, 0.5))
        .w_full()
        .child(clock("player-elapsed", &ui.live.elapsed, true))
        .child(col().grow(1.).min_w(0.).child(bar.build(theme)))
        .child(clock("player-duration", &ui.live.total, false))
}

/// The mute button and the volume slider, which the wheel turns too.
fn sound(ui: &Ui, width: f32) -> El {
    let theme = &ui.theme;
    let state = &ui.chrome.player_bar;
    let level = ui.store.playback.shown_volume(&ui.store.devices);
    let muted = level <= 0.001;
    let mut slider = Scrubber::<Ui>::new("player-volume-bar", &state.volume, level)
        .name(t!("equalizer-status-volume"))
        .value(percent(level))
        .colors(
            theme.progress_bar,
            theme.muted_foreground.opacity(0.3),
            theme.foreground,
        )
        .on_move(|ui: &mut Ui, level: f32, _: &mut Tx| {
            let state = &mut ui.chrome.player_bar;
            state.volume_held = true;
            state.muted = None;
            ui.store.slide_volume(level);
            ui.settle();
        })
        .on_release(|ui: &mut Ui, _: &mut Tx| {
            ui.chrome.player_bar.volume_held = false;
            ui.store.end_volume_slide();
            ui.settle();
        })
        .on_hover(|ui: &mut Ui, at: Option<f32>, tx: &mut Tx| {
            let state = &mut ui.chrome.player_bar;
            match moved(state.over_volume, at) {
                true => state.over_volume = at,
                false => tx.live_only(),
            }
        });
    if state.over_volume.is_some() || state.volume_held {
        slider = slider.bubble(level, percent(level));
    }
    let mute = Button::new("volume")
        .ghost()
        .small()
        .icon(volume_icon(level))
        .tooltip_above(match muted {
            true => t!("player-unmute"),
            false => t!("player-mute"),
        })
        .tint(theme.muted_foreground)
        .on_press(act(move |ui: &mut Ui| {
            let state = &mut ui.chrome.player_bar;
            let wanted = match muted {
                true => state.muted.unwrap_or(RESTORE),
                false => 0.,
            };
            state.muted = (!muted).then_some(level);
            ui.store.set_volume(wanted);
        }))
        .build(theme);
    row()
        .key("player-volume")
        .role("group")
        .label(t!("equalizer-status-volume"))
        .center_y()
        .gap(rems(theme, 0.25))
        .on_wheel(turn_volume)
        .child(mute)
        .child(el().w(width).child(slider.build(theme)))
}

/// Turns the volume a notch for each step of the wheel over the slider.
fn turn_volume(ui: &mut Ui, wheel: WheelEvent, _: &mut Tx) -> bool {
    if wheel.dy == 0. {
        return false;
    }
    let notch = match wheel.dy > 0. {
        true => NOTCH,
        false => -NOTCH,
    };
    let level = (ui.store.playback.shown_volume(&ui.store.devices) + notch).clamp(0., 1.);
    ui.chrome.player_bar.muted = None;
    ui.store.set_volume(level);
    ui.settle();
    true
}

/// The Sessions button and the buttons that open the lyrics and the queue beside the page, one
/// gap apart. The Sessions button takes no gap while it draws nothing.
fn buttons(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut group = row().key("player-panels").center_y().gap(rems(theme, 0.25));
    let picker = settings.sessions_button()
        && (ui.store.devices.available() || ui.store.listening.available(&ui.store.session));
    if picker {
        group = group.child(sessions_button::view(ui, cx, layer));
    }
    if !crate::window_room(cx).fits(Room::Wide) {
        return group;
    }
    let open = ui.chrome.right_open;
    let tab = ui.chrome.right_tab;
    let side = |key: &str, path: &str, hint: &str, side: SideTab| {
        let showing = open && tab == side;
        Button::new(key)
            .ghost()
            .small()
            .icon(path)
            .tooltip_above(i18n::lookup(hint, None))
            .selected(showing)
            .expanded(showing)
            .tint(match showing {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .on_press(act(move |ui: &mut Ui| sidebar_right::show(ui, side)))
            .build(theme)
    };
    if settings.lyrics_enabled() {
        group = group.child(side(
            "player-lyrics",
            "icons/mic-vocal.svg",
            "lyrics-title",
            SideTab::Lyrics,
        ));
    }
    group.child(side(
        "player-queue",
        "icons/list-music.svg",
        "queue-title",
        SideTab::Queue,
    ))
}

fn fullscreen_button(theme: &Theme) -> El {
    Button::new("toggle-fullscreen")
        .ghost()
        .small()
        .icon("icons/maximize.svg")
        .tooltip_above(t!("player-fullscreen"))
        .on_press(act(|ui: &mut Ui| ui.go(Destination::Fullscreen)))
        .build(theme)
}

/// Opens the shared track menu on whatever is playing, where the secondary press landed.
pub(crate) fn open_menu(ui: &mut Ui, press: rune::PointerPress, _: &mut Tx) {
    let Some(track) = ui.store.playback.shown_track(&ui.store.devices) else {
        return;
    };
    menus::open(ui, menus::Target::track(track), press);
}

pub(crate) fn volume_icon(level: f32) -> &'static str {
    match level {
        level if level <= 0.0001 => "icons/volume-x.svg",
        level if level < 0.25 => "icons/volume.svg",
        level if level < 0.5 => "icons/volume-1.svg",
        _ => "icons/volume-2.svg",
    }
}

pub(crate) fn percent(fraction: f32) -> String {
    t!("player-percent", value = (fraction * 100.).round()).into_owned()
}

/// Whether the pointer moved far enough over a slider for its bubble to follow, or came or went.
pub(crate) fn moved(before: Option<f32>, after: Option<f32>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => (before - after).abs() > STEP,
        (before, after) => before.is_some() != after.is_some(),
    }
}
