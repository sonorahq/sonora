use i18n::{SharedString, t};

const HEADER: f32 = 0.75;

pub const LEADING: f32 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Square,
    Subtle,
    Rounded,
    Round,
}

impl Rounding {
    pub const ALL: [Self; 4] = [Self::Square, Self::Subtle, Self::Rounded, Self::Round];

    pub fn id(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Subtle => "subtle",
            Self::Rounded => "rounded",
            Self::Round => "round",
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Square => t!("corners-square"),
            Self::Subtle => t!("corners-subtle"),
            Self::Rounded => t!("corners-rounded"),
            Self::Round => t!("corners-round"),
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "square" => Self::Square,
            "rounded" => Self::Rounded,
            "round" => Self::Round,
            _ => Self::Subtle,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            Self::Square => 0.,
            Self::Subtle => 6.,
            Self::Rounded => 10.,
            Self::Round => 20.,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub row: f32,
    pub header: f32,
    pub pad: f32,
    pub inset: f32,
    pub control: f32,
    pub control_small: f32,
    pub field: f32,
    pub title_bar: f32,
    pub player_bar: f32,
    pub list_row: f32,
    pub thumb: f32,
    pub cover: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new(14.)
    }
}

impl Metrics {
    pub fn new(base: f32) -> Self {
        let text = base;
        let roomy = |step: f32, lines: f32| step.max((text * lines).round());
        let row = roomy(42., 2.4);

        Self {
            row,
            header: (row * HEADER).round(),
            pad: 8.,
            inset: 24.,
            control: roomy(32., 2.),
            control_small: roomy(26., 1.7),
            field: roomy(40., 2.6),
            title_bar: roomy(36., 2.4),
            player_bar: roomy(76., 5.),
            list_row: roomy(52., 3.2),
            thumb: 34.,
            cover: 140.,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Text {
    Tiny,
    Small,
    Label,
    Body,
    Large,
    Title,
    Display,
}

impl Text {
    pub fn ratio(self) -> f32 {
        match self {
            Self::Tiny => 0.77,
            Self::Small => 0.85,
            Self::Label => 0.92,
            Self::Body => 1.,
            Self::Large => 1.38,
            Self::Title => 1.69,
            Self::Display => 2.15,
        }
    }
}

const INSET: f32 = 0.25;
const BORDER: f32 = 1.;

/// The radius of a box inset by a quarter of the text size and a border inside one rounded by
/// `radius`, so nested corners stay concentric.
pub fn tucked(radius: f32, font_size: f32) -> f32 {
    (radius - font_size * INSET - BORDER).max(0.)
}
