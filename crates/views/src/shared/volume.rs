use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    Context, Entity, MouseUpEvent, Pixels, ScrollWheelEvent, Window, deferred, div, px, relative,
};
use i18n::t;
use state::Playback;
use ui::{
    ActiveTheme as _, Button, Fading as _, GLASS_BLUR, GLASS_FILL, Rising as _, Scrubber,
    ScrubberState,
};

use crate::shared::transport::{NOTCH, percent, volume_icon};

/// How tall the slider stands once it has risen.
pub(crate) const VOLUME_RISE: f32 = 132.;
/// How far the level bubble floats from the slider.
pub(crate) const VOLUME_BUBBLE_LIFT: Pixels = px(18.);
/// How far around the risen panel the pointer may stray before it closes.
const ZONE: f32 = 14.;
/// The space between the button and the panel over it.
const GAP: f32 = 2.;
/// The level a muted volume comes back to when nothing remembers what it was.
const RESTORE: f32 = 0.7;

/// What a view keeps for a volume button whose slider rises over it while the pointer is on
/// the button, the slider or the space between them.
pub(crate) struct RisingVolume {
    over_button: bool,
    over_zone: bool,
    over_panel: bool,
    /// Whether the slider is being dragged, which keeps it up wherever the pointer goes.
    pub(crate) held: bool,
    /// The level the mute button silenced, which it restores.
    pub(crate) muted: Option<f32>,
    slider: ScrubberState,
}

impl RisingVolume {
    pub(crate) fn new(id: &'static str) -> Self {
        Self {
            over_button: false,
            over_zone: false,
            over_panel: false,
            held: false,
            muted: None,
            slider: ScrubberState::new(id),
        }
    }

    /// Whether the slider is up.
    pub(crate) fn open(&self) -> bool {
        self.parked() || self.held
    }

    /// Whether the pointer rests on any part of the control.
    pub(crate) fn parked(&self) -> bool {
        self.over_button || self.over_zone || self.over_panel
    }

    /// Forgets where the pointer was, for a view that is shown again and hears hover afresh.
    pub(crate) fn unpark(&mut self) {
        self.over_button = false;
        self.over_zone = false;
        self.over_panel = false;
    }
}

/// A view that draws a [`rising_volume`].
pub(crate) trait VolumeHost: Sized + 'static {
    fn volume(&mut self) -> &mut RisingVolume;
    fn playback(&self) -> &Entity<Playback>;
}

/// Turns the volume by one notch for each step of the wheel, and keeps the wheel from the page.
pub(crate) fn turn_volume<V: VolumeHost>(
    host: &mut V,
    event: &ScrollWheelEvent,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let delta = event.delta.pixel_delta(window.line_height()).y;
    if delta == Pixels::ZERO {
        return;
    }
    cx.stop_propagation();

    let notch = match delta > Pixels::ZERO {
        true => NOTCH,
        false => -NOTCH,
    };
    let level = (host.playback().read(cx).shown_volume(cx) + notch).clamp(0., 1.);
    host.volume().muted = None;
    host.playback()
        .update(cx, |playback, cx| playback.set_volume(level, cx));
}

