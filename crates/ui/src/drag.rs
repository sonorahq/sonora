//! What a drag inside the window shows: a ghost chip that follows the pointer, the line that marks
//! where a dragged row lands, the grip that marks a row as movable, and the geometry that turns a
//! pointer position into a drop place.
//!
//! The ghost follows the pointer through a live offset, so a drag handler that only moves it says
//! `Tx::live_only` and the view does not run for every pointer move. Find the target under the
//! pointer with `Tx::element_at_in`, and its rect with the target's `on_layout`.

use rune::{El, Live, TextStyle, el, icon, row, text};

use crate::artwork::{Artwork, Avatar, Cover};
use crate::metrics::Text;
use crate::theme::Theme;

/// The widest a ghost chip gets before its label ends in an ellipsis.
const CHIP: f32 = 240.;
/// How far the chip sits right of and below the pointer, so it never hides what it is over.
const NUDGE: f32 = 8.;
const ART: f32 = 20.;
const MARKER: f32 = 2.;
/// How far a drop marker stands in from its row's sides.
const MARKER_INSET: f32 = 8.;
/// How far past a row's top and bottom a pointer still drops beside it.
const SLACK: f32 = 10.;
const CHIP_PAD: [f32; 2] = [8., 4.];
const CHIP_GAP: f32 = 8.;

/// Which side of a row a dragged row lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Above,
    Below,
}

/// Where a drag's ghost is, in window points. Keep one in the state that owns the drag, call
/// [`Carry::follow`] from the drag handler, and mount the [`Ghost`] at the view root.
#[derive(Clone)]
pub struct Carry {
    at: Live<[f32; 2]>,
}

impl Default for Carry {
    fn default() -> Self {
        Self::new()
    }
}

impl Carry {
    pub fn new() -> Self {
        Self {
            at: Live::new([0., 0.]),
        }
    }

    /// Moves the ghost under the pointer. A handler that does nothing else says `Tx::live_only`.
    pub fn follow(&self, drag: &rune::Drag) {
        self.at.set(drag.window_point());
    }

    /// Moves the ghost to `at`, in window points.
    pub fn place(&self, at: [f32; 2]) {
        self.at.set(at);
    }

    /// Where the ghost is, in window points.
    pub fn at(&self) -> [f32; 2] {
        self.at.get()
    }
}

/// The chip that follows the pointer while something is dragged: an optional cover, the label and
/// a badge saying what the drop does. It takes no pointer input, so the targets under it still do.
pub struct Ghost<'a> {
    label: String,
    cover: Option<Option<&'a str>>,
    round: bool,
    fallback: Option<&'a str>,
    badge: Option<&'a str>,
}

impl<'a> Ghost<'a> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            cover: None,
            round: false,
            fallback: None,
            badge: None,
        }
    }

    /// Leads the chip with the cover at `url`, or its placeholder when there is none.
    pub fn art(mut self, url: Option<&'a str>) -> Self {
        self.cover = Some(url);
        self
    }

    /// The placeholder glyph for the cover, as `"icons/<name>.svg"`.
    pub fn fallback(mut self, icon: &'a str) -> Self {
        self.fallback = Some(icon);
        self
    }

    /// Draws the cover round, as for a person or an artist.
    pub fn circle(mut self) -> Self {
        self.round = true;
        self
    }

    /// An icon after the label that says what the drop does, such as copy or move.
    pub fn badge(mut self, icon: &'a str) -> Self {
        self.badge = Some(icon);
        self
    }

    /// The ghost, placed by `carry`. Mount it at the view root, over everything.
    pub fn build(self, artwork: &Artwork, carry: &Carry, theme: &Theme) -> El {
        let leading = self.cover.map(|url| match self.round {
            true => {
                let avatar = Avatar::new(artwork, url).size(ART);
                match self.fallback {
                    Some(fallback) => avatar.fallback(fallback).build(theme),
                    None => avatar.build(theme),
                }
            }
            false => {
                let cover = Cover::new(artwork, url).size(ART).radius(theme.radius);
                match self.fallback {
                    Some(fallback) => cover.fallback(fallback).build(theme),
                    None => cover.build(theme),
                }
            }
        });
        let mut chip = row()
            .max_w(CHIP)
            .gap(CHIP_GAP)
            .px(CHIP_PAD[0])
            .py(CHIP_PAD[1])
            .center_y()
            .radius(theme.radius)
            .fill(theme.secondary.color());
        if let Some(leading) = leading {
            chip = chip.child(leading);
        }
        chip = chip.child(
            text(self.label)
                .font(TextStyle::new(theme.text(Text::Body), 400.))
                .color(theme.foreground.color())
                .truncate()
                .shrink()
                .min_w(0.),
        );
        if let Some(badge) = self.badge {
            chip = chip.child(
                icon(icons::get(badge), theme.text(Text::Small))
                    .color(theme.muted_foreground.color()),
            );
        }
        el().absolute(NUDGE, NUDGE)
            .live_offset(&carry.at)
            .inert()
            .a11y_hidden()
            .child(chip)
    }
}

/// The line across a row's top or bottom edge that shows where a dragged row lands. Put it in the
/// row, which lays it over its content.
pub fn drop_marker(edge: Edge, theme: &Theme) -> El {
    let line = el()
        .h(MARKER)
        .radius(MARKER / 2.)
        .fill(theme.primary.color())
        .a11y_hidden();
    match edge {
        Edge::Above => line.pin(Some(MARKER_INSET), Some(0.), Some(MARKER_INSET), None),
        Edge::Below => line.pin(Some(MARKER_INSET), None, Some(MARKER_INSET), Some(0.)),
    }
}

/// The dotted handle at the start of a row that can be dragged to a new place. It only marks the
/// row, so the caller wires the drag itself.
pub fn grip(theme: &Theme) -> El {
    icon(
        icons::get("icons/grip-vertical.svg"),
        theme.text(Text::Body),
    )
    .color(theme.muted_foreground.color())
    .a11y_hidden()
}

/// The gap a drop at `at` lands in for the row at `index` laid out at `rect` (`[x, y, w, h]`, both
/// in window points): `index` above the row's middle, `index + 1` below it, and none when the
/// pointer is past the row and its slack.
pub fn drop_gap(rect: [f32; 4], at: [f32; 2], index: usize) -> Option<usize> {
    let slack = SLACK.min(rect[3] / 2.);
    let inside = at[0] >= rect[0] - slack
        && at[0] <= rect[0] + rect[2] + slack
        && at[1] >= rect[1] - slack
        && at[1] <= rect[1] + rect[3] + slack;
    if !inside {
        return None;
    }
    Some(match at[1] < rect[1] + rect[3] / 2. {
        true => index,
        false => index + 1,
    })
}

/// Whether `at` sits in the middle half of `rect`, where a drop lands on the row itself rather
/// than in the gap above or below it.
pub fn drop_into(rect: [f32; 4], at: [f32; 2]) -> bool {
    let quarter = rect[3] / 4.;
    at[0] >= rect[0]
        && at[0] <= rect[0] + rect[2]
        && at[1] > rect[1] + quarter
        && at[1] < rect[1] + rect[3] - quarter
}
