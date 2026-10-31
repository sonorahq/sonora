use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Div, Hsla, Pixels, Window, div, relative};
use i18n::t;
use state::PlayerBarLayout;
use ui::{ActiveTheme as _, OptionCard, Switch};

use super::rows::{Announced as _, grid};
use super::{Group, Row, SettingsView, Slot};

impl SettingsView {
    /// The player bar's own section in Appearance: layout cards followed by its material.
    pub(super) fn player_bar_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-player-bar-layout", "icons/layout-grid.svg")
                .cards(Slot::PlayerBarLayout),
            Group::new("settings-player-bar-surface", "icons/blend.svg")
                .rows([Slot::PlayerBarTransparent]),
        ]
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
/// pieces stay centred on one line whatever width the settings column gives the card.
fn sketch(layout: PlayerBarLayout, transparent: bool, height: Pixels, cx: &App) -> Div {
    let theme = *cx.theme();
    let radius = theme.radius * 0.25;
    let ink = theme.foreground.opacity(0.6);
    let muted = theme.muted_foreground.opacity(0.4);
    let bar = height
        * match layout {
            PlayerBarLayout::Compact => 0.26,
            PlayerBarLayout::Classic | PlayerBarLayout::Centered => 0.44,
        };
    let gap = height * 0.06;
    let title = height * 0.035;
    let line = height * 0.025;

    let cover = div()
        .flex_none()
        .size(bar * 0.7)
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

    let panel = div()
        .absolute()
        .bottom_0()
        .w_full()
        .h(bar)
        .flex()
        .items_center()
        .gap(gap)
        .px(gap)
        .border_t_1()
        .border_color(theme.border)
        .map(|panel| match transparent {
            true => ui::popover_surface(panel, cx),
            false => panel.bg(theme.secondary),
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
    };

    div()
        .relative()
        .size_full()
        .bg(theme.background)
        .child(tile([0.05, 0.10, 0.35, 0.05], muted, radius))
        .children((0..4).map(|index| {
            tile(
                [0.05 + index as f32 * 0.24, 0.28, 0.18, 0.46],
                theme.primary.opacity(0.15),
                radius,
            )
        }))
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
