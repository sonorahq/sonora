use i18n::{SharedString, t};

/// How the spectrum is drawn behind the fullscreen artwork.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualizerStyle {
    None,
    Bars,
    Wave,
    #[default]
    Both,
}

impl VisualizerStyle {
    pub const ALL: [Self; 4] = [Self::None, Self::Bars, Self::Wave, Self::Both];

    pub fn shown(self) -> bool {
        self != Self::None
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bars => "bars",
            Self::Wave => "wave",
            Self::Both => "both",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "none" => Self::None,
            "bars" => Self::Bars,
            "wave" => Self::Wave,
            _ => Self::default(),
        }
    }

    /// The localized name. Resolved at render, never stored.
    pub fn label(self) -> SharedString {
        match self {
            Self::None => t!("settings-visualizer-style-none"),
            Self::Bars => t!("settings-visualizer-style-bars"),
            Self::Wave => t!("settings-visualizer-style-wave"),
            Self::Both => t!("settings-visualizer-style-both"),
        }
    }

    pub fn bars(self) -> bool {
        matches!(self, Self::Bars | Self::Both)
    }

    pub fn wave(self) -> bool {
        matches!(self, Self::Wave | Self::Both)
    }
}

/// A frame of the spectrum as the visualizer draws it: one band set per channel.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Levels {
    /// Both channels folded together, the louder of the two per band. What the bars draw.
    pub fn mixed(&self) -> Vec<f32> {
        let mut bands = self.left.clone();
        bands.resize(bands.len().max(self.right.len()), 0.);
        for (band, right) in bands.iter_mut().zip(&self.right) {
            *band = band.max(*right);
        }
        bands
    }
}

