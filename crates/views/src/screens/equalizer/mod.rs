//! The equalizer's own page, a drill-in of Settings > Sound. Simple mode shows the sound picker,
//! undo, the response graph over the spectrum, the output meter and the bass and treble sliders;
//! advanced mode trades the sliders for the preamp and a group each for the filters, the device
//! rules and Equalizer APO text in and out.

mod graph;
mod meter;
mod rows;
mod rules;
mod transfer;

use std::time::Duration;

use i18n::t;
use music::equalizer::{Curve, PREAMP_RANGE, Preamp, Preset, TONE_RANGE, Tone};
use router::Destination;
use rune::{Cx, El, TextEdit, Tx, col, el, row};
use state::{RuleDevice, Sound, Store, equalizer_profile_label};
use ui::{Button, Input, MenuItem, Modal, Picker, Popovers, Room, Scrubber, ScrubberState, Switch};

use crate::screens::settings::{self, GROUPS, Layout, Row, frame, segmented};
use crate::shared::confirm::{self, Kind};
use crate::shared::device_menu::DeviceMenu;
use crate::shared::equalizer::{preset_key, sound_picker};
use crate::shared::page::rems;
use crate::{Ui, act};

use graph::{Graph, Spectrum};
use meter::Meter;
use rows::{Field, Fields, figure, figure_cell};

const SOUND: &str = "equalizer-sound";
const PREAMP: &str = "equalizer-preamp";
/// How wide a number field is, in cover sizes.
const FIELD: f32 = 0.6;
/// How wide the name dialog is, in cover sizes.
const DIALOG: f32 = 2.8;
/// How wide the sound picker and its menu are in a wide room, in cover sizes. A narrower room
/// lets the picker fill its row instead.
const SOUND_WIDTH: f32 = 2.2;
/// The icons before the sound group's heading and the filter group's.
const SOUND_ICON: &str = "icons/sliders-horizontal.svg";
const FILTERS_ICON: &str = "icons/funnel.svg";
/// How wide the bass and treble sliders are, and the gain beside each, in cover sizes.
const TONE_SLIDER: f32 = 1.6;
const TONE_VALUE: f32 = 0.6;
/// The step the bass and treble move in, and how near 0 dB a drag snaps to it.
const TONE_STEP: f64 = 0.5;
const TONE_REST: f64 = 0.75;
/// How often the spectrum and the meter are looked at while they move or the music plays, about
/// the rate the signal's watch reads them at, and while everything is still.
const TICK: Duration = Duration::from_millis(16);
const IDLE: Duration = Duration::from_millis(120);
/// The space between a heading and the action at the end of its line.
const HEADING_GAP: f32 = 16.;

/// This screen's own view state.
pub struct State {
    /// The pickers of the page; one is open at a time.
    popovers: Popovers,
    preamp: TextEdit,
    rows: Vec<Fields>,
    /// The number field that had focus last frame, which is committed once it loses it.
    focused: Option<Field>,
    /// The bass and treble sliders Simple mode shows under the graph.
    bass: ScrubberState,
    treble: ScrubberState,
    name: TextEdit,
    /// What the name dialog is open for, while it is.
    naming: Option<Naming>,
    /// The device and the sound chosen in the row that adds a device rule, until it is added.
    rule_device: Option<RuleDevice>,
    rule_sound: Option<Sound>,
    /// The add row's device menu, the same one the output picker opens.
    device_menu: DeviceMenu,
    /// The curve the fields were last filled from, so they are only rewritten when it changes.
    shown: Option<Curve>,
    graph: Graph,
    /// The spectrum under the graph and the level meter under it, drawn from the poll.
    spectrum: Spectrum,
    meter: Meter,
    /// The spectrum's, the output meter's and the signal path's reads, held while the page is
    /// the one shown and dropped with it.
    watch: Option<state::Job>,
    /// The file being read in or written out.
    file: Option<state::Job>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            popovers: Popovers::default(),
            preamp: TextEdit::default(),
            rows: Fields::rows(),
            focused: None,
            bass: ScrubberState::new(),
            treble: ScrubberState::new(),
            name: TextEdit::default(),
            naming: None,
            rule_device: None,
            rule_sound: None,
            device_menu: DeviceMenu::default(),
            shown: None,
            graph: Graph::default(),
            spectrum: Spectrum::default(),
            meter: Meter::default(),
            watch: None,
            file: None,
        }
    }
}

