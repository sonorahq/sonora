//! The Storage section of Playback: the cache of played tracks, offline mode, and what the
//! cache and the downloads take on disk.

use std::time::Duration;

use i18n::t;
use rune::{Cx, El, Tx, row, text};
use state::Job;
use ui::{Button, Scrubber, ScrubberState, Text, Theme, Weight, font};

use super::{Row, group, switch};
use crate::shared::confirm::{self, Kind};
use crate::{Ui, act};

/// The bytes in a gigabyte, the unit the cache limit is set in.
const GIGABYTE: u64 = 1 << 30;
const MEGABYTE: u64 = 1 << 20;
/// How often the section measures the cache and the downloads again while it shows, since
/// tracks are kept and evicted as they play.
const POLL: Duration = Duration::from_secs(2);
/// The space between a size and its button.
const GAP: f32 = 12.;
/// How wide the cache slider and its label stand, as parts of a cover.
const SLIDER: f32 = 1.;
const LABEL: f32 = 0.7;
/// How strongly the empty part of the cache slider shows.
const EMPTY: f32 = 0.3;

/// What the section keeps between frames.
#[derive(Default)]
pub struct State {
    cache: ScrubberState,
    /// The cache size a drag of its slider points at, until it lets go.
    pending: Option<u32>,
    /// Measures the cache and the downloads every so often while the section shows.
    poll: Option<Job>,
}

/// The section: the cache settings show only for a provider whose played tracks are kept, while
/// the space each takes always shows, since files stay on disk across providers.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> Vec<El> {
    let theme = &ui.theme;
    let mut cache = Vec::new();
    if ui.store.session.capabilities().keeps {
        cache.push(switch(
            theme,
            "keep-tracks",
            t!("settings-keep-tracks"),
            ui.store.playback.keep_tracks(),
            |ui, on| ui.store.set_keep_tracks(on),
        ));
        if ui.store.playback.keep_tracks() {
            cache.push(cache_size(ui));
        }
    }
    cache.push(cache_used(ui));
    let groups = vec![
        group(
            theme,
            Some(("settings-group-cache", "icons/rotate-ccw-clock.svg")),
            None,
            cache,
        ),
        group(
            theme,
            Some(("settings-group-downloads", "icons/download.svg")),
            None,
            [
                switch(
                    theme,
                    "offline-mode",
                    t!("settings-offline-mode"),
                    ui.store.network.offline(),
                    |ui, on| ui.store.set_offline(on),
                ),
                downloads_used(ui),
            ],
        ),
    ];
    // measuring runs only while the section is on screen
    groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| match index {
            0 => group.on_visible(|ui: &mut Ui, visible: bool, _: &mut Tx| watch(ui, visible)),
            _ => group,
        })
        .collect()
}

/// Starts measuring the cache and the downloads every [`POLL`] while the section shows, and
/// stops when it leaves.
fn watch(ui: &mut Ui, visible: bool) {
    let storage = &mut ui.screens.settings.storage;
    match visible {
        false => storage.poll = None,
        true if storage.poll.is_none() => {
            ui.store.playback.measure_storage(&mut ui.store.cx);
            let io = ui.store.cx.io.clone();
            storage.poll = Some(ui.store.cx.spawn(|link| async move {
                loop {
                    // the runtime this crate reaches offers no timer of its own, so a blocking
                    // thread waits out the interval
                    if io
                        .spawn_blocking(|| std::thread::sleep(POLL))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    link.post(|store| store.playback.measure_storage(&mut store.cx));
                }
            }));
        }
        true => {}
    }
}

