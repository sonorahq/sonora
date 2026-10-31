use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, Focusable as _, Pixels, Render,
    ScrollHandle, SharedString, StyleRefinement, Task, WeakEntity, Window, div,
};
use i18n::t;
use music::equalizer::{Curve, MAX_FILTERS, PREAMP_RANGE, Preamp, Preset, TONE_RANGE, Tone};
use music::output::{DeviceOption, OutputId, Port};
use router::Destination;
use state::{
    AppSettings, Equalizer, Outputs, RuleDevice, Signal, Sonora, Sound, equalizer_profile_label,
};
use ui::{
    ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Input, Lazy, MenuItem, Modal, Picker,
    Popovers, Room, Scrollbar, Scrubber, ScrubberState, Submit, Switch, TabBar,
};

use crate::chrome::Chrome;
use crate::screens::settings::{self, Row};
use crate::shared::confirm::{self, Confirm};
use crate::shared::equalizer::{preset_key, sound_picker};

use graph::{EqualizerGraph, EqualizerSpectrum};
use meter::OutputMeter;
use rows::{Fields, figure, figure_cell};

mod graph;
mod meter;
mod rows;
mod rules;
mod transfer;

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

/// The equalizer's own screen. Simple mode shows the sound picker, undo, the response graph
/// and the bass and treble sliders, and advanced mode trades the sliders for the preamp and a
/// group each for the filters, the device rules and Equalizer APO text in and out.
pub(crate) struct EqualizerView {
    equalizer: Entity<Equalizer>,
    settings: Entity<AppSettings>,
    signal: Entity<Signal>,
    outputs: Entity<Outputs>,
    popovers: Popovers,
    scrollbar: Entity<Scrollbar>,
    preamp: Entity<Input>,
    rows: Vec<Fields>,
    /// The bass and treble sliders Simple mode shows under the graph.
    bass: ScrubberState,
    treble: ScrubberState,
    name: Entity<Input>,
    /// What the name dialog is open for, while it is.
    naming: Option<Naming>,
    /// The device and the sound chosen in the row that adds a device rule, until it is added.
    rule_device: Option<RuleDevice>,
    rule_sound: Option<Sound>,
    /// The name dialog's own focus, and what had focus before it opened, given back on close.
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    /// The curve the fields were last filled from, so they are only rewritten when it changes.
    shown: Option<Curve>,
    graph: Entity<EqualizerGraph>,
    /// The spectrum under the graph, which redraws on its own with every read.
    spectrum: Entity<EqualizerSpectrum>,
    /// The rest of the page in parts drawn as views of their own, made on the first render.
    parts: Option<Parts>,
    /// The level meter and clip warning under the graph, which redraw on their own.
    meter: Entity<OutputMeter>,
    /// What the screen last drew from the equalizer and the output, so a notify
    /// from them that changed none of it draws nothing.
    drawn: Option<Drawn>,
    /// The spectrum's, the output meter's and the signal path's reads, held while the screen is
    /// the one shown and dropped with it.
    watch: Option<Task<()>>,
    /// The file being read in or written out.
    file: Option<Task<()>>,
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

/// What the screen shows from the equalizer and the output. Position ticks and
/// meter reads that leave all of it as it was do not redraw the screen, and the level meter and
/// clip warning redraw on their own.
#[derive(PartialEq)]
struct Drawn {
    curve: Curve,
    tone: Tone,
    on: bool,
    history: (bool, bool),
    preamp: f64,
    output: Option<OutputId>,
    port: Option<Port>,
    /// What the device rules' menu offers.
    devices: Vec<DeviceOption>,
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

/// The parts of the page that are views of their own.
#[derive(Clone)]
struct Parts {
    controls: Entity<Part>,
    below: Entity<Part>,
    advanced: Entity<Part>,
}

/// What a part draws, from the page's own state.
type Build = fn(&mut EqualizerView, &mut Window, &mut Context<EqualizerView>) -> AnyElement;

/// A part of the page drawn as a view of its own and laid out cached. The spectrum and the meter
/// redraw the page every frame, and a cached part is not built again then. It redraws when the
/// page notifies, and stands as tall as its content measured the last time it drew.
struct Part {
    page: WeakEntity<EqualizerView>,
    build: Build,
    height: Pixels,
}

impl Part {
    fn new(page: &Entity<EqualizerView>, build: Build, cx: &mut Context<Self>) -> Self {
        cx.observe(page, |_, _, cx| cx.notify()).detach();
        Self {
            page: page.downgrade(),
            build,
            height: Pixels::ZERO,
        }
    }
}

impl Render for Part {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let build = self.build;
        let content = self
            .page
            .update(cx, |page, cx| build(page, window, cx))
            .unwrap_or_else(|_| div().into_any_element());
        let part = cx.entity().downgrade();
        div()
            .w_full()
            .when(self.height == Pixels::ZERO, |this| this.invisible())
            .child(content)
            .on_children_prepainted(move |bounds, _, cx| {
                let Some(height) = bounds.first().map(|bounds| bounds.size.height) else {
                    return;
                };
                part.update(cx, |part, cx| {
                    if part.height != height {
                        part.height = height;
                        cx.notify();
                    }
                })
                .ok();
            })
    }
}

impl EqualizerView {
    pub(crate) fn new(
        equalizer: Entity<Equalizer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let signal = Sonora::global(cx).signal.clone();
        let outputs = Sonora::global(cx).outputs.clone();
        let id = cx.entity_id();
        let preamp = cx.new(|cx| {
            Input::new("equalizer-decibels", cx)
                .compact()
                .align_end()
                .suffix("equalizer-decibels")
        });
        let rows = (0..MAX_FILTERS).map(|_| Fields::new(cx)).collect();
        let meter = cx.new(|cx| OutputMeter::new(equalizer.clone(), signal.clone(), cx));
        let graph = cx.new(|cx| {
            EqualizerGraph::new(equalizer.clone(), settings.clone(), outputs.clone(), cx)
        });
        let spectrum = cx.new(|cx| EqualizerSpectrum::new(signal.clone(), cx));

        cx.observe(&equalizer, |this, _, cx| {
            this.show_curve(cx);
            this.redraw(cx);
        })
        .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&outputs, |this, _, cx| this.redraw(cx)).detach();
        cx.subscribe(&graph, |_, _, _: &graph::Selected, cx| cx.notify())
            .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let mut view = Self {
            equalizer,
            settings,
            signal,
            outputs,
            popovers: Popovers::default(),
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
            preamp,
            rows,
            bass: ScrubberState::new("equalizer-bass"),
            treble: ScrubberState::new("equalizer-treble"),
            name: cx.new(|cx| Input::new("equalizer-profile-name-hint", cx)),
            naming: None,
            rule_device: None,
            rule_sound: None,
            focus: cx.focus_handle(),
            restore: None,
            shown: None,
            graph,
            spectrum,
            parts: None,
            meter,
            drawn: None,
            watch: None,
            file: None,
        };
        let fields: Vec<_> = view
            .fields()
            .map(|(field, input)| (field, input.read(cx).focus_handle(cx)))
            .collect();
        for (field, focus) in fields {
            cx.on_blur(&focus, window, move |this, _, cx| this.commit(field, cx))
                .detach();
        }
        view.show_curve(cx);
        view.drawn = Some(view.drawing(cx));
        view
    }

