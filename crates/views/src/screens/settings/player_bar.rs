use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Hsla, Pixels, Window, div, linear_color_stop, linear_gradient,
    relative,
};
use i18n::t;
use state::PlayerBarLayout;
use ui::{ActiveTheme as _, OptionCard, Switch};

use super::rows::{Announced as _, grid};
use super::{Group, Row, SettingsView, Slot};

/// How much of the page a transparent bar's miniature lets through. The real bar is far more
/// solid, but at a thumbnail's size only a thin fill shows that the page runs on under it.
const SEE_THROUGH: f32 = 0.55;

impl SettingsView {
    /// The Player bar page: the layout cards under the page's own title, then the bar's
    /// material and its clock, all in one group.
    pub(super) fn player_bar_groups(&self) -> Vec<Group> {
        vec![
            Group::untitled()
                .cards(Slot::PlayerBarLayout)
                .rows([Slot::PlayerBarTransparent, Slot::PlayerBarTimeLeft]),
        ]
    }

    pub(super) fn player_bar_time_left_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).player_bar_time_left();
        Row::new(
            t!("settings-player-bar-time-left"),
            Switch::new("player-bar-time-left", on)
                .name(t!("settings-player-bar-time-left"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_player_bar_time_left(!on, cx);
                    });
                }))
                .into_any_element(),
        )
        .detail(t!("settings-player-bar-time-left-detail"))
        .into_any_element()
    }

    /// Miniature layouts share the saved choice with the actual workspace player bar.
    pub(super) fn player_bar_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).player_bar_layout();
        let transparent = self.settings.read(cx).player_bar_translucent();
        let height = cx.theme().metrics.list_row * 1.6;
        grid(PlayerBarLayout::ALL.len() as u16, window, cx)
            .children(
                PlayerBarLayout::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(place, layout)| {
                        OptionCard::new(layout.id())
                            .choice(
                                i18n::lookup(layout.key(), None),
                                layout == chosen,
                                place,
                                PlayerBarLayout::ALL.len(),
                            )
                            .thumb_height(height)
                            .thumb(sketch(layout, transparent, height, cx))
                            .label(i18n::lookup(layout.key(), None))
                            .selected(layout == chosen)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.update(cx, |settings, cx| {
                                    settings.set_player_bar_layout(layout, cx);
                                });
                            }))
                    }),
            )
            .into_any_element()
    }

    /// Explains that transparency also moves the page behind the player bar. The switch stands
    /// off and held while Blur UI is off, since the bar cannot be see-through without it.
    pub(super) fn player_bar_transparent_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let on = settings.player_bar_transparent();
        let blur = settings.blur();
        Row::new(
            t!("settings-player-bar-transparent"),
            Switch::new("player-bar-transparent", on && blur)
                .name(t!("settings-player-bar-transparent"))
                .disabled(!blur)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_player_bar_transparent(!on, cx);
                    });
                }))
                .into_any_element(),
        )
        .detail(match blur {
            true => t!("settings-player-bar-transparent-detail"),
            false => t!("settings-player-bar-transparent-needs-blur"),
        })
        .into_any_element()
    }
}

