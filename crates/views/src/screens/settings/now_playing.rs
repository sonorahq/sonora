//! The Now Playing section of Appearance: the fullscreen player's background, its visualizer
//! and its controls.

use i18n::t;
use rune::{Cx, El, col};
use state::FullscreenControlsAutohide;
use ui::{Levels, OptionCard, Visualizer, VisualizerStyle};

use super::appearance::card_width;
use super::{Layout, Row, columns, glyph_thumb, grid, group, segmented, switch, toggle};
use crate::{Ui, act};

/// How large the cross on the Off card draws.
const GLYPH: f32 = 18.;
/// How much of a visualizer card's height its spectrum takes.
const SPECTRUM: f32 = 0.8;
/// How thick a card's border is, which the spectrum sits inside.
const EDGE: f32 = 1.;
/// The band levels a visualizer card draws, a spectrum caught at one moment.
const STILL: [f32; 16] = [
    0.42, 0.66, 0.84, 0.7, 0.5, 0.58, 0.76, 0.6, 0.44, 0.52, 0.64, 0.46, 0.34, 0.4, 0.28, 0.22,
];

/// The still spectra the visualizer cards draw, one per style, fed their frame once.
pub struct State {
    spectra: Vec<Visualizer>,
}

impl Default for State {
    fn default() -> Self {
        let still = Levels {
            left: STILL.to_vec(),
            right: STILL.iter().rev().copied().collect(),
        };
        let spectra = VisualizerStyle::ALL
            .iter()
            .map(|_| {
                let spectrum = Visualizer::new(STILL.len());
                spectrum.push(&still);
                spectrum
            })
            .collect();
        Self { spectra }
    }
}

pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let ambient = settings.ambient();
    let mut visualizer = Vec::new();
    if settings.visualizer_style().shown() {
        visualizer.push(switch(
            theme,
            "visualizer-absolute",
            t!("settings-visualizer-absolute"),
            settings.visualizer_absolute(),
            |ui, on| {
                ui.store
                    .settings
                    .set_visualizer_absolute(on, &mut ui.store.cx)
            },
        ));
    }
    vec![
        group(
            theme,
            Some(("settings-group-background", "icons/blend.svg")),
            None,
            [
                // the background carries the cover's hues itself, so fullscreen tints from the
                // artwork with the adaptive theme off as well
                switch(
                    theme,
                    "ambient",
                    t!("settings-ambient"),
                    ambient,
                    |ui, on| ui.store.settings.set_ambient(on, &mut ui.store.cx),
                ),
                Row::new(
                    "ambient-motion",
                    t!("settings-ambient-motion"),
                    toggle(
                        theme,
                        "ambient-motion",
                        t!("settings-ambient-motion"),
                        settings.ambient_motion(),
                        !ambient,
                        |ui, on| ui.store.settings.set_ambient_motion(on, &mut ui.store.cx),
                    ),
                )
                .disabled(!ambient)
                .build(theme),
            ],
        ),
        group(
            theme,
            Some(("settings-visualizer", "icons/music-2.svg")),
            Some(cards(ui, layer, layout)),
            visualizer,
        ),
        group(
            theme,
            Some(("settings-group-controls", "icons/sliders-horizontal.svg")),
            None,
            [
                Row::new(
                    "fullscreen-controls-autohide",
                    t!("settings-fullscreen-controls-autohide"),
                    segmented(
                        theme,
                        "fullscreen-controls-autohide",
                        t!("settings-fullscreen-controls-autohide"),
                        FullscreenControlsAutohide::ALL
                            .map(|choice| (choice, i18n::lookup(choice.key(), None).into_owned())),
                        settings.fullscreen_controls_autohide(),
                        |ui, choice| {
                            ui.store
                                .settings
                                .set_fullscreen_controls_autohide(choice, &mut ui.store.cx)
                        },
                    ),
                )
                .build(theme),
                switch(
                    theme,
                    "os-fullscreen",
                    t!("settings-os-fullscreen"),
                    settings.os_fullscreen(),
                    |ui, on| ui.store.settings.set_os_fullscreen(on, &mut ui.store.cx),
                ),
            ],
        ),
    ]
}

/// How the spectrum is drawn behind the fullscreen artwork, off included: one card per style,
/// each drawing a still spectrum the way that style does.
fn cards(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let chosen = ui.store.settings.visualizer_style();
    let height = theme.metrics.list_row * 1.5;
    let count = VisualizerStyle::ALL.len();
    let width = (card_width(layout, count) - EDGE * 2.).max(0.);
    // the spectrum cuts itself to the inside of the card's rounded border
    let corner = (theme.radius - EDGE).max(0.);
    let spectra = &ui.screens.settings.visualizers.spectra;
    let cards = VisualizerStyle::ALL
        .into_iter()
        .zip(spectra)
        .map(|(style, spectrum)| {
            let thumb = match style.shown() {
                false => glyph_thumb(theme, "icons/x.svg", GLYPH),
                true => col().w_full().h_full().justify_end().pb(EDGE).child(
                    spectrum
                        .view(width, height * SPECTRUM)
                        .style(style)
                        .tint(theme.foreground)
                        .behind(theme.secondary)
                        .corner_radius(corner)
                        .build(theme)
                        .ml(EDGE),
                ),
            };
            OptionCard::new(format!("visualizer-{}", style.id()))
                .thumb(thumb)
                .label(style.label())
                .selected(style == chosen)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store
                        .settings
                        .set_visualizer_style(style, &mut ui.store.cx)
                }))
                .build(theme, layer)
        });
    grid(
        "visualizer",
        columns(count, layout.room()),
        cards.collect::<Vec<_>>(),
    )
}