    /// The page's parts, made the first time the page draws, when it can be observed.
    fn parts(&mut self, cx: &mut Context<Self>) -> Parts {
        let page = cx.entity();
        self.parts
            .get_or_insert_with(|| Parts {
                controls: cx.new(|cx| Part::new(&page, Self::controls, cx)),
                below: cx.new(|cx| Part::new(&page, Self::below, cx)),
                advanced: cx.new(|cx| Part::new(&page, Self::advanced, cx)),
            })
            .clone()
    }

    /// What the screen shows from the equalizer and the output right now.
    fn drawing(&self, cx: &App) -> Drawn {
        let equalizer = self.equalizer.read(cx);
        let outputs = self.outputs.read(cx);
        Drawn {
            curve: equalizer.curve().clone(),
            tone: equalizer.tone(),
            on: equalizer.enabled(),
            history: equalizer.history(),
            preamp: equalizer.preamp_db(),
            output: outputs.binding_key(),
            port: outputs
                .playing_sink()
                .and_then(|sink| sink.shown_port().cloned()),
            devices: outputs.devices().to_vec(),
        }
    }

    /// Redraws the screen when anything it shows from the equalizer or the output changed since
    /// it last drew.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        let drawn = self.drawing(cx);
        if self.drawn.as_ref() != Some(&drawn) {
            self.drawn = Some(drawn);
            cx.notify();
        }
    }

    /// Reads the output meter, the signal path and the spectrum while the screen is the one
    /// shown, and the outputs the device rules offer once it comes into view.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible != self.watch.is_some() {
            self.watch = visible.then(|| self.signal.update(cx, |signal, cx| signal.watch(cx)));
            if visible {
                self.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
            }
        }
    }

    /// Closes an open menu on escape. Returns false when none was open.
    pub(crate) fn escape(&mut self) -> bool {
        let open = self.popovers.any();
        self.popovers.close();
        open
    }

    /// Plays `curve`, an edit made on this screen.
    fn apply(&mut self, curve: Curve, cx: &mut Context<Self>) {
        self.graph.update(cx, |graph, cx| graph.apply(curve, cx));
    }

    fn set_preamp(&mut self, auto: bool, cx: &mut Context<Self>) {
        let Some(curve) = self.shown.clone() else {
            return;
        };
        let preamp = curve.preamp_db();
        if auto == (curve.preamp() == Preamp::Auto) {
            return;
        }
        // A manual preamp starts where the sound's automatic one was, rounded down so switching
        // never raises the level. The tone lowers both alike.
        let preamp = match auto {
            true => Preamp::Auto,
            false => Preamp::Manual(
                ((preamp * 10.).floor() / 10.).clamp(*PREAMP_RANGE.start(), *PREAMP_RANGE.end()),
            ),
        };
        if let Ok(curve) = Curve::new(preamp, curve.filters().to_vec()) {
            self.apply(curve, cx);
        }
    }

    /// A name for the playing sound: the preset it matches while it is the unnamed profile, else
    /// the first numbered profile name no profile shows yet.
    fn suggested_name(&self, cx: &App) -> SharedString {
        let settings = self.settings.read(cx);
        let preset = settings
            .equalizer_scratch()
            .filter(|scratch| *scratch == settings.equalizer_profile())
            .and_then(|_| Preset::matching(self.equalizer.read(cx).curve()));
        if let Some(preset) = preset {
            return i18n::lookup(preset_key(preset), None);
        }
        let taken: Vec<SharedString> = settings
            .equalizer_profiles()
            .map(equalizer_profile_label)
            .collect();
        (1..)
            .map(|number| t!("equalizer-profile-new", number = number))
            .find(|name| !taken.contains(name))
            .unwrap_or_default()
    }

    /// Opens the name dialog for `naming`, filled with `text` and focused.
    fn open_naming(
        &mut self,
        naming: Naming,
        text: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore = window.focused(cx);
        self.name.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.focus(window, cx);
        });
        self.naming = Some(naming);
        cx.notify();
    }

    /// Opens the name dialog to save the playing sound as a new profile.
    fn open_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.suggested_name(cx);
        self.open_naming(Naming::Save, name, window, cx);
    }

    /// Opens the name dialog on the playing profile, filled with the name it shows.
    fn open_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);
        let (index, stored) = (
            settings.equalizer_profile(),
            settings.equalizer_profile_name().to_owned(),
        );
        let shown = equalizer_profile_label(&stored);
        self.open_naming(Naming::Rename(index, stored), shown, window, cx);
    }

    fn close_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.naming = None;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Saves or renames with the name the dialog holds. An empty name is not taken and the
    /// dialog stays open.
    fn submit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(naming) = self.naming.clone() else {
            return;
        };
        let name = self.name.read(cx).text().to_owned();
        let done = match naming {
            Naming::Rename(index, from) => self.settings.update(cx, |settings, cx| {
                settings.rename_equalizer_profile(index, &from, &name, cx)
            }),
            Naming::Save => {
                let named = !name.trim().is_empty();
                if named {
                    self.equalizer
                        .update(cx, |equalizer, cx| equalizer.save_as(&name, cx));
                }
                named
            }
        };
        if done {
            self.close_naming(window, cx);
        }
    }

    /// Asks before deleting the playing profile, and deletes the one it asked about.
    fn delete(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);
        let index = settings.equalizer_profile();
        let name = settings.equalizer_profile_name().to_owned();
        let equalizer = self.equalizer.clone();
        Confirm::ask(
            confirm::Kind::EqualizerProfile(name.clone().into()),
            move |cx| {
                equalizer.update(cx, |equalizer, cx| {
                    equalizer.remove_profile(index, &name, cx)
                })
            },
            cx,
        );
    }

    /// The Simple and Advanced switch, at the end of the sound group's heading. The choice is
    /// remembered across launches.
    fn mode(&self, advanced: bool, cx: &mut Context<Self>) -> AnyElement {
        let mode = |id: &'static str, label: SharedString, choice: bool| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .selected(advanced == choice)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_equalizer_advanced(choice, cx)
                    })
                }))
        };
        let modes = [
            mode("equalizer-simple", t!("equalizer-simple"), false),
            mode("equalizer-advanced", t!("equalizer-advanced"), true),
        ];

        TabBar::new("equalizer-mode")
            .name(t!("equalizer-mode"))
            .flex_none()
            .items(modes)
            .into_any_element()
    }

    /// The top of the sound group: its heading, then the one picker for every preset and profile
    /// with the on switch, undo and redo beside it. Advanced mode adds what can be done to the
    /// playing profile.
    fn controls(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let room = settings::Layout::of(window, cx).room();
        let advanced = self.settings.read(cx).equalizer_advanced();
        let theme = *cx.theme();
        let on = self.equalizer.read(cx).enabled();
        let settings = self.settings.read(cx);
        let only = settings.equalizer_profiles().count() < 2;
        // the unnamed profile is where presets load, so it keeps no name and is never removed
        let built_in = settings.equalizer_scratch() == Some(settings.equalizer_profile());
        let (undo, redo) = self.equalizer.read(cx).history();
        let width = theme.metrics.cover * SOUND_WIDTH;

        let picker = sound_picker(SOUND, &self.popovers, &self.equalizer, &self.settings, cx)
            .large()
            .left()
            .width(width)
            .map(|picker| match room {
                Room::Wide | Room::Vast => picker.w(width),
                Room::Roomy => picker.flex_1().min_w_0(),
                Room::Tight | Room::Snug => match advanced {
                    true => picker.w_full(),
                    false => picker.flex_1().min_w_0(),
                },
            });
        let action = |id: &'static str, icon: &'static str, tooltip: &'static str| {
            Button::new(id).ghost().icon(icon).tooltip(tooltip)
        };
        let history = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                action("equalizer-undo", "icons/undo-2.svg", "equalizer-undo")
                    .disabled(!undo)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.equalizer
                            .update(cx, |equalizer, cx| equalizer.undo(cx))
                    })),
            )
            .child(
                action("equalizer-redo", "icons/redo-2.svg", "equalizer-redo")
                    .disabled(!redo)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.equalizer
                            .update(cx, |equalizer, cx| equalizer.redo(cx))
                    })),
            );
        let manage = advanced.then(|| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .ml_auto()
                .child(
                    action("equalizer-save-as", "icons/plus.svg", "equalizer-save-as")
                        .on_click(cx.listener(|this, _, window, cx| this.open_save(window, cx))),
                )
                .child(
                    action("equalizer-rename", "icons/pencil.svg", "equalizer-rename")
                        .disabled(built_in)
                        .on_click(cx.listener(|this, _, window, cx| this.open_rename(window, cx))),
                )
                .child(
                    action("equalizer-delete", "icons/trash-2.svg", "equalizer-delete")
                        .disabled(only || built_in)
                        .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                )
        });
        let mode = self.mode(advanced, cx);

        div()
            .flex()
            .flex_col()
            .child(heading("equalizer-sound", SOUND_ICON, Some(mode), cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_2()
                    .gap_y_3()
                    .child(picker)
                    .child(
                        Switch::new("equalizer-enabled", on)
                            .name(t!("settings-equalizer"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.equalizer
                                    .update(cx, |equalizer, cx| equalizer.set_enabled(!on, cx))
                            })),
                    )
                    .child(history)
                    .children(manage),
            )
            .into_any_element()
    }

    /// The bottom of the sound group under the graph and the meter: the preamp's row in
    /// advanced mode, and the bass and treble sliders in simple mode.
    fn below(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.settings.read(cx).equalizer_advanced() {
            true => self.preamp_row(cx),
            false => self.tone_rows(cx),
        }
    }

    /// The groups only advanced mode shows, the filters, the device rules and Equalizer APO text
    /// in and out, each built only while it is on screen.
    fn advanced(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(settings::GROUPS)
            .child(card(&view, "equalizer-filters", |this, window, cx| {
                let row = settings::Layout::of(window, cx).room();
                let filters = this.filters(row.fits(Room::Roomy), cx);
                form(filters, cx).into_any_element()
            }))
            .child(card(&view, "equalizer-rules", |this, _, cx| this.rules(cx)))
            .child(card(&view, "equalizer-transfer", |this, _, cx| {
                this.transfer(cx)
            }))
            .into_any_element()
    }

    /// The bass and treble rows Simple mode shows under the graph, which tilt whatever sound
    /// plays without editing it.
    fn tone_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .mt_2()
            .child(self.tone_row(Shelf::Bass, cx))
            .child(self.tone_row(Shelf::Treble, cx))
            .into_any_element()
    }

    /// One shelf's row: its name, a slider from -12 to +12 dB, and the gain it is at. Pressing
    /// the gain puts the shelf back to 0 dB.
    fn tone_row(&self, shelf: Shelf, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let tone = self.equalizer.read(cx).tone();
        let gain = shelf.gain(tone);
        let (low, high) = (*TONE_RANGE.start(), *TONE_RANGE.end());
        let (title, state, reset) = match shelf {
            Shelf::Bass => (t!("equalizer-bass"), &self.bass, "equalizer-bass-reset"),
            Shelf::Treble => (
                t!("equalizer-treble"),
                &self.treble,
                "equalizer-treble-reset",
            ),
        };
        let signed = match gain > 0. {
            true => format!("+{}", figure(gain, 1)),
            false => figure(gain, 1),
        };

        let slider = div()
            .id(match shelf {
                Shelf::Bass => "equalizer-bass-slider",
                Shelf::Treble => "equalizer-treble-slider",
            })
            .role(Role::Group)
            .aria_label(title.clone())
            .w(theme.metrics.cover * TONE_SLIDER)
            .child(
                Scrubber::new(state, ((gain - low) / (high - low)) as f32)
                    .name(title.clone())
                    .value(t!("equalizer-decibels-value", db = signed.clone()))
                    .colors(theme.muted, theme.muted, theme.foreground)
                    .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                        let gain = tone_gain(*fraction);
                        let tone = shelf.with(this.equalizer.read(cx).tone(), gain);
                        this.equalizer
                            .update(cx, |equalizer, cx| equalizer.glide_tone(tone, cx));
                    })),
            );
        let value = Button::new(reset)
            .small()
            .w(theme.metrics.cover * TONE_VALUE)
            .label(t!("equalizer-decibels-value", db = signed))
            .tooltip("equalizer-tone-reset")
            .disabled(gain == 0.)
            .on_click(cx.listener(move |this, _, _, cx| {
                let tone = shelf.with(this.equalizer.read(cx).tone(), 0.);
                this.equalizer
                    .update(cx, |equalizer, cx| equalizer.set_tone(tone, cx));
            }));
        let controls = div()
            .flex()
            .items_center()
            .gap_3()
            .child(slider)
            .child(value)
            .into_any_element();

        Row::new(title, controls).into_any_element()
    }

    /// The preamp's row: whether it follows the filters on its own or holds a level, and that
    /// level, in a field while it is held and as a figure in the field's place while automatic.
    /// It is the sound's own preamp, before the tone lowers it further.
    fn preamp_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let curve = self.equalizer.read(cx).curve();
        let auto = curve.preamp() == Preamp::Auto;
        let resolved = curve.preamp_db();
        let mode = Picker::new(
            PREAMP,
            &self.popovers,
            match auto {
                true => t!("equalizer-preamp-auto"),
                false => t!("equalizer-preamp-manual"),
            },
        )
        .name(t!("equalizer-preamp"))
        .width(Picker::NARROW)
        .items(
            [
                (true, t!("equalizer-preamp-auto")),
                (false, t!("equalizer-preamp-manual")),
            ]
            .map(|(choice, label)| {
                MenuItem::new(("equalizer-preamp-mode", usize::from(choice)), label)
                    .selected(choice == auto)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_preamp(choice, cx)))
            }),
        );
        let level = t!("equalizer-decibels-value", db = figure(resolved, 1));
        let value = match auto {
            true => figure_cell(
                "equalizer-preamp-value",
                level.clone(),
                level,
                theme.metrics.cover * FIELD,
                cx,
            ),
            false => div()
                .id("equalizer-preamp-field")
                .role(Role::Group)
                .aria_label(t!("equalizer-preamp-level"))
                .flex_none()
                .w(theme.metrics.cover * FIELD)
                .child(self.preamp.clone())
                .into_any_element(),
        };
        let controls = div()
            .flex()
            .items_center()
            .gap_2()
            .child(mode)
            .child(value)
            .into_any_element();

        form(Row::new(t!("equalizer-preamp"), controls), cx).into_any_element()
    }

    /// The dialog that names a profile, titled for saving or renaming.
    fn name_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let title = match self.naming.as_ref()? {
            Naming::Save => t!("equalizer-save-as"),
            Naming::Rename(..) => t!("equalizer-rename"),
        };
        let theme = *cx.theme();
        let modal = Modal::new("equalizer-name-dialog", title)
            .w(theme.metrics.cover * DIALOG)
            .child(self.name.clone())
            .action(
                Button::new("equalizer-name-cancel")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_naming(window, cx))),
            )
            .action(
                Button::new("equalizer-name-save")
                    .primary()
                    .label(t!("common-save"))
                    .on_click(cx.listener(|this, _, window, cx| this.submit_name(window, cx))),
            )
            .on_dismiss(cx.listener(|this, _, window, cx| this.close_naming(window, cx)));

        Some(
            div()
                .absolute()
                .inset_0()
                .key_context(FORM_CONTEXT)
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                    cx.stop_propagation();
                    this.close_naming(window, cx);
                }))
                .on_action(cx.listener(|this, _: &Submit, window, cx| {
                    cx.stop_propagation();
                    this.submit_name(window, cx);
                }))
                .child(modal)
                .into_any_element(),
        )
    }
}

