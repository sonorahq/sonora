use gpui::{Pixels, px};

pub const ALWAYS: Pixels = Pixels::ZERO;
pub const SNUG: Pixels = px(420.);
pub const ROOMY: Pixels = px(620.);
pub const WIDE: Pixels = px(740.);
pub const VAST: Pixels = px(1180.);

pub const MIN_CONTENT: Pixels = px(200.);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Room {
    Tight,
    Snug,
    Roomy,
    Wide,
    Vast,
}

impl Room {
    pub fn of(width: Pixels) -> Self {
        match width {
            width if width >= VAST => Self::Vast,
            width if width >= WIDE => Self::Wide,
            width if width >= ROOMY => Self::Roomy,
            width if width >= SNUG => Self::Snug,
            _ => Self::Tight,
        }
    }

    pub fn fits(self, step: Room) -> bool {
        self >= step
    }

    /// The narrowest width that counts as this step, for an element that has to be as wide as a
    /// window of this room.
    pub fn least(self) -> Pixels {
        match self {
            Self::Tight => ALWAYS,
            Self::Snug => SNUG,
            Self::Roomy => ROOMY,
            Self::Wide => WIDE,
            Self::Vast => VAST,
        }
    }
}
