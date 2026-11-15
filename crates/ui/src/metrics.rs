use gpui::{Pixels, SharedString, TextRun, Window, px};
use i18n::t;

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

    pub fn radius(self) -> Pixels {
        match self {
            Self::Square => px(0.),
            Self::Subtle => px(6.),
            Self::Rounded => px(10.),
            Self::Round => px(20.),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub row: Pixels,
    pub header: Pixels,
    pub pad: Pixels,
    pub inset: Pixels,
    pub control: Pixels,
    pub control_small: Pixels,
    pub field: Pixels,
    pub title_bar: Pixels,
    pub player_bar: Pixels,
    pub list_row: Pixels,
    pub thumb: Pixels,
    pub cover: Pixels,
    /// The widest a page grows, its side insets included. A wider content area centres the
    /// page and gives what is left to its side margins.
    pub page: Pixels,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new(px(14.))
    }
}

impl Metrics {
    pub fn new(base: Pixels) -> Self {
        let text = base / px(1.);
        let roomy = |step: f32, lines: f32| px(step.max((text * lines).round()));
        let row = roomy(42., 2.4);

        Self {
            row,
            header: px((row / px(1.) * HEADER).round()),
            pad: px(8.),
            inset: px(24.),
            control: roomy(32., 2.),
            control_small: roomy(26., 1.7),
            field: roomy(40., 2.6),
            title_bar: roomy(36., 2.4),
            player_bar: roomy(76., 5.),
            list_row: roomy(52., 3.2),
            thumb: px(34.),
            cover: px(140.),
            page: px(1600.),
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
    pub(crate) fn ratio(self) -> f32 {
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
const BORDER: Pixels = px(1.);

pub fn snapped(value: Pixels, window: &Window) -> Pixels {
    let scale = window.scale_factor();
    px((value / px(1.) * scale).round() / scale)
}

pub fn text_width(text: impl Into<SharedString>, window: &Window) -> Pixels {
    let text = text.into();
    if text.is_empty() {
        return Pixels::ZERO;
    }

    let style = window.text_style();
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let size = style.font_size.to_pixels(window.rem_size());

    window
        .text_system()
        .shape_line(text, size, &[run], None)
        .width
}

pub fn tucked(radius: Pixels, window: &Window) -> Pixels {
    (radius - window.rem_size() * INSET - BORDER).max(Pixels::ZERO)
}