/// The mute button with a vertical slider that rises over it on hover, for a place with no room
/// for a slider beside the button. `volume` and `playback` are the host's own. A `frosted` button and
/// panel float over artwork and blur what is under them.
pub(crate) fn rising_volume<V: VolumeHost>(
    volume: &RisingVolume,
    playback: &Entity<Playback>,
    frosted: bool,
    cx: &mut Context<V>,
) -> impl IntoElement + use<V> {
    let theme = *cx.theme();
    let hazy = frosted && ui::blurring(cx);
    let zone = px(ZONE);
    let level = playback.read(cx).shown_volume(cx);
    let empty = theme.muted_foreground.opacity(0.3);
    let restore = volume.muted.unwrap_or(RESTORE);
    let open = volume.open();
    let span = theme.metrics.control_small + zone * 2.;
    let reach = Scrubber::bubble_reach(VOLUME_BUBBLE_LIFT, cx);
    // The level shows for as long as the pointer is anywhere over the control, not only on
    // the track, and goes with it.
    let bubble = open.then(|| (level, percent(level)));

    div()
        .relative()
        .flex()
        .flex_none()
        .on_scroll_wheel(cx.listener(turn_volume))
        .child(
            div()
                .id("volume-hover")
                .on_hover(cx.listener(|this: &mut V, hovering: &bool, _, cx| {
                    this.volume().over_button = *hovering;
                    cx.notify();
                }))
                .child(
                    Button::new("volume")
                        .ghost()
                        .when(frosted, Button::frosted)
                        .small()
                        .icon(volume_icon(level))
                        // No tooltip: it would sit over the slider rising from the same spot,
                        // so the button carries its name for screen readers alone.
                        .name(match level <= 0.001 {
                            true => t!("player-unmute"),
                            false => t!("player-mute"),
                        })
                        .tint(match open {
                            true => theme.foreground,
                            false => theme.muted_foreground,
                        })
                        .on_click(cx.listener(move |this: &mut V, _, _, cx| {
                            let wanted = match level <= 0.001 {
                                true => restore,
                                false => 0.,
                            };
                            this.volume().muted = match wanted {
                                0. => Some(level),
                                _ => None,
                            };
                            this.playback()
                                .update(cx, |playback, cx| playback.set_volume(wanted, cx));
                        })),
                ),
        )
        .when(open, |this| {
            // The panel rises the way a menu does. A paint filter drops any backdrop
            // inside it, so the frost is a sibling underneath that fades in on the same
            // curve rather than arriving only once the rise is over.
            let panel = div()
                .id("volume-panel")
                .role(Role::Group)
                .aria_label(t!("equalizer-status-volume"))
                .occlude()
                .flex()
                .justify_center()
                .p_1()
                .py_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .map(|this| match hazy {
                    true => this.bg(theme.popover.opacity(GLASS_FILL)),
                    false => this.bg(theme.popover),
                })
                .on_scroll_wheel(cx.listener(turn_volume))
                .on_hover(cx.listener(|this: &mut V, hovering: &bool, _, cx| {
                    this.volume().over_panel = *hovering;
                    cx.notify();
                }))
                .child(
                    div().h(px(VOLUME_RISE)).flex().child(
                        Scrubber::new(&volume.slider, level)
                            .vertical()
                            .name(t!("equalizer-status-volume"))
                            .value(percent(level))
                            .colors(theme.progress_bar, empty, theme.foreground)
                            .lift(VOLUME_BUBBLE_LIFT)
                            .when_some(bubble, |this, (at, text)| this.bubble(at, text))
                            .on_move(cx.listener(|this: &mut V, fraction: &f32, _, cx| {
                                let level = *fraction;
                                this.volume().held = true;
                                this.volume().muted = None;
                                this.playback()
                                    .update(cx, |playback, cx| playback.slide_volume(level, cx));
                                cx.notify();
                            }))
                            .on_release(cx.listener(|this: &mut V, _: &MouseUpEvent, _, cx| {
                                this.volume().held = false;
                                this.playback()
                                    .update(cx, |playback, cx| playback.end_volume_slide(cx));
                                cx.notify();
                            })),
                    ),
                );

            this.child(deferred(
                div()
                    .id("volume-zone")
                    .absolute()
                    .bottom_0()
                    .left(relative(0.5))
                    .ml(Pixels::ZERO - span / 2.)
                    .w(span)
                    .pt(zone)
                    .pb(theme.metrics.control_small + px(GAP) * 2.)
                    .flex()
                    .justify_center()
                    .on_scroll_wheel(cx.listener(turn_volume))
                    .on_hover(cx.listener(|this: &mut V, hovering: &bool, _, cx| {
                        this.volume().over_zone = *hovering;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .relative()
                            .when(hazy, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .rounded(theme.radius)
                                        .backdrop_blur(GLASS_BLUR)
                                        .fading("volume-frost"),
                                )
                            })
                            // The rise composites what it carries as one layer the size of
                            // its bounds, so the wrapper reaches out as far as the level
                            // bubble can, and gives the room back with its margins.
                            .child(
                                div()
                                    .px(reach)
                                    .mx(Pixels::ZERO - reach)
                                    .child(panel)
                                    .rising("volume-rise"),
                            ),
                    ),
            ))
        })
}
