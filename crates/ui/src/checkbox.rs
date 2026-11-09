//! A checkbox with an optional label beside it. The tick pops in on a spring when it is checked.

use rune::{Cursor, El, TextStyle, Tx, el, icon, row, text};

use crate::label::Weight;
use crate::metrics::Text;
use crate::motion::Springs;
use crate::paint::Press;
use crate::theme::Theme;

/// The box's side against a small control.
const SCALE: f32 = 0.66;
/// The tick's size against the box.
const MARK: f32 = 0.68;
const GAP: f32 = 8.;
const BORDER: f32 = 1.;
/// How small an unchecked tick shrinks to on its way out.
const TUCKED: f32 = 0.5;
/// How far a disabled checkbox fades.
const FADED: f32 = 0.4;

/// A checkbox. The box and its label are one pressable element; the caller flips its own state
/// in [`Checkbox::on_press`].
pub struct Checkbox {
    key: String,
    checked: bool,
    disabled: bool,
    label: Option<String>,
    /// What a screen reader calls the checkbox in place of its label.
    name: Option<String>,
    press: Press,
}

impl Checkbox {
    pub fn new(key: impl Into<String>, checked: bool) -> Self {
        Self {
            key: key.into(),
            checked,
            disabled: false,
            label: None,
            name: None,
            press: Press::default(),
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Names the checkbox for screen readers in place of its label, for one whose label sits
    /// elsewhere in the row or that has none.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_press<S: 'static>(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.press.press(f);
        self
    }

    pub fn on_press_tx<S: 'static>(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.press.press_tx(f);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let side = (theme.metrics.control_small * SCALE).round();
        let radius = theme.radius.min(side / 3.);
        let (fill, edge, ink) = match self.checked {
            true => (theme.primary, theme.primary, theme.primary_foreground),
            false => (theme.muted, theme.border, theme.muted),
        };
        let mark = icon(icons::get("icons/check.svg"), (side * MARK).round())
            .key("tick")
            .color(ink.color())
            .scale(match self.checked {
                true => 1.,
                false => TUCKED,
            })
            .opacity(match self.checked {
                true => 1.,
                false => 0.,
            })
            .motion(Springs::PRESS);
        let square = row()
            .key("box")
            .center()
            .size(side, side)
            .radius(radius)
            .fill(fill.color())
            .border(BORDER, edge.color())
            .motion(Springs::CONTROL)
            .child(mark)
            .a11y_hidden();
        let mut checkbox = row()
            .key(self.key)
            .role("checkbox")
            .checked(self.checked)
            .center_y()
            .gap(GAP)
            .child(square);
        if let Some(name) = self.name.clone().or_else(|| self.label.clone()) {
            checkbox = checkbox.label(name);
        }
        if let Some(label) = self.label {
            checkbox = checkbox.child(
                text(label)
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.foreground.color())
                    .a11y_hidden(),
            );
        }
        let checkbox = match self.disabled {
            true => checkbox.disabled(true).opacity(FADED),
            false => checkbox.focusable(true).cursor(Cursor::Pointer),
        };
        let enabled = !self.disabled;
        self.press.apply(checkbox, enabled)
    }
}

/// The hole in the middle of a chosen [`crate::Radio`], kept here so both marks size alike.
pub(crate) fn dot(side: f32, color: rune::Color) -> El {
    el().size(side, side).radius(side / 2.).fill(color)
}
