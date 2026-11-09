//! Placeholders: a pulsing block where content is still loading, and the initials of a name
//! where its picture is missing.

use rune::{El, LoopSpec, TextStyle, el, row, text};

use crate::label::Weight;
use crate::theme::Theme;

/// How long one breath of the pulse takes, in and out.
const PULSE: f64 = 0.7;
/// The pulse's dimmest and brightest opacity.
const DIM: f32 = 0.55;
const BRIGHT: f32 = 0.7;
/// A radius larger than any block, which the renderer caps at half its shorter side.
const ROUND: f32 = 9999.;
/// How large the letters stand against the circle.
const LETTERS: f32 = 0.34;

/// A muted block that breathes while what it stands for loads. The caller gives it its size by
/// chaining `.size(w, h)` (or `.w`, `.h`) on the built element. It is decoration.
#[derive(Default)]
pub struct Skeleton {
    circle: bool,
}

impl Skeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        el().fill(theme.muted.color())
            .radius(match self.circle {
                true => ROUND,
                false => theme.radius,
            })
            .looping(LoopSpec::opacity(PULSE, 0., DIM, BRIGHT))
            .a11y_hidden()
    }
}

/// The first letters of up to two words of a name in a muted circle, in place of a picture.
pub struct Initials {
    name: String,
    size: f32,
}

impl Initials {
    pub fn new(name: impl Into<String>, size: f32) -> Self {
        Self {
            name: name.into(),
            size,
        }
    }

    pub fn build(self, theme: &Theme) -> El {
        let letters = self
            .name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase();
        row()
            .center()
            .size(self.size, self.size)
            .radius(self.size / 2.)
            .fill(theme.secondary.color())
            .child(
                text(letters)
                    .font(TextStyle::new(self.size * LETTERS, Weight::NORMAL))
                    .color(theme.muted_foreground.color()),
            )
            .a11y_hidden()
    }
}