impl Render for EqualizerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = settings::Layout::of(window, cx).room();
        let advanced = self.settings.read(cx).equalizer_advanced();
        let parts = self.parts(cx);
        let theme = *cx.theme();
        let layer = || {
            StyleRefinement::default()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
        };

        let sound = div()
            .flex()
            .flex_col()
            .child(shown(&parts.controls, cx))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(graph::height(room, cx))
                    .mt_3()
                    .child(self.spectrum.clone().cached(layer()))
                    .child(self.graph.clone().cached(layer())),
            )
            .child(
                self.meter.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(theme.metrics.control_small)
                        .mt_3(),
                ),
            )
            .child(shown(&parts.below, cx));
        let body = div()
            .flex()
            .flex_col()
            .gap(settings::GROUPS)
            .child(sound)
            .when(advanced, |this| this.child(shown(&parts.advanced, cx)));

        settings::frame(
            &Destination::Equalizer,
            None,
            "equalizer-page",
            &self.scrollbar,
            body,
            window,
            cx,
        )
        .children(self.name_dialog(cx))
    }
}

/// `part` laid out cached at the height it measured.
fn shown(part: &Entity<Part>, cx: &App) -> impl IntoElement + use<> {
    let height = part.read(cx).height;
    part.clone()
        .cached(StyleRefinement::default().w_full().h(height))
}

