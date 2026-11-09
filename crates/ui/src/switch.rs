//! An on and off switch: a pill track with a round knob that slides across when it flips.

use rune::{Cursor, El, Tx, el, row};

use crate::motion::Springs;
use crate::paint::Press;
use crate::theme::Theme;

/// The track's height against a small control.
const SCALE: f32 = 0.85;
/// The room between the track's edge and the knob.
const INSET: f32 = 2.;
/// The track's width against its height.
const WIDTH: f32 = 1.75;
const BORDER: f32 = 1.;
/// How far a disabled switch fades.
const FADED: f32 = 0.4;

/// A switch for a setting that is on or off. It draws no text of its own, so give it a
/// [`Switch::name`]; the caller flips its own state in [`Switch::on_press`].
pub struct Switch {
    key: String,
    checked: bool,
    disabled: bool,
    name: Option<String>,
    press: Press,
}

impl Switch {
    pub fn new(key: impl Into<String>, checked: bool) -> Self {
        Self {
            key: key.into(),
            checked,
            disabled: false,
            name: None,
            press: Press::default(),
        }
    }

    /// Names the switch for screen readers, which otherwise hear nothing but its state.
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
        let height = (theme.metrics.control_small * SCALE).round();
        let width = (height * WIDTH).round();
        let knob = height - (INSET + BORDER) * 2.;
        let travel = width - height;
        let (track, edge, hover, ink) = match self.checked {
            true => (
                theme.primary,
                theme.primary,
                theme.primary_hover,
                theme.primary_foreground,
            ),
            false => (
                theme.muted,
                theme.border,
                theme.secondary_hover,
                theme.muted_foreground,
            ),
        };
        let thumb = el()
            .key("knob")
            .size(knob, knob)
            .radius(knob / 2.)
            .ml(match self.checked {
                true => travel,
                false => 0.,
            })
            .fill(ink.color())
            .motion(Springs::QUICK)
            .a11y_hidden();
        let mut switch = row()
            .key(self.key)
            .role("switch")
            .checked(self.checked)
            .center_y()
            .size(width, height)
            .padding(INSET)
            .radius(height / 2.)
            .fill(track.color())
            .border(BORDER, edge.color())
            .motion(Springs::CONTROL)
            .child(thumb);
        if let Some(name) = self.name {
            switch = switch.label(name);
        }
        let switch = match self.disabled {
            true => switch.disabled(true).opacity(FADED),
            false => switch
                .focusable(true)
                .hover(hover.color())
                .cursor(Cursor::Pointer),
        };
        let enabled = !self.disabled;
        self.press.apply(switch, enabled)
    }
}
