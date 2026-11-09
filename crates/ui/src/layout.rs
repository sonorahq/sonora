pub const ALWAYS: f32 = 0.;
pub const SNUG: f32 = 420.;
pub const ROOMY: f32 = 620.;
pub const WIDE: f32 = 740.;
pub const VAST: f32 = 1180.;

pub const MIN_CONTENT: f32 = 200.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// How much width a view has, on the one ladder every layout decision reads.
pub enum Room {
    Tight,
    Snug,
    Roomy,
    Wide,
    Vast,
}

impl Room {
    pub fn of(width: f32) -> Self {
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
}