/// A still miniature of a page and its player bar, `height` tall. The page is placed in
/// proportions of the thumbnail and the bar in flex rows sized from `height`, so the bar's
/// pieces stay centred on one line whatever width the settings column gives the card. The
/// page runs on under the bar, which shows it through while `transparent` and stands on the
/// chrome's colour, as the real bar does, while not.
fn sketch(layout: PlayerBarLayout, transparent: bool, height: Pixels, cx: &App) -> Div {
    let theme = *cx.theme();
    let radius = theme.radius * 0.25;
    let ink = theme.foreground.opacity(0.6);
    let muted = theme.muted_foreground.opacity(0.4);
    let bar = height
        * match layout {
            PlayerBarLayout::Compact => 0.26,
            PlayerBarLayout::Island => 0.3,
            PlayerBarLayout::Classic | PlayerBarLayout::Centered => 0.34,
        };
    let gap = height * 0.06;
    let title = height * 0.035;
    let line = height * 0.025;

    // A full-height bar keeps its cover well inside its height, as the real bar does around a
    // row-sized cover. The compact bar is about as tall as its cover.
    let cover_side = bar
        * match layout {
            PlayerBarLayout::Classic | PlayerBarLayout::Centered => 0.6,
            PlayerBarLayout::Compact | PlayerBarLayout::Island => 0.7,
        };
    let cover = div()
        .flex_none()
        .size(cover_side)
        .rounded(radius)
        .bg(theme.primary.opacity(0.6));
    let details = |centered: bool| {
        div()
            .flex()
            .flex_col()
            .when(centered, |this| this.items_center())
            .gap(title)
            .child(stroke(0.8, title, ink, radius))
            .child(stroke(0.55, line, muted, radius))
    };
    let seek = div()
        .relative()
        .w_full()
        .h(line)
        .rounded(radius)
        .bg(muted)
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(0.4))
                .rounded(radius)
                .bg(ink),
        );
    let sound = |width: f32| {
        div()
            .flex()
            .justify_end()
            .child(stroke(width, line, muted, radius))
    };

    // The island floats clear of the thumbnail's edges as a pill, its end marks set in from its
    // round ends. Every other bar spans the thumbnail.
    let panel = div()
        .absolute()
        .h(bar)
        .flex()
        .items_center()
        .gap(gap)
        .border_color(theme.border)
        .map(|panel| match layout {
            PlayerBarLayout::Island => panel
                .bottom(gap)
                .left(gap * 2.)
                .right(gap * 2.)
                .px(bar * 0.3)
                .rounded(bar / 2.)
                .border_1(),
            _ => panel.bottom_0().w_full().px(gap).border_t_1(),
        })
        .map(|panel| match (transparent, layout) {
            (true, _) => panel.bg(theme.popover.opacity(SEE_THROUGH)),
            (false, PlayerBarLayout::Island) => panel.bg(theme.popover),
            (false, _) => panel.bg(theme.sidebar),
        });
    let panel = match layout {
        PlayerBarLayout::Classic => panel
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(gap)
                    .child(cover)
                    .child(details(false).flex_1().min_w_0()),
            )
            .child(
                div()
                    .w(relative(0.3))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(gap * 0.6)
                    .child(transport(height, cx))
                    .child(seek),
            )
            .child(sound(0.5).flex_1()),
        PlayerBarLayout::Centered => panel
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(gap)
                    .child(cover)
                    .child(transport(height, cx)),
            )
            .child(
                div()
                    .w(relative(0.3))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(gap * 0.6)
                    .child(details(true).w_full())
                    .child(seek),
            )
            .child(sound(0.5).flex_1()),
        PlayerBarLayout::Compact => panel
            .child(cover)
            .child(details(false).w(relative(0.14)))
            .child(transport(height, cx))
            .child(div().flex_1().child(seek))
            .child(sound(1.).w(relative(0.12))),
        // The cover and title sit over the seek line, all three small enough to share the pill's
        // height, with the tools after them as marks like the transport's.
        PlayerBarLayout::Island => panel
            .child(transport(height, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(title)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(gap * 0.6)
                            .child(
                                div()
                                    .flex_none()
                                    .size(bar * 0.4)
                                    .rounded(radius)
                                    .bg(theme.primary.opacity(0.6)),
                            )
                            .child(details(false).flex_1().min_w_0()),
                    )
                    .child(seek),
            )
            .child(transport(height, cx)),
    };
    // Under the island the page fades into its own colour, the way it does in the window.
    let fade = (layout == PlayerBarLayout::Island).then(|| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(bar + gap * 2.)
            .bg(linear_gradient(
                180.,
                linear_color_stop(theme.background.opacity(0.), 0.),
                linear_color_stop(theme.background, 0.7),
            ))
    });

    div()
        .relative()
        .size_full()
        .bg(theme.background)
        .child(tile([0.05, 0.10, 0.35, 0.05], muted, radius))
        .children((0..4).map(|index| {
            tile(
                [0.05 + index as f32 * 0.24, 0.28, 0.18, 0.72],
                theme.primary.opacity(0.15),
                radius,
            )
        }))
        .children(fade)
        .child(panel)
}

/// One block of the page behind the bar, placed in proportions of the thumbnail.
fn tile([left, top, width, height]: [f32; 4], fill: Hsla, radius: Pixels) -> Div {
    div()
        .absolute()
        .left(relative(left))
        .top(relative(top))
        .w(relative(width))
        .h(relative(height))
        .rounded(radius)
        .bg(fill)
}

/// A line of text or a level in the bar, `width` of its slot across and `thickness` tall.
fn stroke(width: f32, thickness: Pixels, fill: Hsla, radius: Pixels) -> Div {
    div()
        .w(relative(width))
        .h(thickness)
        .rounded(radius)
        .bg(fill)
}

/// Three muted marks that suggest the playback buttons without drawing the live icons.
fn transport(height: Pixels, cx: &App) -> Div {
    let theme = cx.theme();
    let mark = height * 0.07;
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(mark * 0.8)
        .children((0..3).map(|_| {
            div()
                .size(mark)
                .rounded(theme.radius * 0.15)
                .bg(theme.muted_foreground.opacity(0.45))
        }))
}
