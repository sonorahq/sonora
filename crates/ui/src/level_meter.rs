//! A thin horizontal level meter in dBFS.

use rune::{El, el, row};

use crate::theme::Theme;

/// The levels the meter spans, in dBFS, its left edge first. Full scale sits near the right, so
/// the clip zone past it is a narrow band.
const RANGE: (f32, f32) = (-48., 6.);
/// How tall the bar is.
const THICKNESS: f32 = 4.;
/// How wide the peak tick is.
const TICK: f32 = 2.;
/// How strongly the clip zone tints the track where the level does not reach.
const CLIP_ZONE: f32 = 0.25;
const ROUND: f32 = 9999.;

/// A meter filled to a level, with the held peak marked by a tick and everything past full
/// scale in the danger colour. The fill springs to each new level.
pub struct LevelMeter {
    level: f32,
    peak: Option<f32>,
}

impl LevelMeter {
    /// A meter filled to `level`, in dBFS.
    pub fn new(level: f32) -> Self {
        Self { level, peak: None }
    }

    /// Marks `peak`, in dBFS, with a tick. A peak below the meter's range shows none.
    pub fn peak(mut self, peak: f32) -> Self {
        self.peak = Some(peak);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let full = share(0.);
        let level = share(self.level);
        let shown = level.min(full);
        let over = (level - full).max(0.);
        let mut bar = row()
            .key("level")
            .w_full()
            .h(THICKNESS)
            .radius(ROUND)
            .clip_rounded(ROUND)
            .fill(theme.muted.color())
            .child(el().h_full().w_frac(shown).fill(theme.primary.color()))
            .child(el().h_full().w_frac(full - shown))
            .child(el().h_full().w_frac(over).fill(theme.danger.color()))
            .child(
                el().h_full()
                    .grow(1.)
                    .fill(theme.danger.opacity(CLIP_ZONE).color()),
            );
        if let Some(peak) = self.peak.map(share).filter(|peak| *peak > 0.) {
            let tick = match peak > full {
                true => theme.danger,
                false => theme.foreground,
            };
            bar = bar.child(
                row()
                    .pin(Some(0.), Some(0.), Some(0.), Some(0.))
                    .child(el().h_full().w_frac(peak).mr(-TICK / 2.))
                    .child(el().h_full().w(TICK).fill(tick.color())),
            );
        }
        bar.a11y_hidden()
    }
}

/// How far along the meter `db` sits, from 0 at its left edge to 1 at its right.
fn share(db: f32) -> f32 {
    ((db - RANGE.0) / (RANGE.1 - RANGE.0)).clamp(0., 1.)
}