/// What the name dialog is open for.
#[derive(Clone)]
enum Naming {
    /// Saving the playing sound as a new profile.
    Save,
    /// Renaming the profile at this index, by its stored name then, so a profile list that
    /// changed meanwhile renames nothing.
    Rename(usize, String),
}

/// One of the two shelves of the tone.
#[derive(Clone, Copy)]
enum Shelf {
    Bass,
    Treble,
}

impl Shelf {
    fn gain(self, tone: Tone) -> f64 {
        match self {
            Self::Bass => tone.bass(),
            Self::Treble => tone.treble(),
        }
    }

    /// `tone` with this shelf at `gain` and the other as it was.
    fn with(self, tone: Tone, gain: f64) -> Tone {
        match self {
            Self::Bass => Tone::new(gain, tone.treble()),
            Self::Treble => Tone::new(tone.bass(), gain),
        }
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let layout = Layout::of(ui, cx);
    let room = layout.room();
    let advanced = ui.store.settings.equalizer_advanced();
    let still = ui
        .store
        .settings
        .stillness()
        .still()
        .unwrap_or_else(|| cx.system_reduced_motion());

    let below = match advanced {
        true => preamp_row(ui, layer),
        false => tone_rows(ui),
    };
    let sound = col()
        .key("equalizer-sound-group")
        .w_full()
        .child(controls(ui, layer, room, advanced))
        .child(graph::view(ui, layout.width, room, cx.env().scale, still))
        .child(meter::view(ui))
        .child(below);
    let mut body = col()
        .key("equalizer-body")
        .w_full()
        .gap(GROUPS)
        .child(sound);
    if advanced {
        body = body
            .child(rows::filters(ui, layer, room.fits(Room::Roomy)).transition(fade()))
            .child(rules::view(ui, layer).transition(fade()))
            .child(transfer::view(ui).transition(fade()));
    }
    name_dialog(ui, layer);

    el().key("equalizer-screen").w_full().h_full().child(frame(
        ui,
        cx,
        layer,
        &Destination::Equalizer,
        None,
        "equalizer-page".to_owned(),
        body,
    ))
}

/// How a group advanced mode brings in or takes away comes and goes.
fn fade() -> rune::Transition {
    rune::Transition::opacity().combined(rune::Transition::offset(0., 8.))
}

/// Reads the output meter, the signal path and the spectrum while the page is the one shown, and
/// the outputs the device rules offer once it comes into view.
fn watch(ui: &mut Ui, shown: bool) {
    let state = &mut ui.screens.equalizer;
    if shown == state.watch.is_some() {
        return;
    }
    state.watch = None;
    if shown {
        state.spectrum.reset();
        state.meter.reset();
        let job = ui.store.watch_signal();
        ui.screens.equalizer.watch = Some(job);
        ui.store.outputs.refresh(&mut ui.store.cx);
    }
    ui.settle();
}

/// Brings the page in step with the store before a frame: the fields and the graph's nodes with
/// the playing curve, a field that lost focus committed, and the spectrum and the meter drawn
/// from their latest reads. Returns whether anything the view reads changed.
pub(crate) fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    if !matches!(ui.nav.current(), Destination::Equalizer) {
        let state = &mut ui.screens.equalizer;
        if state.watch.take().is_some() {
            state.popovers.close();
        }
        return false;
    }
    let mut changed = false;
    if ui.screens.equalizer.watch.is_none() {
        watch(ui, true);
        changed = true;
    }
    changed |= rows::show_curve(ui);
    changed |= graph::follow(ui);
    changed |= rows::blurred(ui);
    if ui.screens.equalizer.watch.is_some() {
        let fresh = ui.store.signal.take_fresh();
        let read = fresh.then(|| ui.store.signal.levels());
        let level = ui.store.equalizer.output_level();
        let state = &mut ui.screens.equalizer;
        let moving = state.spectrum.feed(read);
        state.meter.feed(level);
        let soon = fresh || moving || ui.store.playback.wants_playing();
        tx.wake_after(match soon {
            true => TICK,
            false => IDLE,
        });
    }
    changed
}

