//! The Lyrics page: whether lyrics show, how they look, and where they come from; and the
//! sources page, the one list of every lyrics source in the order ties are settled, which is
//! reordered by dragging.

use i18n::t;
use music::WritingSystem;
use router::{Destination, SettingsPage};
use rune::{Cursor, Cx, Drag, El, Phase, Transition, Tx, col, row, text};
use state::LYRICS_SOURCES;
use ui::{
    Carry, Edge, Ghost, MAX_LYRICS_SCALE, MIN_LYRICS_SCALE, MenuItem, Picker, TABULAR, Text,
    Theme, Weight, drop_marker, font, grip,
};

use super::{Layout, Row, drill, group, switch, toggle, whole};
use crate::{Ui, act};

/// How far one press of a size stepper moves the scale.
const SIZE_STEP: f32 = 0.1;

/// What the sources page keeps while a source is dragged: which one, where it would land, and
/// where its chip is.
#[derive(Default)]
pub struct State {
    /// The source being dragged, by provider name.
    dragging: Option<String>,
    /// Where the dragged source would land, as a gap between the enabled sources. `None` while
    /// the drop would leave the order as it is.
    gap: Option<usize>,
    carry: Carry,
}

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.settings.lyrics
}

/// The Lyrics page: the preview, the switch, and while lyrics are on how they look and the way
/// into their sources.
pub(super) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut blocks = vec![super::preview::lyrics::view(ui, cx, layout)];
    blocks.push(group(
        theme,
        None,
        None,
        [switch(
            theme,
            "lyrics",
            t!("settings-lyrics"),
            settings.lyrics_enabled(),
            |ui, on| ui.store.settings.set_lyrics_enabled(on, &mut ui.store.cx),
        )],
    ));
    if !settings.lyrics_enabled() {
        return blocks;
    }
    blocks.push(
        group(
            theme,
            Some(("settings-group-display", "icons/mic-vocal.svg")),
            None,
            [
                switch(
                    theme,
                    "karaoke-lyrics",
                    t!("settings-karaoke-lyrics"),
                    settings.karaoke_lyrics(),
                    |ui, on| ui.store.settings.set_karaoke_lyrics(on, &mut ui.store.cx),
                ),
                switch(
                    theme,
                    "blur-lyrics",
                    t!("settings-blur-lyrics"),
                    settings.blur_lyrics(),
                    |ui, on| ui.store.settings.set_blur_lyrics(on, &mut ui.store.cx),
                ),
                romanized_row(ui, layer),
                size_row(
                    theme,
                    "panel-lyrics-size",
                    "settings-panel-lyrics-size",
                    settings.panel_lyrics_scale(),
                    |ui, scale| {
                        ui.store
                            .settings
                            .set_panel_lyrics_scale(scale, &mut ui.store.cx)
                    },
                ),
                size_row(
                    theme,
                    "fullscreen-lyrics-size",
                    "settings-fullscreen-lyrics-size",
                    settings.fullscreen_lyrics_scale(),
                    |ui, scale| {
                        ui.store
                            .settings
                            .set_fullscreen_lyrics_scale(scale, &mut ui.store.cx)
                    },
                ),
            ],
        )
        .transition(Transition::offset(0., 8.).combined(Transition::opacity())),
    );
    blocks.push(
        group(
            theme,
            Some(("settings-group-sources", "icons/list-music.svg")),
            None,
            [
                sources_row(ui),
                switch(
                    theme,
                    "lyrics-for-local-files",
                    t!("settings-lyrics-for-local-files"),
                    settings.lyrics_for_local_files(),
                    |ui, on| {
                        ui.store
                            .settings
                            .set_lyrics_for_local_files(on, &mut ui.store.cx)
                    },
                ),
            ],
        )
        .transition(Transition::offset(0., 8.).combined(Transition::opacity())),
    );
    blocks
}

/// The sources page: every lyrics source in one list.
pub(super) fn sources(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, _layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let state = &ui.screens.settings.lyrics;
    if let Some(dragging) = &state.dragging {
        layer.push(
            Ghost::new(source_name(dragging))
                .badge("icons/arrow-up-down.svg")
                .build(ui.artwork(), &state.carry, theme),
        );
    }
    vec![group(
        theme,
        Some(("settings-group-lyrics-order", "icons/arrow-up-down.svg")),
        None,
        [priority(ui)],
    )]
}

/// The row that opens the sources page, saying how many sources are on.
fn sources_row(ui: &Ui) -> El {
    let settings = &ui.store.settings;
    let count = LYRICS_SOURCES
        .iter()
        .filter(|provider| settings.lyrics_provider_enabled(provider))
        .count();
    whole(
        drill(
            "lyrics-sources",
            t!("settings-lyrics-sources"),
            Some(t!("settings-lyrics-providers-selected", count = count).into_owned()),
        )
        .on_press(act(|ui: &mut Ui| {
            ui.go(Destination::Settings(SettingsPage::LyricsSources))
        })),
        &ui.theme,
    )
}

