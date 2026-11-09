//! The round mark that says which one of a set is chosen.

use rune::{El, row};

use crate::checkbox::dot;
use crate::motion::Springs;
use crate::theme::Theme;

/// The mark's side against a small control.
const SCALE: f32 = 0.6;
/// How much of the mark's width the hole in the middle takes.
const HOLE: f32 = 0.34;
const BORDER: f32 = 1.;

/// Filled in the accent colour with a hole punched in the middle when chosen, an empty ring when
/// not. It takes no presses of its own, so the row around it is what the pointer presses.
pub struct Radio {
    key: String,
    selected: bool,
    /// What a screen reader calls the option the mark stands for.
    name: Option<String>,
    /// Whether the mark is a node of its own in the accessibility tree.
    announced: bool,
}

impl Radio {
    pub fn new(key: impl Into<String>, selected: bool) -> Self {
        Self {
            key: key.into(),
            selected,
            name: None,
            announced: true,
        }
    }

    /// Names the option for screen readers, which otherwise hear nothing but its state.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Leaves the mark out of the accessibility tree, for a row that already reports the choice
    /// and its name itself.
    pub fn silent(mut self) -> Self {
        self.announced = false;
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let side = (theme.metrics.control_small * SCALE).round();
        let hole = (side * HOLE).round();
        let (fill, edge, middle) = match self.selected {
            true => (theme.primary, theme.primary, theme.primary_foreground),
            false => (
                theme.primary.opacity(0.),
                theme.border,
                theme.primary_foreground.opacity(0.),
            ),
        };
        let mark = row()
            .key(self.key)
            .center()
            .size(side, side)
            .radius(side / 2.)
            .fill(fill.color())
            .border(BORDER, edge.color())
            .motion(Springs::CONTROL)
            .child(
                dot(hole, middle.color())
                    .key("hole")
                    .scale(match self.selected {
                        true => 1.,
                        false => 0.,
                    })
                    .motion(Springs::PRESS)
                    .a11y_hidden(),
            );
        match self.announced {
            true => {
                let mark = mark.role("radio").checked(self.selected);
                match self.name {
                    Some(name) => mark.label(name),
                    None => mark,
                }
            }
            false => mark.a11y_hidden(),
        }
    }
}