/// Plays `curve`, an edit made on this page.
pub(super) fn apply(ui: &mut Ui, curve: Curve) {
    graph::apply(ui, curve);
}

/// A group's heading as every settings page draws it, its icon before an uppercase eyebrow, with
/// `action` at the end of its line when the group has one.
pub(super) fn heading(theme: &ui::Theme, key: &str, icon: &str, action: Option<El>) -> El {
    row()
        .key(format!("equalizer-heading-{key}"))
        .w_full()
        .center_y()
        .justify_between()
        .gap(HEADING_GAP)
        .mb(theme.metrics.pad)
        .child(settings::heading(theme, key, Some(icon)).mb(0.))
        .children(action)
}

/// The top of the sound group: its heading with the Simple and Advanced switch, then the one
/// picker for every preset and profile with the on switch, undo and redo beside it. Advanced
/// mode adds what can be done to the playing profile.
fn controls(ui: &Ui, layer: &mut Vec<El>, room: Room, advanced: bool) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.equalizer;
    let settings = &ui.store.settings;
    let equalizer = &ui.store.equalizer;
    let on = equalizer.enabled();
    let only = settings.equalizer_profiles().count() < 2;
    // the unnamed profile is where presets load, so it keeps no name and is never removed
    let built_in = settings.equalizer_scratch() == Some(settings.equalizer_profile());
    let (undo, redo) = equalizer.history();
    let width = theme.metrics.cover * SOUND_WIDTH;

    let picker = sound_picker(ui, SOUND, &state.popovers, |ui: &mut Ui| {
        &mut ui.screens.equalizer.popovers
    })
    .large()
    .left()
    .width(width)
    .build(theme, layer);
    let picker = match room {
        // the trigger keeps its own width inside the picker's box, so a wide room only caps it
        Room::Wide | Room::Vast => picker.max_w(width),
        Room::Roomy => picker.grow(1.).shrink().min_w(0.),
        Room::Tight | Room::Snug => match advanced {
            true => picker.w_full(),
            false => picker.grow(1.).shrink().min_w(0.),
        },
    };
    let action = |key: &str, icon: &str, tip: &str| {
        Button::new(key)
            .ghost()
            .icon(icon)
            .tooltip(i18n::lookup(tip, None))
    };
    let history = row()
        .key("equalizer-history")
        .center_y()
        .gap(rems(theme, 0.25))
        .child(
            action("equalizer-undo", "icons/undo-2.svg", "equalizer-undo")
                .disabled(!undo)
                .on_press(act(|ui: &mut Ui| {
                    let Store {
                        equalizer,
                        settings,
                        cx,
                        ..
                    } = &mut ui.store;
                    equalizer.undo(settings, cx);
                }))
                .build(theme),
        )
        .child(
            action("equalizer-redo", "icons/redo-2.svg", "equalizer-redo")
                .disabled(!redo)
                .on_press(act(|ui: &mut Ui| {
                    let Store {
                        equalizer,
                        settings,
                        cx,
                        ..
                    } = &mut ui.store;
                    equalizer.redo(settings, cx);
                }))
                .build(theme),
        );
    let switch = Switch::new("equalizer-enabled", on)
        .name(t!("settings-equalizer"))
        .on_press(act(move |ui: &mut Ui| {
            let Store {
                equalizer,
                settings,
                cx,
                ..
            } = &mut ui.store;
            equalizer.set_enabled(!on, settings, cx);
        }))
        .build(theme);
    let mut line = row()
        .key("equalizer-controls")
        .w_full()
        .flex_wrap()
        .center_y()
        .gaps(rems(theme, 0.5), rems(theme, 0.75))
        .child(picker)
        .child(switch)
        .child(history);
    if advanced {
        line = line
            .child(el().key("equalizer-controls-gap").grow(1.))
            .child(
                row()
                    .key("equalizer-manage")
                    .center_y()
                    .gap(rems(theme, 0.25))
                    .transition(fade())
                    .child(
                        action("equalizer-save-as", "icons/plus.svg", "equalizer-save-as")
                            .on_press(act(open_save))
                            .build(theme),
                    )
                    .child(
                        action("equalizer-rename", "icons/pencil.svg", "equalizer-rename")
                            .disabled(built_in)
                            .on_press(act(open_rename))
                            .build(theme),
                    )
                    .child(
                        action("equalizer-delete", "icons/trash-2.svg", "equalizer-delete")
                            .disabled(only || built_in)
                            .on_press(act(delete))
                            .build(theme),
                    ),
            );
    }
    let mode = segmented(
        theme,
        "equalizer-mode",
        t!("equalizer-mode"),
        [
            (false, t!("equalizer-simple").into_owned()),
            (true, t!("equalizer-advanced").into_owned()),
        ],
        advanced,
        |ui: &mut Ui, choice: bool| {
            let store = &mut ui.store;
            store.settings.set_equalizer_advanced(choice, &mut store.cx);
        },
    );

    col()
        .key("equalizer-controls-block")
        .w_full()
        .child(heading(theme, "equalizer-sound", SOUND_ICON, Some(mode)))
        .child(line)
}