/// A lyrics text size, stepped a tenth at a time between its limits, with the size between the
/// minus and the plus.
fn size_row(
    theme: &Theme,
    slug: &'static str,
    title: &'static str,
    scale: f32,
    apply: fn(&mut Ui, f32),
) -> El {
    let step = |suffix: &str, glyph: &str, name: &str, delta: f32| {
        let wanted = ((scale + delta) * 10.).round() / 10.;
        let wanted = wanted.clamp(MIN_LYRICS_SCALE, MAX_LYRICS_SCALE);
        ui::Button::new(format!("{slug}-{suffix}"))
            .label(glyph)
            .name(i18n::lookup(name, None))
            .small()
            .outline()
            .disabled((wanted - scale).abs() < f32::EPSILON)
            .on_press(act(move |ui: &mut Ui| apply(ui, wanted)))
            .build(theme)
    };
    let value = t!(
        "settings-lyrics-size-value",
        size = (scale * 100.).round() as i64
    );
    let control = row()
        .center_y()
        .gap(theme.font_size * 0.5)
        .child(step("smaller", "\u{2212}", "settings-font-smaller", -SIZE_STEP))
        .child(
            ui::spoken(format!("{slug}-value"), value, theme)
                .font(font(theme, Text::Body, Weight::NORMAL).features(TABULAR)),
        )
        .child(step("larger", "+", "settings-font-larger", SIZE_STEP));
    Row::new(slug, i18n::lookup(title, None), control).build(theme)
}

/// Romanized lyrics: the switch, and while it is on the picker of the writing systems it
/// romanizes, which stays open while several are ticked.
fn romanized_row(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let on = settings.romanized_lyrics();
    let scripts = settings.romanization_scripts();
    let mut control = row().center_y().gap(theme.font_size * 0.5);
    if on {
        let picker = Picker::new(
            "romanization-scripts",
            &ui.screens.settings.popovers,
            |ui: &mut Ui| &mut ui.screens.settings.popovers,
            t!("settings-romanization-writing-systems"),
        )
        .name(t!("settings-romanization-writing-systems"))
        .width(Picker::<Ui>::REGULAR)
        .sticky()
        .items(WritingSystem::ALL.map(|writing_system| {
            let (id, label) = romanization_script_copy(writing_system);
            let selected = scripts.contains(writing_system);
            MenuItem::new(id, i18n::lookup(label, None))
                .selected(selected)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.settings.set_romanization_script(
                        writing_system,
                        !selected,
                        &mut ui.store.cx,
                    )
                }))
        }))
        .build(theme, layer);
        control = control.child(picker.transition(Transition::scale(0.9).combined(Transition::opacity())));
    }
    control = control.child(toggle(
        theme,
        "romanized-lyrics",
        t!("settings-romanized-lyrics"),
        on,
        false,
        |ui, on| ui.store.settings.set_romanized_lyrics(on, &mut ui.store.cx),
    ));
    Row::new("romanized-lyrics", t!("settings-romanized-lyrics"), control).build(theme)
}

/// Every lyrics source in one list. The ones that are on come first, in the order ties between
/// them are settled, and each can be dragged anywhere among them. The ones that are off follow.
/// Every source carries its own switch. Turning one on or off slides it to its new place.
fn priority(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let order = settings.enabled_lyrics_provider_priority();
    let off = LYRICS_SOURCES
        .into_iter()
        .filter(|provider| !settings.lyrics_provider_enabled(provider))
        .collect::<Vec<_>>();
    let count = order.len();
    let total = count + off.len();
    let mut entries: Vec<El> = order
        .into_iter()
        .enumerate()
        .map(|(index, provider)| source_on(ui, index, provider, count, total))
        .collect();
    entries.extend(
        off.into_iter()
            .enumerate()
            .map(|(index, provider)| source_off(theme, (count + index, total), provider)),
    );
    col()
        .key("settings-row-lyrics-priority")
        .role("list")
        .label(t!("settings-group-lyrics-order"))
        .w_full()
        .children(entries)
}

/// The switch that turns one source on or off.
fn source_switch(theme: &Theme, provider: &str, on: bool) -> El {
    let named = provider.to_owned();
    ui::Switch::new(format!("lyrics-source-on-{provider}"), on)
        .name(source_name(provider))
        .on_press(act(move |ui: &mut Ui| {
            ui.store
                .settings
                .set_lyrics_provider(&named, !on, &mut ui.store.cx)
        }))
        .build(theme)
}

/// A source that is off, placed by its index and the list's length: no place in the order,
/// its name muted, and its switch.
fn source_off(theme: &Theme, (index, total): (usize, usize), provider: &str) -> El {
    row()
        .key(format!("lyrics-source-{provider}"))
        .role("listitem")
        .label(source_name(provider))
        .position_in_set(index + 1, total)
        .w_full()
        .center_y()
        .gap(theme.font_size * 0.75)
        .h(theme.metrics.list_row)
        .motion(ui::Springs::BASE)
        .child(
            text(source_name(provider))
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        )
        .child(source_switch(theme, provider, false))
}

