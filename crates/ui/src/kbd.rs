//! A keyboard shortcut drawn as a row of key caps.

use rune::{El, TextStyle, el, row, text};

use crate::label::Weight;
use crate::metrics::{Text, tucked};
use crate::theme::Theme;

/// How tall a key stands against a small control.
const CAP: f32 = 0.7;
/// How much of the key's side shows under its face, the depth that makes it read as a key.
const DEPTH: f32 = 2.;
/// The edge of the key around its face.
const EDGE: f32 = 1.;
/// The space between two caps.
const GAP: f32 = 2.;

/// A shortcut, one cap per part of `keystroke`. `keystroke` is spelled the way key bindings are,
/// parts joined by `-`, such as `"ctrl-f"` or `"shift-secondary-f"`. `secondary` stands for the
/// platform's command key: ⌘ on macOS and Ctrl elsewhere. macOS shows modifiers as symbols,
/// other platforms name them through Fluent, so German reads Strg.
pub struct Kbd {
    keystroke: String,
    /// The key under which the shortcut reaches screen readers, which it does only once given one.
    spoken: Option<String>,
}

impl Kbd {
    pub fn new(keystroke: impl Into<String>) -> Self {
        Self {
            keystroke: keystroke.into(),
            spoken: None,
        }
    }

    /// Exposes the shortcut to screen readers as one line naming its keys, under `key`. Without
    /// it the caps are drawn but never read.
    pub fn spoken(mut self, key: impl Into<String>) -> Self {
        self.spoken = Some(key.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let height = theme.metrics.control_small * CAP;
        let radius = tucked(theme.radius, theme.font_size).min(height / 2.);
        // the side of the key, opaque so the face's own fill never mixes with it
        let side = theme.background.blend(theme.border);
        let style = TextStyle::new(theme.text(Text::Tiny), Weight::MEDIUM);
        let caps = self.keystroke.split('-').map(|part| {
            el().radius(radius)
                .fill(side.color())
                .pt(EDGE)
                .px(EDGE)
                .pb(DEPTH)
                .child(
                    row()
                        .center()
                        .h(height - EDGE - DEPTH)
                        .min_w(height - EDGE * 2.)
                        .px(theme.metrics.pad / 2.)
                        .radius((radius - EDGE).max(0.))
                        .fill(theme.secondary.color())
                        .child(
                            text(cap(part))
                                .font(style)
                                .color(theme.muted_foreground.color()),
                        ),
                )
                .a11y_hidden()
        });
        let kbd = row().center_y().gap(GAP).children(caps);
        match self.spoken {
            Some(key) => {
                let spelled = self.keystroke.split('-').map(cap).collect::<Vec<_>>();
                kbd.key(key).role("text").label(spelled.join("+"))
            }
            None => kbd.a11y_hidden(),
        }
    }
}

/// What the cap of one part of a keystroke says.
fn cap(part: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let word = |key: &str| i18n::lookup(key, None).into_owned();
    match (part, mac) {
        ("secondary" | "cmd", true) => "⌘".to_owned(),
        ("ctrl", true) => "⌃".to_owned(),
        ("alt", true) => "⌥".to_owned(),
        ("shift", true) => "⇧".to_owned(),
        ("secondary" | "ctrl", false) => word("key-ctrl"),
        ("cmd", false) => word("key-super"),
        ("alt", false) => word("key-alt"),
        ("shift", false) => word("key-shift"),
        ("escape", _) => word("key-escape"),
        ("enter", _) => word("key-enter"),
        ("space", _) => word("key-space"),
        ("tab", _) => word("key-tab"),
        ("backspace", _) => word("key-backspace"),
        ("delete", _) => word("key-delete"),
        ("up", _) => "↑".to_owned(),
        ("down", _) => "↓".to_owned(),
        ("left", _) => "←".to_owned(),
        ("right", _) => "→".to_owned(),
        (key, _) => key.to_uppercase(),
    }
}