/// The bass and treble rows Simple mode shows under the graph, which tilt whatever sound plays
/// without editing it.
fn tone_rows(ui: &Ui) -> El {
    col()
        .key("equalizer-tone")
        .w_full()
        .mt(rems(&ui.theme, 0.5))
        .transition(fade())
        .child(tone_row(ui, Shelf::Bass))
        .child(tone_row(ui, Shelf::Treble))
}

/// One shelf's row: its name, a slider from -12 to +12 dB, and the gain it is at. Pressing the
/// gain puts the shelf back to 0 dB.
fn tone_row(ui: &Ui, shelf: Shelf) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.equalizer;
    let tone = ui.store.equalizer.tone();
    let gain = shelf.gain(tone);
    let (low, high) = (*TONE_RANGE.start(), *TONE_RANGE.end());
    let (slug, title, scrubber) = match shelf {
        Shelf::Bass => ("equalizer-bass", t!("equalizer-bass"), &state.bass),
        Shelf::Treble => ("equalizer-treble", t!("equalizer-treble"), &state.treble),
    };
    let signed = match gain > 0. {
        true => format!("+{}", figure(gain, 1)),
        false => figure(gain, 1),
    };
    let value = t!("equalizer-decibels-value", db = signed.clone()).into_owned();

    let slider = row()
        .key(format!("{slug}-slider"))
        .role("group")
        .label(title.clone())
        .w(theme.metrics.cover * TONE_SLIDER)
        .center_y()
        .child(
            Scrubber::new(
                format!("{slug}-scrubber"),
                scrubber,
                ((gain - low) / (high - low)) as f32,
            )
            .name(title.clone())
            .value(value.clone())
            .colors(theme.muted, theme.muted, theme.foreground)
            .on_move(move |ui: &mut Ui, fraction: f32, _: &mut Tx| {
                let gain = tone_gain(fraction);
                let Store {
                    equalizer,
                    settings,
                    cx,
                    ..
                } = &mut ui.store;
                let tone = shelf.with(equalizer.tone(), gain);
                equalizer.glide_tone(tone, settings, cx);
                ui.settle();
            })
            .build(theme),
        );
    let reset = Button::new(format!("{slug}-reset"))
        .small()
        .label(value)
        .tooltip(t!("equalizer-tone-reset"))
        .disabled(gain == 0.)
        .on_press(act(move |ui: &mut Ui| {
            let Store {
                equalizer,
                settings,
                cx,
                ..
            } = &mut ui.store;
            let tone = shelf.with(equalizer.tone(), 0.);
            equalizer.set_tone(tone, settings, cx);
        }))
        .build(theme)
        .w(theme.metrics.cover * TONE_VALUE);
    let controls = row()
        .key(format!("{slug}-controls"))
        .center_y()
        .gap(rems(theme, 0.75))
        .child(slider)
        .child(reset);

    Row::new(slug, title, controls).build(theme)
}

