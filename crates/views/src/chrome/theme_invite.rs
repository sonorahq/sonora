//! The question before a theme or family from a `sonora://theme/` link is installed, with its
//! name in the title, a miniature of the window cut into its variants, their names under it,
//! and Install or Cancel. `state::Links` holds the offer and does the installing. This only
//! draws it.

use i18n::t;
use rune::{Cx, El, Tx, col, el, text};
use ui::{Button, Modal, Text, Theme, Weight};

use crate::{Ui, act, act_tx};

/// How many variants of a family the miniature shows side by side at most.
const SLICES: usize = 4;
/// How many times a theme card's size the preview is drawn.
const PREVIEW: f32 = 2.;
/// The miniature window inside the card's border at a scale of 1, and the border itself.
const WIDTH: f32 = 148.;
const HEIGHT: f32 = 94.;
const EDGE: f32 = 1.;
/// How much of the theme's corner radius the miniature's tiles keep.
const RADIUS_SCALE: f32 = 0.3;
/// How strongly the bars show against what they sit on, the first bar of the sidebar standing
/// for the active entry, and how much of its strength the accent tile keeps.
const BAR: f32 = 0.2;
const ACTIVE_BAR: f32 = 0.4;
const ACCENT: f32 = 0.9;
/// The sidebar's width. It runs the full height, so it alone carries the left corners.
const SIDEBAR: f32 = 38.;
/// The columns a cut between two palettes may run down, so a cut never splits a tile.
const SEAMS: [f32; 3] = [SIDEBAR, 76., 110.];
/// The sidebar's entries and the content's heading, as left, top, width and height.
const ENTRIES: [[f32; 4]; 3] = [[8., 14., 22., 5.], [8., 25., 16., 5.], [8., 36., 19., 5.]];
const HEADING: [f32; 4] = [45., 14., 34., 5.];
/// The row of tiles under the heading by their left edges, the first one in the accent.
const TILES: [f32; 3] = [45., 79., 113.];
const TILE_TOP: f32 = 27.;
const TILE: f32 = 28.;
/// The player floating at the foot of the content, and its play button.
const PLAYER: [f32; 4] = [45., 66., 96., 14.];
const PLAY: [f32; 4] = [50., 70., 6., 6.];

/// Mounts the dialog into `layer` while a link offers a theme.
pub(crate) fn mount(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) {
    let _ = cx;
    let theme = &ui.theme;
    let Some(offer) = ui.store.links.theme_offer() else {
        return;
    };
    let family = &offer.family;
    let palettes: Vec<Theme> = family
        .variants
        .iter()
        .take(SLICES)
        .map(|variant| variant.palette())
        .collect();
    let names = family
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let (width, height) = (WIDTH * PREVIEW + EDGE * 2., HEIGHT * PREVIEW + EDGE * 2.);
    let mut preview = col()
        .key("theme-install-preview")
        .items_center()
        .gap(theme.metrics.pad)
        .child(
            el().size(width, height)
                .radius(theme.radius)
                .border(EDGE, theme.border.color())
                .role("img")
                .label(names.clone())
                .child(sliced(&palettes, theme.radius, PREVIEW)),
        );
    if family.variants.len() > 1 {
        preview = preview.child(
            text(names)
                .font(ui::font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        );
    }
    Modal::<Ui>::new(
        "theme-install",
        t!("theme-install-title", name = family.name.as_str()),
    )
    // the modal pads its body by twice `pad` on each side, so the preview lines up with the
    // title and the buttons
    .width(width + theme.metrics.pad * 4. + EDGE * 2.)
    .detail(t!("theme-install-detail"))
    .child(preview)
    .action(
        Button::new("decline-theme")
            .ghost()
            .label(t!("common-cancel"))
            .on_press(act(|ui: &mut Ui| ui.store.links.decline_theme()))
            .build(theme),
    )
    .action(
        Button::new("install-theme")
            .primary()
            .label(t!("theme-install-confirm"))
            .on_press_tx(act_tx(install))
            .build(theme)
            .default_button(),
    )
    .on_dismiss(act(|ui: &mut Ui| ui.store.links.decline_theme()))
    .build(theme, layer);
}

/// Installs the theme and fades the window into it on the theme spring, the way choosing a
/// card does.
fn install(ui: &mut Ui, tx: &mut Tx) {
    if ui.store.install_theme_offer() {
        tx.color_motion(ui::Springs::THEME);
    }
}

/// The miniature cut into one vertical slice per palette, left to right, each slice the same
/// window painted in its own palette so the layout runs on across the cuts, which run down
/// `SEAMS`. `scale` multiplies every shape.
fn sliced(palettes: &[Theme], radius: f32, scale: f32) -> El {
    let count = palettes.len().max(1);
    let cuts = (1..count).map(|index| {
        let even = WIDTH * index as f32 / count as f32;
        SEAMS
            .into_iter()
            .min_by(|a, b| (a - even).abs().total_cmp(&(b - even).abs()))
            .unwrap_or(even)
    });
    let edges: Vec<f32> = std::iter::once(0.)
        .chain(cuts)
        .chain(std::iter::once(WIDTH))
        .collect();
    let mark = (radius * RADIUS_SCALE * scale).floor();
    let outer = (radius - EDGE).max(0.).min(SIDEBAR * scale / 2.);
    el().size(WIDTH * scale, HEIGHT * scale)
        .absolute(EDGE, EDGE)
        .a11y_hidden()
        .children(palettes.iter().zip(edges.windows(2)).map(|(palette, edge)| {
            let (left, right) = (edge[0], edge[1]);
            el().absolute(left * scale, 0.)
                .size((right - left) * scale, HEIGHT * scale)
                .clip_rounded(0.)
                .child(miniature(palette, outer, mark, scale).absolute(-left * scale, 0.))
        }))
}

/// The Sonora window in `palette`'s colours as a few soft, flat shapes: the sidebar with three
/// entries, a heading over a row of tiles, the first in the accent, and a floating player with
/// its play button.
fn miniature(palette: &Theme, outer: f32, mark: f32, scale: f32) -> El {
    let panel = match palette.sidebar == palette.background {
        true => palette.secondary,
        false => palette.sidebar,
    };
    let rect = |[x, y, w, h]: [f32; 4]| el().absolute(x * scale, y * scale).size(w * scale, h * scale);
    let bar = |shape: [f32; 4], strength: f32| {
        rect(shape)
            .radius(9999.)
            .fill(palette.foreground.opacity(strength).color())
    };
    el().size(WIDTH * scale, HEIGHT * scale)
        .radius(outer)
        .fill(palette.background.color())
        .child(
            rect([0., 0., SIDEBAR, HEIGHT])
                .corners([outer, 0., 0., outer])
                .fill(panel.color()),
        )
        .children(ENTRIES.into_iter().enumerate().map(|(index, entry)| {
            bar(
                entry,
                match index {
                    0 => ACTIVE_BAR,
                    _ => BAR,
                },
            )
        }))
        .child(bar(HEADING, ACTIVE_BAR))
        .children(TILES.into_iter().enumerate().map(|(index, left)| {
            let fill = match index {
                0 => palette.primary.opacity(ACCENT),
                _ => palette.muted,
            };
            rect([left, TILE_TOP, TILE, TILE])
                .radius(mark)
                .fill(fill.color())
        }))
        .child(rect(PLAYER).radius(9999.).fill(panel.color()))
        .child(rect(PLAY).radius(9999.).fill(palette.primary.color()))
}
