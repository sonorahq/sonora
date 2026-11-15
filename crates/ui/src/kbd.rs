use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Div, ElementId, FontWeight, Pixels, SharedString, StyleRefinement, Window, div, px,
};

use crate::metrics::{Text, tucked};
use crate::theme::ActiveTheme as _;

/// How tall a key stands against a small control.
const CAP: f32 = 0.7;
/// How tall a large key stands against a small control.
const LARGE_CAP: f32 = 0.9;
/// How much of the key's side shows under its face, the depth that makes it read as a key.
const DEPTH: Pixels = px(2.);
/// The depth of a large key.
const LARGE_DEPTH: Pixels = px(3.);
/// The edge of the key around its face.
const EDGE: Pixels = px(1.);

/// A keyboard shortcut drawn as a row of keys, one key cap per part of `keystroke`.
///
/// `keystroke` is spelled the way key bindings are, parts joined by `-`, such as `"ctrl-f"` or
/// `"ctrl-shift-f"`, and the modifiers are drawn in the platform's own order whatever order they
/// come in. `secondary` stands for the platform's command key: ⌘ on macOS and Ctrl
/// elsewhere. macOS shows its modifiers as symbols, other platforms name them through Fluent,
/// so German reads Strg.
#[derive(IntoElement)]
pub struct Kbd {
    base: Div,
    keystroke: SharedString,
    /// The id under which the shortcut reaches screen readers, which it does only once given one.
    spoken: Option<ElementId>,
    /// Whether the keys are the content rather than a hint beside it.
    large: bool,
}

impl Kbd {
    pub fn new(keystroke: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            keystroke: keystroke.into(),
            spoken: None,
            large: false,
        }
    }

    /// Exposes the shortcut to screen readers as one line naming its keys, under the element id
    /// `id`. Without it the key caps are drawn but never read.
    pub fn spoken(mut self, id: impl Into<ElementId>) -> Self {
        self.spoken = Some(id.into());
        self
    }

    /// Draws taller keys with their labels in the text colour and a deeper side, for a list where
    /// the shortcut is the thing being read, such as the shortcuts in settings.
    pub fn large(mut self) -> Self {
        self.large = true;
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
            large,
        } = self;
        let theme = cx.theme();
        let overrides = std::mem::take(base.style());
        let (cap, depth, text, ink) = match large {
            true => (LARGE_CAP, LARGE_DEPTH, Text::Small, theme.foreground),
            false => (CAP, DEPTH, Text::Tiny, theme.muted_foreground),
        };
        let height = theme.metrics.control_small * cap;
        let radius = tucked(theme.radius, window).min(height / 2.);
        // the side of the key, opaque so the face's own fill never mixes with it
        let side = theme.background.blend(theme.border);

        let keys = parts(&keystroke).into_iter().map(|part| {
            div()
                .flex_none()
                .rounded(radius)
                .bg(side)
                .pt(EDGE)
                .px(EDGE)
                .pb(depth)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(height - EDGE - depth)
                        .min_w(height - EDGE * 2.)
                        .px(theme.metrics.pad / 2.)
                        .rounded((radius - EDGE).max(Pixels::ZERO))
                        .bg(theme.secondary)
                        .text_size(theme.text(text))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ink)
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
                let spelled = parts(&keystroke)
                    .into_iter()
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

/// The parts of `keystroke`, one per key, the modifiers in the order the platform writes them and
/// the key last. A trailing `--` is the minus key after a modifier.
fn parts(keystroke: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = match keystroke.strip_suffix("--") {
        Some(head) => head.split('-').chain(["-"]).collect(),
        None if keystroke == "-" => vec!["-"],
        None => keystroke.split('-').collect(),
    };
    if let Some((_, modifiers)) = parts.split_last_mut() {
        modifiers.sort_by_key(|modifier| rank(modifier));
    }
    parts
}

/// Where a modifier stands among the others: Ctrl, Super, Alt, Shift elsewhere, and the
/// ⌃⌥⇧⌘ order of macOS.
fn rank(modifier: &str) -> u8 {
    let mac = cfg!(target_os = "macos");
    match (modifier, mac) {
        ("ctrl", _) | ("secondary", false) => 0,
        ("cmd" | "super" | "win", false) => 1,
        ("alt", _) => 2,
        ("shift", _) => 3,
        ("cmd" | "secondary", true) => 4,
        _ => 5,
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
        ("cmd" | "super" | "win", false) => i18n::lookup("key-super", None),
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
