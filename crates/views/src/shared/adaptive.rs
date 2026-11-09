//! The adaptive theme: the theme's tint follows the playing track's cover, and the next track's
//! cover is sampled ahead so a track change recolours on the frame it happens rather than once
//! a download and a decode have come back. Rune springs every colour, so the new palette fades
//! in on its own.

use router::Destination;
use state::QueuePosition;
use ui::{CoverPalette, Look, Theme, ThemeKind, ThemeOverrides};

use crate::Ui;

/// What the theme was last dressed from here, so an unchanged frame costs a comparison.
#[derive(Default)]
pub(crate) struct Adaptive {
    /// The cover the tint comes from, and the palette it gave, kept while the next cover is
    /// still being sampled so the theme never flashes back to no tint between tracks.
    cover: Option<String>,
    painted: CoverPalette,
    worn: Option<(Look, ThemeOverrides, bool)>,
}

/// Brings the theme's tint up to date with the playing cover, and samples the next one ahead.
/// True when the theme changed. The window calls it after resolving the theme from settings and
/// on every poll, since a cover's palette lands outside any handler.
pub(crate) fn wear(ui: &mut Ui) -> bool {
    let fullscreen = matches!(ui.nav.current(), Destination::Fullscreen);
    let store = &ui.store;
    let settings = &store.settings;
    let tinted = settings.cover_tint(fullscreen);
    let cover = tinted
        .then(|| {
            store
                .playback
                .shown_track(&store.devices)
                .and_then(|track| track.cover)
        })
        .flatten();
    let next = tinted
        .then(|| {
            store
                .shown_queue()
                .track(QueuePosition::Upcoming(0))
                .and_then(|track| track.cover)
        })
        .flatten();

    let artwork = ui.artwork.as_ref();
    if let (Some(artwork), Some(next)) = (artwork, &next) {
        // Asking samples it, so it is waiting when the track starts.
        let _ = artwork.palette(next);
    }
    let adaptive = &mut ui.dialogs.adaptive;
    match &cover {
        None => {
            adaptive.cover = None;
            adaptive.painted = CoverPalette::default();
        }
        Some(url) => {
            if let Some(palette) = artwork.and_then(|artwork| artwork.palette(url)) {
                adaptive.cover = Some(url.clone());
                adaptive.painted = palette;
            }
        }
    }

    let (look, overrides) = dress(ui, fullscreen);
    let wanted = (look, overrides, ui.system_dark);
    let adaptive = &mut ui.dialogs.adaptive;
    if adaptive.worn.as_ref() == Some(&wanted) {
        return false;
    }
    ui.theme = Theme::resolve(wanted.0, &wanted.1, wanted.2);
    adaptive.worn = Some(wanted);
    true
}

/// The look and overrides to wear. Under the ambient background fullscreen puts the plain
/// adaptive Dark theme on, since the field behind the lyrics is dark whatever the cover gave it
/// and a light theme or a theme file's own colours would lose their text on it; only the theme
/// file's radius and font size survive, so the layout does not shift on the way in.
fn dress(ui: &Ui, fullscreen: bool) -> (Look, ThemeOverrides) {
    let settings = &ui.store.settings;
    let palette = ui.dialogs.adaptive.painted;
    let base = settings.look();
    let overrides = settings.theme_overrides();
    let (kind, overrides) = match fullscreen && settings.ambient() {
        true => (ThemeKind::Dark, stripped(overrides)),
        false => (base.kind, overrides),
    };
    (
        Look {
            kind,
            tint: palette.primary,
            tint_secondary: palette.secondary,
            ..base
        },
        overrides,
    )
}

/// The overrides with every colour taken out, leaving the radius and font size.
fn stripped(overrides: ThemeOverrides) -> ThemeOverrides {
    ThemeOverrides {
        radius: overrides.radius,
        font_size: overrides.font_size,
        ..ThemeOverrides::default()
    }
}
