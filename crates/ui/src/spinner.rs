//! A turning ring for work that has started and not answered yet.

use rune::{El, LoopSpec, icon};

use crate::color::Hsla;
use crate::theme::Theme;

/// How long one turn takes.
const TURN: f64 = 0.9;
/// The spinner's size, the same as a button's icon.
const SIZE: f32 = 16.;

/// A turning ring. Rune holds it still when the system asks for less motion. It is decoration
/// unless given a [`Spinner::name`], since what is loading usually says so itself.
#[derive(Default)]
pub struct Spinner {
    size: Option<f32>,
    color: Option<Hsla>,
    name: Option<String>,
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// The ring's colour, the muted foreground unless set.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Announces the spinner to screen readers as a busy indicator named `name`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let ring = icon(
            icons::get("icons/loader-circle.svg"),
            self.size.unwrap_or(SIZE),
        )
        .color(self.color.unwrap_or(theme.muted_foreground).color())
        .looping(LoopSpec::rotation(TURN));
        match self.name {
            Some(name) => ring.role("progressbar").label(name),
            None => ring.a11y_hidden(),
        }
    }
}