/// One source that is on: the grip it is dragged by, its muted place in the order, its name and
/// its switch. The whole row drags, the grip only shows that it does. `count` sources are on out
/// of the `total` the list shows. Rows spring to their new places when a drop reorders them.
fn source_on(ui: &Ui, index: usize, provider: String, count: usize, total: usize) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings.lyrics;
    let name = source_name(&provider);
    let edge = match state.gap {
        Some(gap) if gap == index => Some(Edge::Above),
        Some(gap) if gap == count && index + 1 == count => Some(Edge::Below),
        _ => None,
    };
    let lifted = state.dragging.as_deref() == Some(provider.as_str());
    let dragged = provider.clone();
    let mut entry = row()
        .key(format!("lyrics-source-{provider}"))
        .role("listitem")
        .label(name.clone())
        .position_in_set(index + 1, total)
        .w_full()
        .center_y()
        .gap(theme.font_size * 0.75)
        .h(theme.metrics.list_row)
        .cursor(Cursor::Grab)
        .motion(ui::Springs::BASE)
        .opacity(match lifted {
            true => 0.5,
            false => 1.,
        })
        .on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            drag_source(ui, drag, tx, &dragged, index, count)
        })
        .child(grip(theme))
        .child(
            text((index + 1).to_string())
                .font(font(theme, Text::Body, Weight::NORMAL).features(TABULAR))
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        )
        .child(
            text(name)
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.color())
                .truncate()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        );
    if LYRICS_SOURCES.contains(&provider.as_str()) {
        entry = entry.child(source_switch(theme, &provider, true));
    }
    if let Some(edge) = edge {
        entry = entry.child(drop_marker(edge, theme));
    }
    entry
}

/// Follows a drag of the source at `index` among the `count` that are on: the chip follows the
/// pointer, the marker moves to the gap under it, and the drop places the source there. A move
/// that changes nothing but the chip's place runs no view.
fn drag_source(ui: &mut Ui, drag: &mut Drag, tx: &mut Tx, provider: &str, index: usize, count: usize) {
    match drag.phase {
        Phase::Down => {
            let state = st(ui);
            state.carry.follow(drag);
            state.gap = None;
            tx.live_only();
        }
        Phase::Move => {
            let state = st(ui);
            state.carry.follow(drag);
            let started = state.dragging.is_some();
            if !started {
                state.dragging = Some(provider.to_owned());
            }
            // rows touch and share one height, so the pointer's place in the list is this row's
            // top plus how far into it the pointer is
            let height = drag.h.max(1.);
            let at = index as f32 * height + drag.y;
            let gap = ((at / height).round().max(0.) as usize).min(count);
            let gap = Some(gap).filter(|gap| *gap != index && *gap != index + 1);
            if started && state.gap == gap {
                tx.live_only();
            }
            state.gap = gap;
        }
        Phase::Up => {
            let state = st(ui);
            let gap = state.gap.take();
            state.dragging = None;
            if drag.cancelled() {
                return;
            }
            if let Some(gap) = gap {
                ui.store
                    .settings
                    .place_lyrics_provider(provider, gap, &mut ui.store.cx);
                ui.settle();
            }
        }
    }
}

/// The id and the Fluent key a writing system goes by in the romanization picker.
pub(super) fn romanization_script_copy(
    writing_system: WritingSystem,
) -> (&'static str, &'static str) {
    match writing_system {
        WritingSystem::Japanese => ("romanization-japanese", "settings-romanization-japanese"),
        WritingSystem::Chinese => ("romanization-chinese", "settings-romanization-chinese"),
        WritingSystem::Korean => ("romanization-korean", "settings-romanization-korean"),
        WritingSystem::Cyrillic => ("romanization-cyrillic", "settings-romanization-cyrillic"),
        WritingSystem::Greek => ("romanization-greek", "settings-romanization-greek"),
        WritingSystem::Arabic => ("romanization-arabic", "settings-romanization-arabic"),
        WritingSystem::Other => ("romanization-other", "settings-romanization-other"),
    }
}

/// The name a lyrics source goes by in the settings, falling back to the provider's own name.
pub(super) fn source_name(provider: &str) -> String {
    let key = match provider {
        music::lyrics::LOCAL => "settings-lyrics-provider-local",
        "Subsonic" => "settings-lyrics-provider-subsonic",
        "Spotify" => "settings-lyrics-provider-spotify",
        "YouTube Music" => "settings-lyrics-provider-youtube",
        "Apple Music" => "settings-lyrics-provider-apple-music",
        "Musixmatch" => "settings-lyrics-provider-musixmatch",
        "LrcLib" => "settings-lyrics-provider-lrclib",
        "Kugou" => "settings-lyrics-provider-kugou",
        "NetEase" => "settings-lyrics-provider-netease",
        _ => return provider.to_owned(),
    };
    i18n::lookup(key, None).into_owned()
}
