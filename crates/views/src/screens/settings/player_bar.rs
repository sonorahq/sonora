//! The Player Bar section of Appearance: the bar's layout as miniature cards, then its
//! material.

use i18n::t;
use rune::{El, col, el, row};
use state::PlayerBarLayout;
use ui::{Hsla, OptionCard, Theme};

use super::appearance::card_width;
use super::{Layout, Row, columns, grid, group, toggle};
use crate::{Ui, act};

/// How tall a layout card's picture stands, in list rows.
const THUMB: f32 = 1.6;
/// How strongly the strokes of the sketch show.
const INK: f32 = 0.6;
const MUTED: f32 = 0.4;
const TILES: f32 = 0.15;
const MARKS: f32 = 0.45;

pub(super) fn view(
    ui: &Ui,
    _cx: &mut rune::Cx<Ui>,
    layer: &mut Vec<El>,
    layout: &Layout,
) -> Vec<El> {
    let theme = &ui.theme;
    vec![
        group(
            theme,
            Some(("settings-player-bar-layout", "icons/layout-grid.svg")),
            Some(cards(ui, layer, layout)),
            [],
        ),
        group(
            theme,
            Some(("settings-player-bar-surface", "icons/blend.svg")),
            None,
            [transparent(ui)],
        ),
    ]
}

/// Miniature layouts share the saved choice with the workspace's player bar.
fn cards(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let chosen = settings.player_bar_layout();
    let translucent = settings.player_bar_translucent();
    let height = theme.metrics.list_row * THUMB;
    let count = PlayerBarLayout::ALL.len();
    let width = card_width(layout, count);
    let cards = PlayerBarLayout::ALL.into_iter().map(|shape| {
        OptionCard::new(format!("player-bar-{}", shape.id()))
            .thumb_height(height)
            .thumb(sketch(theme, shape, translucent, width, height))
            .label(i18n::lookup(shape.key(), None))
            .selected(shape == chosen)
            .on_press(act(move |ui: &mut Ui| {
                ui.store
                    .settings
                    .set_player_bar_layout(shape, &mut ui.store.cx)
            }))
            .build(theme, layer)
    });
    grid(
        "player-bar",
        columns(count, layout.room()),
        cards.collect::<Vec<_>>(),
    )
}

/// Transparency also moves the page behind the player bar. The switch stands off and held while
/// Blur UI is off, since the bar cannot be see-through without it.
fn transparent(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let on = settings.player_bar_transparent();
    let blur = settings.blur();
    Row::new(
        "player-bar-transparent",
        t!("settings-player-bar-transparent"),
        toggle(
            theme,
            "player-bar-transparent",
            t!("settings-player-bar-transparent"),
            on && blur,
            !blur,
            |ui, on| {
                ui.store
                    .settings
                    .set_player_bar_transparent(on, &mut ui.store.cx)
            },
        ),
    )
    .detail(match blur {
        true => t!("settings-player-bar-transparent-detail"),
        false => t!("settings-player-bar-transparent-needs-blur"),
    })
    .build(theme)
}