/// The preamp's row: whether it follows the filters on its own or holds a level, and that level,
/// in a field while it is held and as a figure in the field's place while automatic. It is the
/// sound's own preamp, before the tone lowers it further.
fn preamp_row(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let curve = ui.store.equalizer.curve();
    let auto = curve.preamp() == Preamp::Auto;
    let resolved = curve.preamp_db();
    let label = |auto: bool| match auto {
        true => t!("equalizer-preamp-auto").into_owned(),
        false => t!("equalizer-preamp-manual").into_owned(),
    };
    let mode = Picker::new(
        PREAMP,
        &ui.screens.equalizer.popovers,
        |ui: &mut Ui| &mut ui.screens.equalizer.popovers,
        label(auto),
    )
    .name(t!("equalizer-preamp"))
    .width(Picker::<Ui>::NARROW)
    .items([true, false].map(|choice| {
        MenuItem::new(
            format!("equalizer-preamp-mode-{}", usize::from(choice)),
            label(choice),
        )
        .selected(choice == auto)
        .on_press(act(move |ui: &mut Ui| set_preamp(ui, choice)))
    }))
    .build(theme, layer);
    let level = t!("equalizer-decibels-value", db = figure(resolved, 1)).into_owned();
    let width = theme.metrics.cover * FIELD;
    let value = match auto {
        true => figure_cell(
            theme,
            "equalizer-preamp-value".to_owned(),
            level.clone(),
            level,
            width,
        ),
        false => row()
            .key("equalizer-preamp-field")
            .role("group")
            .label(t!("equalizer-preamp-level"))
            .w(width)
            .child(
                rows::number(
                    ui,
                    "equalizer-preamp-input".to_owned(),
                    Field::Preamp,
                    "equalizer-decibels",
                )
                .grow(1.),
            ),
    };
    let controls = row()
        .key("equalizer-preamp-controls")
        .center_y()
        .gap(rems(theme, 0.5))
        .child(mode)
        .child(value.transition(rune::Transition::opacity()));

    col()
        .key("equalizer-preamp-block")
        .w_full()
        .mt(rems(theme, 0.5))
        .transition(fade())
        .child(Row::new("equalizer-preamp", t!("equalizer-preamp"), controls).build(theme))
}

/// Makes the preamp automatic, or holds it where the sound's automatic one was.
fn set_preamp(ui: &mut Ui, auto: bool) {
    let curve = ui.store.equalizer.curve().clone();
    if auto == (curve.preamp() == Preamp::Auto) {
        return;
    }
    // A manual preamp starts where the sound's automatic one was, rounded down so switching
    // never raises the level. The tone lowers both alike.
    let preamp = match auto {
        true => Preamp::Auto,
        false => Preamp::Manual(
            ((curve.preamp_db() * 10.).floor() / 10.)
                .clamp(*PREAMP_RANGE.start(), *PREAMP_RANGE.end()),
        ),
    };
    if let Ok(curve) = Curve::new(preamp, curve.filters().to_vec()) {
        apply(ui, curve);
    }
}

/// A name for the playing sound: the preset it matches while it is the unnamed profile, else the
/// first numbered profile name no profile shows yet.
fn suggested_name(ui: &Ui) -> String {
    let settings = &ui.store.settings;
    let preset = settings
        .equalizer_scratch()
        .filter(|scratch| *scratch == settings.equalizer_profile())
        .and_then(|_| Preset::matching(ui.store.equalizer.curve()));
    if let Some(preset) = preset {
        return i18n::lookup(preset_key(preset), None).into_owned();
    }
    let taken: Vec<String> = settings
        .equalizer_profiles()
        .map(|name| equalizer_profile_label(name).into_owned())
        .collect();
    (1..)
        .map(|number| t!("equalizer-profile-new", number = number).into_owned())
        .find(|name| !taken.contains(name))
        .unwrap_or_default()
}

