use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Div, ElementId, FontWeight, Pixels, SharedString, StyleRefinement, Window, div, px,
};

use crate::metrics::{Text, tucked};
use crate::theme::ActiveTheme as _;

/// How tall a key stands against a small control.
const CAP: f32 = 0.7;
/// How much of the key's side shows under its face, the depth that makes it read as a key.
const DEPTH: Pixels = px(2.);
/// The edge of the key around its face.
const EDGE: Pixels = px(1.);

/// A keyboard shortcut drawn as a row of keys, one key cap per part of `keystroke`.
///
/// `keystroke` is spelled the way key bindings are, parts joined by `-`, such as `"ctrl-f"` or
/// `"shift-ctrl-f"`. `secondary` stands for the platform's command key: ⌘ on macOS and Ctrl
/// elsewhere. macOS shows its modifiers as symbols, other platforms name them through Fluent,
/// so German reads Strg.
#[derive(IntoElement)]
pub struct Kbd {
    base: Div,
    keystroke: SharedString,
    /// The id under which the shortcut reaches screen readers, which it does only once given one.
    spoken: Option<ElementId>,
}

impl Kbd {
    pub fn new(keystroke: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            keystroke: keystroke.into(),
            spoken: None,
        }
    }

    /// Exposes the shortcut to screen readers as one line naming its keys, under the element id
    /// `id`. Without it the key caps are drawn but never read.
    pub fn spoken(mut self, id: impl Into<ElementId>) -> Self {
        self.spoken = Some(id.into());
        self
    }
}

impl Styled for Kbd {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Kbd {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            keystroke,
            spoken,
        } = self;
        let theme = cx.theme();
        let overrides = std::mem::take(base.style());
        let height = theme.metrics.control_small * CAP;
        let radius = tucked(theme.radius, window).min(height / 2.);
        // the side of the key, opaque so the face's own fill never mixes with it
        let side = theme.background.blend(theme.border);

        let keys = keystroke.split('-').map(|part| {
            div()
                .flex_none()
                .rounded(radius)
                .bg(side)
                .pt(EDGE)
                .px(EDGE)
                .pb(DEPTH)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(height - EDGE - DEPTH)
                        .min_w(height - EDGE * 2.)
                        .px(theme.metrics.pad / 2.)
                        .rounded((radius - EDGE).max(Pixels::ZERO))
                        .bg(theme.secondary)
                        .text_size(theme.text(Text::Tiny))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .child(label(part)),
                )
        });

        let mut kbd = base
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .children(keys);
        kbd.style().refine(&overrides);
        match spoken {
            Some(id) => {
                let spelled = keystroke
                    .split('-')
                    .map(|part| label(part).to_string())
                    .collect::<Vec<_>>()
                    .join("+");
                kbd.id(id)
                    .role(Role::Label)
                    .aria_value(spelled)
                    .into_any_element()
            }
            None => kbd.into_any_element(),
        }
    }
}

/// What the cap of one part of a keystroke says.
fn label(part: &str) -> SharedString {
    let mac = cfg!(target_os = "macos");
    match (part, mac) {
        ("secondary" | "cmd", true) => SharedString::new_static("⌘"),
        ("ctrl", true) => SharedString::new_static("⌃"),
        ("alt", true) => SharedString::new_static("⌥"),
        ("shift", true) => SharedString::new_static("⇧"),
        ("secondary" | "ctrl", false) => i18n::lookup("key-ctrl", None),
        ("cmd", false) => i18n::lookup("key-super", None),
        ("alt", false) => i18n::lookup("key-alt", None),
        ("shift", false) => i18n::lookup("key-shift", None),
        (part, _) => named(part),
    }
}

/// The cap of a key that is not a modifier.
fn named(part: &str) -> SharedString {
    match part {
        "escape" => i18n::lookup("key-escape", None),
        "enter" => i18n::lookup("key-enter", None),
        "space" => i18n::lookup("key-space", None),
        "tab" => i18n::lookup("key-tab", None),
        "backspace" => i18n::lookup("key-backspace", None),
        "delete" => i18n::lookup("key-delete", None),
        "up" => SharedString::new_static("↑"),
        "down" => SharedString::new_static("↓"),
        "left" => SharedString::new_static("←"),
        "right" => SharedString::new_static("→"),
        key => SharedString::from(key.to_uppercase()),
    }
}