/// A still miniature of a page and its player bar, `width` by `height`. The page is placed in
/// proportions of the thumbnail and the bar in rows sized from `height`, so the bar's pieces
/// stay centred on one line whatever width the column gives the card.
fn sketch(theme: &Theme, shape: PlayerBarLayout, translucent: bool, width: f32, height: f32) -> El {
    let radius = theme.radius * 0.25;
    let ink = theme.foreground.opacity(INK);
    let muted = theme.muted_foreground.opacity(MUTED);
    let bar = height
        * match shape {
            PlayerBarLayout::Compact => 0.26,
            PlayerBarLayout::Classic | PlayerBarLayout::Centered => 0.44,
        };
    let gap = height * 0.06;
    let title = height * 0.035;
    let line = height * 0.025;
    let inner = width - gap * 2.;

    let cover = || {
        el().size(bar * 0.7, bar * 0.7)
            .radius(radius)
            .fill(theme.primary.opacity(0.6).color())
    };
    let details = |centered: bool, across: f32| {
        let mut words = col()
            .w(across)
            .gap(title)
            .child(stroke(across * 0.8, title, ink, radius))
            .child(stroke(across * 0.55, line, muted, radius));
        if centered {
            words = words.items_center();
        }
        words
    };
    let seek = |across: f32| {
        el().w(across)
            .h(line)
            .radius(radius)
            .fill(muted.color())
            .child(stroke(across * 0.4, line, ink, radius).absolute(0., 0.))
    };
    let sound = |across: f32, share: f32| {
        row()
            .w(across)
            .justify_end()
            .child(stroke(across * share, line, muted, radius))
    };
    let middle = inner * 0.3;
    let side = (inner - middle - gap * 2.) / 2.;
    let lead = (side - bar * 0.7 - gap).max(0.);

    let mut panel = row()
        .pin(Some(0.), None, Some(0.), Some(0.))
        .h(bar)
        .center_y()
        .gap(gap)
        .px(gap)
        .border_sides([1., 0., 0., 0.], theme.border.color());
    panel = match translucent {
        true => panel.fill(ui::popover_fill(theme).color()),
        false => panel.fill(theme.secondary.color()),
    };
    let panel = match shape {
        PlayerBarLayout::Classic => panel
            .child(
                row()
                    .w(side)
                    .center_y()
                    .gap(gap)
                    .child(cover())
                    .child(details(false, lead)),
            )
            .child(
                col()
                    .w(middle)
                    .items_center()
                    .gap(gap * 0.6)
                    .child(transport(theme, height))
                    .child(seek(middle)),
            )
            .child(sound(side, 0.5)),
        PlayerBarLayout::Centered => panel
            .child(
                row()
                    .w(side)
                    .center_y()
                    .gap(gap)
                    .child(cover())
                    .child(transport(theme, height)),
            )
            .child(
                col()
                    .w(middle)
                    .items_center()
                    .gap(gap * 0.6)
                    .child(details(true, middle))
                    .child(seek(middle)),
            )
            .child(sound(side, 0.5)),
        PlayerBarLayout::Compact => {
            let words = inner * 0.14;
            let level = inner * 0.12;
            let marks = height * 0.07 * 3. + height * 0.07 * 0.8 * 2.;
            let rest = (inner - bar * 0.7 - words - level - marks - gap * 4.).max(0.);
            panel
                .child(cover())
                .child(details(false, words))
                .child(transport(theme, height))
                .child(seek(rest))
                .child(sound(level, 1.))
        }
    };

    el().w_full()
        .h_full()
        .fill(theme.background.color())
        .child(tile([0.05, 0.10, 0.35, 0.05], width, height, muted, radius))
        .children((0..4).map(|index| {
            tile(
                [0.05 + index as f32 * 0.24, 0.28, 0.18, 0.46],
                width,
                height,
                theme.primary.opacity(TILES),
                radius,
            )
        }))
        .child(panel)
        .a11y_hidden()
}

/// One block of the page behind the bar, placed in proportions of the thumbnail.
fn tile(
    [left, top, across, tall]: [f32; 4],
    width: f32,
    height: f32,
    fill: Hsla,
    radius: f32,
) -> El {
    el().absolute(left * width, top * height)
        .size(across * width, tall * height)
        .radius(radius)
        .fill(fill.color())
}

/// A line of text or a level in the bar, `width` across and `thickness` tall.
fn stroke(width: f32, thickness: f32, fill: Hsla, radius: f32) -> El {
    el().size(width.max(0.), thickness)
        .radius(radius)
        .fill(fill.color())
}

/// Three muted marks that suggest the playback buttons without drawing the live icons.
fn transport(theme: &Theme, height: f32) -> El {
    let mark = height * 0.07;
    row().center_y().gap(mark * 0.8).children((0..3).map(|_| {
        el().size(mark, mark)
            .radius(theme.radius * 0.15)
            .fill(theme.muted_foreground.opacity(MARKS).color())
    }))
}
