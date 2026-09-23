use gpui::SharedString;
use i18n::t;

/// How fullscreen stages the cover: as itself, haloed in drifting particles,
/// riding a turning record as its label, or slid halfway out of its sleeve
/// with the record peeking out behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StageStyle {
    /// The cover as it is, nothing staged around it.
    #[default]
    Cover,
    /// The cover haloed in particles, with a spectrum ring around it.
    Starry,
    /// The starry stage with the cover dressed as a turning record's label.
    Vinyl,
    /// The cover slid out of its sleeve, the record turning behind it.
    Sleeve,
}

impl StageStyle {
    pub const ALL: [Self; 4] = [Self::Cover, Self::Starry, Self::Vinyl, Self::Sleeve];

    /// Whether fullscreen stages anything at all.
    pub fn shown(self) -> bool {
        self != Self::Cover
    }

    /// Whether the style carries the halo, the particle field and the spectrum
    /// ring — everything but the sleeve, which stages the cover on bare paint.
    pub fn fielded(self) -> bool {
        matches!(self, Self::Starry | Self::Vinyl)
    }

    /// Whether the cover rides a turning record as its label.
    pub fn turned(self) -> bool {
        self == Self::Vinyl
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Starry => "starry",
            Self::Vinyl => "vinyl",
            Self::Sleeve => "sleeve",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "starry" => Self::Starry,
            "vinyl" => Self::Vinyl,
            "sleeve" => Self::Sleeve,
            _ => Self::default(),
        }
    }

    /// The localized name. Resolved at render, never stored.
    pub fn label(self) -> SharedString {
        match self {
            Self::Cover => t!("settings-stage-style-cover"),
            Self::Starry => t!("settings-stage-style-starry"),
            Self::Vinyl => t!("settings-stage-style-vinyl"),
            Self::Sleeve => t!("settings-stage-style-sleeve"),
        }
    }
}