/// Opens the name dialog for `naming`, filled with `text` and focused.
fn open_naming(ui: &mut Ui, naming: Naming, text: String) {
    let state = &mut ui.screens.equalizer;
    state.name.replace_text(text);
    state.name.focused = true;
    state.naming = Some(naming);
    state.popovers.close();
}

/// Opens the name dialog to save the playing sound as a new profile.
fn open_save(ui: &mut Ui) {
    let name = suggested_name(ui);
    open_naming(ui, Naming::Save, name);
}

/// Opens the name dialog on the playing profile, filled with the name it shows.
fn open_rename(ui: &mut Ui) {
    let settings = &ui.store.settings;
    let (index, stored) = (
        settings.equalizer_profile(),
        settings.equalizer_profile_name().to_owned(),
    );
    let shown = equalizer_profile_label(&stored).into_owned();
    open_naming(ui, Naming::Rename(index, stored), shown);
}

fn close_naming(ui: &mut Ui) {
    let state = &mut ui.screens.equalizer;
    state.naming = None;
    state.name.focused = false;
}

/// Saves or renames with the name the dialog holds. An empty name is not taken and the dialog
/// stays open.
fn submit_name(ui: &mut Ui) {
    let Some(naming) = ui.screens.equalizer.naming.clone() else {
        return;
    };
    let name = ui.screens.equalizer.name.text.clone();
    let Store {
        equalizer,
        settings,
        cx,
        ..
    } = &mut ui.store;
    let done = match naming {
        Naming::Rename(index, from) => settings.rename_equalizer_profile(index, &from, &name, cx),
        Naming::Save => {
            let named = !name.trim().is_empty();
            if named {
                equalizer.save_as(&name, settings, cx);
            }
            named
        }
    };
    if done {
        close_naming(ui);
    }
}

/// Asks before deleting the playing profile, and deletes the one it asked about.
fn delete(ui: &mut Ui) {
    let settings = &ui.store.settings;
    let index = settings.equalizer_profile();
    let name = settings.equalizer_profile_name().to_owned();
    confirm::ask(
        ui,
        Kind::EqualizerProfile(name.clone()),
        move |ui: &mut Ui| {
            let Store {
                equalizer,
                settings,
                cx,
                ..
            } = &mut ui.store;
            equalizer.remove_profile(index, &name, settings, cx);
            ui.settle();
        },
    );
}

/// The dialog that names a profile, titled for saving or renaming, mounted into `layer` while it
/// is open.
fn name_dialog(ui: &Ui, layer: &mut Vec<El>) {
    let Some(naming) = &ui.screens.equalizer.naming else {
        return;
    };
    let theme = &ui.theme;
    let title = match naming {
        Naming::Save => t!("equalizer-save-as"),
        Naming::Rename(..) => t!("equalizer-rename"),
    };
    let field = Input::new(
        "equalizer-name",
        &ui.screens.equalizer.name,
        t!("equalizer-profile-name-hint"),
        |ui: &mut Ui| &mut ui.screens.equalizer.name,
    )
    .on_submit(|ui: &mut Ui, _: &mut Tx| {
        submit_name(ui);
        ui.settle();
    })
    .build(theme);
    Modal::new("equalizer-name-dialog", title)
        .width(theme.metrics.cover * DIALOG)
        .child(row().key("equalizer-name-row").w_full().child(field))
        .action(
            Button::new("equalizer-name-cancel")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(close_naming)
                .build(theme),
        )
        .action(
            Button::new("equalizer-name-save")
                .primary()
                .label(t!("common-save"))
                .on_press(act(submit_name))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(close_naming)
        .build(theme, layer);
}

/// The gain a bass or treble slider at `fraction` of its travel stands for, in `TONE_STEP`s,
/// resting at 0 dB when it comes within `TONE_REST` of it.
fn tone_gain(fraction: f32) -> f64 {
    let (low, high) = (*TONE_RANGE.start(), *TONE_RANGE.end());
    let gain = low + f64::from(fraction) * (high - low);
    match gain.abs() < TONE_REST {
        true => 0.,
        false => (gain / TONE_STEP).round() * TONE_STEP,
    }
}