/// How many gigabytes the cache of played tracks may take, as a slider with the size before
/// it. The label follows a drag at once, and the limit changes when the drag lets go.
fn cache_size(ui: &Ui) -> El {
    let theme = &ui.theme;
    let storage = &ui.screens.settings.storage;
    let (low, high) = state::CACHE_GB_RANGE;
    let current = storage
        .pending
        .unwrap_or_else(|| ui.store.playback.cache_gb());
    let fraction = current.saturating_sub(low) as f32 / (high - low) as f32;
    let said = t!("settings-cache-size-value", count = current).into_owned();
    let slider = Scrubber::new("settings-cache-slider", &storage.cache, fraction)
        .name(t!("settings-cache-size"))
        .value(said.clone())
        .colors(
            theme.progress_bar,
            theme.muted_foreground.opacity(EMPTY),
            theme.foreground,
        )
        .on_move(move |ui: &mut Ui, fraction: f32, _: &mut Tx| {
            let span = (high - low) as f32;
            ui.screens.settings.storage.pending =
                Some(low + (fraction.clamp(0., 1.) * span).round() as u32);
        })
        .on_release(|ui: &mut Ui, _: &mut Tx| {
            if let Some(gb) = ui.screens.settings.storage.pending.take() {
                ui.store.set_cache_gb(gb);
                ui.settle();
            }
        })
        .build(theme);
    let control = row()
        .key("cache-size")
        .center_y()
        .gap(GAP)
        .child(
            spoken(theme, "settings-cache-size-value", said, theme.foreground)
                .w(theme.metrics.cover * LABEL)
                .align(rune::Align::End),
        )
        .child(
            row()
                .key("cache-track")
                .w(theme.metrics.cover * SLIDER)
                .child(slider.w_full()),
        );
    Row::new("cache-size", t!("settings-cache-size"), control).build(theme)
}

/// What the cached tracks take on disk, with a button that removes them all. Downloads stay.
fn cache_used(ui: &Ui) -> El {
    let theme = &ui.theme;
    let used = ui.store.playback.usage().map(|(cache, _)| cache);
    let control = row()
        .key("cache-used")
        .center_y()
        .gap(GAP)
        .child(spoken(
            theme,
            "cache-used",
            size(used),
            theme.muted_foreground,
        ))
        .child(
            Button::new("clear-cache")
                .label(t!("settings-cache-clear"))
                .small()
                .outline()
                .disabled(used.is_none_or(|used| used == 0))
                .on_press(act(|ui: &mut Ui| {
                    ui.store.playback.clear_cache(&mut ui.store.cx)
                }))
                .build(theme),
        );
    Row::new("cache-used", t!("settings-cache-used"), control).build(theme)
}

/// What the downloads take on disk, with a button that removes them all once confirmed.
fn downloads_used(ui: &Ui) -> El {
    let theme = &ui.theme;
    let used = ui.store.playback.usage().map(|(_, downloads)| downloads);
    let control = row()
        .key("downloads-used")
        .center_y()
        .gap(GAP)
        .child(spoken(
            theme,
            "downloads-used",
            size(used),
            theme.muted_foreground,
        ))
        .child(
            Button::new("remove-downloads")
                .label(t!("settings-downloads-remove"))
                .small()
                .outline()
                .disabled(used.is_none_or(|used| used == 0))
                .on_press(|ui: &mut Ui| {
                    confirm::ask(ui, Kind::Downloads, |ui: &mut Ui| {
                        ui.store.remove_all_downloads();
                        ui.settle();
                    })
                })
                .build(theme),
        );
    Row::new("downloads-used", t!("settings-downloads-used"), control).build(theme)
}

/// A line of static text screen readers reach under `key`, in `ink`.
fn spoken(theme: &Theme, key: &str, said: String, ink: ui::Hsla) -> El {
    text(said.clone())
        .key(key.to_owned())
        .role("text")
        .label(said)
        .font(font(theme, Text::Body, Weight::NORMAL))
        .color(ink.color())
}

/// A size in bytes as a short label: gigabytes to one decimal from a gigabyte up, whole
/// megabytes below. Nothing measured yet reads as measuring.
fn size(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return t!("settings-size-measuring").into_owned();
    };
    let mut args = i18n::FluentArgs::new();
    match bytes >= GIGABYTE {
        true => {
            args.set("size", format!("{:.1}", bytes as f64 / GIGABYTE as f64));
            i18n::lookup("settings-size-gb", Some(&args)).into_owned()
        }
        false => {
            args.set("size", (bytes / MEGABYTE) as i64);
            i18n::lookup("settings-size-mb", Some(&args)).into_owned()
        }
    }
}