/// A group's heading as every settings page draws it, its icon before an uppercase eyebrow, with
/// `action` at the end of its line when the group has one.
fn heading(
    key: &'static str,
    icon: &'static str,
    action: Option<AnyElement>,
    cx: &App,
) -> gpui::Stateful<Div> {
    let theme = cx.theme();
    let label = i18n::lookup(key, None);
    div()
        .id(SharedString::from(key))
        .role(Role::Heading)
        .aria_level(2)
        .aria_label(label.clone())
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .mb(theme.metrics.pad)
        .child(settings::group_heading(label, Some(icon), cx).mb_0())
        .children(action)
}

/// Gives `content` the form keys of the number fields in it: Enter commits the focused field and
/// Escape puts it back.
fn form(content: impl IntoElement, cx: &mut Context<EqualizerView>) -> Div {
    div()
        .key_context(FORM_CONTEXT)
        .on_action(cx.listener(|this, _: &Submit, window, cx| {
            cx.stop_propagation();
            this.submit(window, cx);
        }))
        .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
            cx.stop_propagation();
            this.revert(window, cx);
        }))
        .child(content)
}

/// An advanced group that is built and laid out only while it is on screen, so scrolling the
/// long page costs what shows rather than every group on it.
fn card(
    view: &Entity<EqualizerView>,
    id: &'static str,
    build: impl FnOnce(&mut EqualizerView, &mut Window, &mut Context<EqualizerView>) -> AnyElement
    + 'static,
) -> Lazy {
    let view = view.clone();
    Lazy::new(id, move |window, cx| {
        view.update(cx, |this, cx| build(this, window, cx))
    })
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
