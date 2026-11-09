//! The question before a theme or family from a `sonora://theme/` link is installed, with its
//! name in the title, a miniature of the window cut into its variants, their names under it,
//! and Install or Cancel. `state::Links` holds the offer and does the installing. This only
//! draws it.

use i18n::t;
use rune::{Cx, El, Tx, col, el, text};

use ui::{Button, Modal, Text, Theme, Weight};

use crate::shared::theme_miniature::{SLICES, framed, sliced};
use crate::{Ui, act, act_tx};

/// How many times a theme card's size the preview is drawn.
const PREVIEW: f32 = 2.;
/// The modal panel's border, which counts toward its width.
const BORDER: f32 = 1.;

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
    let (width, height) = framed(PREVIEW);
    let mut preview = col()
        .key("theme-install-preview")
        .items_center()
        .gap(theme.metrics.pad)
        .child(
            el().size(width, height)
                .radius(theme.radius)
                .border(BORDER, theme.border.color())
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
    .width(width + theme.metrics.pad * 4. + BORDER * 2.)
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
