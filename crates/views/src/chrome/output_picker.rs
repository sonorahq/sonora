use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Context, Entity, Pixels, SharedString, Window, div, px};
use i18n::t;
use music::output::DeviceOption;
use state::Outputs;
use ui::ActiveTheme as _;
use ui::{Menu, MenuItem, Picker, Popovers, Scrollbar, Spinner, SubmenuState, Text};

/// The popover key of the device menu, among the other popovers of the page it sits on.
const OUTPUT_DEVICES: &str = "output-devices";
/// How tall the device menu grows before it scrolls.
const MENU_HEIGHT: Pixels = px(360.);
/// How wide the device menus are. A longer name is cut short rather than widening the menu.
const MENU_WIDTH: Pixels = px(320.);
/// Past this many devices, a card's devices are folded into a submenu, since a card with many
/// outputs otherwise floods the list.
const GROUP_AFTER: usize = 10;

/// The output device picker: "System default" first, then every device the system offers, with
/// a card's devices folded into a submenu once the list is long. A sound server's outputs are
/// listed flat, and the sound cards' own devices follow in one "Direct to hardware" submenu.
/// Picking goes through `Outputs`, and the picker is held behind a spinner while a pick is being
/// opened.
pub(crate) struct OutputPicker {
    outputs: Entity<Outputs>,
    popovers: Popovers,
    /// Scrolls the device menu once it outgrows `MENU_HEIGHT`.
    scrollbar: Entity<Scrollbar>,
    /// Each submenu, by what it groups, so a device plugged in while the menu is open does not
    /// hand one card's submenu to another. Emptied while the menu is shut, so none reopens
    /// already open or scrolled.
    groups: HashMap<Group, DeviceGroup>,
}

/// What a card's submenu keeps between frames.
struct DeviceGroup {
    state: SubmenuState,
    scrollbar: Entity<Scrollbar>,
}

/// A row of the device menu: one device by index, or a group of devices by index.
enum DeviceRow {
    Device(usize),
    Group { group: Group, members: Vec<usize> },
}

/// What a submenu of the device menu holds.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Group {
    /// One ALSA card's devices, by card name.
    Card(String),
    /// The sound cards' own devices, which play past the sound server.
    Direct,
}

impl OutputPicker {
    /// `popovers` is the group of the page the picker sits on, so opening it closes the rest.
    pub(crate) fn new(
        outputs: Entity<Outputs>,
        popovers: Popovers,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&outputs, |_, _, cx| cx.notify()).detach();
        let me = cx.entity_id();
        Self {
            outputs,
            popovers,
            scrollbar: cx.new(|_| Scrollbar::inset().watching(me)),
            groups: HashMap::new(),
        }
    }
}

impl Render for OutputPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let rows = device_rows(self.outputs.read(cx).devices());
        match self.popovers.shows(OUTPUT_DEVICES) {
            true => {
                let me = cx.entity_id();
                for row in &rows {
                    if let DeviceRow::Group { group, .. } = row
                        && !self.groups.contains_key(group)
                    {
                        let state = DeviceGroup {
                            state: SubmenuState::instant(),
                            scrollbar: cx.new(|_| Scrollbar::inset().watching(me)),
                        };
                        self.groups.insert(group.clone(), state);
                    }
                }
            }
            false => self.groups.clear(),
        }

        let outputs = self.outputs.read(cx);
        let chosen = outputs.chosen();
        let switching = outputs.switching();
        let devices = outputs.devices();
        let label = match chosen {
            None => t!("settings-output-device-system"),
            Some(id) => match devices.iter().find(|device| device.id == id) {
                Some(device) => device.name.clone().into(),
                // until the list is read, the open stream is the one place the name is known
                None if !outputs.listed() => match outputs.active().filter(|open| open.id == id) {
                    Some(open) => open.name.clone().into(),
                    None => t!("settings-output-device-loading"),
                },
                None => t!("settings-output-device-missing"),
            },
        };

        // the default names the device it plays on, which only a sound server says
        let current = devices
            .iter()
            .find(|device| device.system)
            .map(|device| device.name.clone());
        let default = MenuItem::new("output-system", t!("settings-output-device-system"))
            .selected(chosen.is_none())
            .when_some(current, |item, current| {
                item.detail(div().text_color(muted).text_size(small).child(current))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.outputs
                    .update(cx, |outputs, cx| outputs.choose(None, cx));
            }));
        let item = |index: usize, label: String| {
            let device = &devices[index];
            let id = device.id.clone();
            MenuItem::new(SharedString::from(format!("output-device-{id}")), label)
                .selected(chosen == Some(id.as_str()))
                .when_some(device.detail.clone(), |item, detail| {
                    item.detail(div().text_color(muted).text_size(small).child(detail))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.outputs
                        .update(cx, |outputs, cx| outputs.choose(Some(id), cx));
                }))
        };
        let items = rows
            .into_iter()
            .enumerate()
            .flat_map(|(place, row)| match row {
                DeviceRow::Device(index) => vec![item(index, devices[index].name.clone())],
                DeviceRow::Group { group, members } => {
                    let holds_chosen = members
                        .iter()
                        .any(|index| chosen == Some(devices[*index].id.as_str()));
                    let title = match &group {
                        Group::Card(card) => card.clone().into(),
                        Group::Direct => t!("settings-output-device-direct"),
                    };
                    let Some(state) = self.groups.get(&group) else {
                        // the menu is shut, so the row is never shown
                        return vec![MenuItem::new(("output-device-group", place), title)];
                    };
                    let direct = group == Group::Direct;
                    let menu = Menu::new(("output-device-submenu", place))
                        .w(MENU_WIDTH)
                        .max_h(MENU_HEIGHT)
                        .scrollbar(state.scrollbar.clone())
                        .items(members.into_iter().map(|index| {
                            let device = &devices[index];
                            let label = match direct {
                                true => device.name.clone(),
                                false => device.label.clone(),
                            };
                            item(index, label)
                        }));
                    let row = MenuItem::new(("output-device-group", place), title)
                        .selected(holds_chosen)
                        .submenu(menu, state.state.clone());
                    match direct {
                        true => vec![
                            MenuItem::separator(("output-device-direct-rule", place)),
                            row,
                        ],
                        false => vec![row],
                    }
                }
            });
        let menu = Menu::new("output-devices-menu")
            .w(MENU_WIDTH)
            .max_h(MENU_HEIGHT)
            .scrollbar(self.scrollbar.clone());
        // a device plugged in since the list was read is only found by asking again
        let picker = Picker::new(OUTPUT_DEVICES, &self.popovers, label)
            .name(t!("settings-output-device"))
            // a long device name is cut short rather than crowding the setting's own label
            .max_w(Picker::WIDE)
            .on_open(cx.listener(|this, _, _, cx| {
                this.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
            }))
            .menu(menu)
            .disabled(switching)
            .item(default)
            .items(items);

        div()
            .flex()
            .items_center()
            .gap_2()
            .when(switching, |row| row.child(Spinner::new("output-switching")))
            .child(picker)
    }
}

/// Lays the devices out as menu rows, in the order the system lists them, with the sound cards'
/// own devices last, in one submenu. Up to `GROUP_AFTER` other devices each get a row. Past
/// that, a card with more than one device becomes a submenu, placed where its first device was.
fn device_rows(devices: &[DeviceOption]) -> Vec<DeviceRow> {
    let (direct, listed): (Vec<usize>, Vec<usize>) =
        (0..devices.len()).partition(|&index| devices[index].direct);
    let mut rows = listed_rows(devices, listed);
    if !direct.is_empty() {
        rows.push(DeviceRow::Group {
            group: Group::Direct,
            members: direct,
        });
    }
    rows
}

/// The rows of the devices at `listed`, with a card's devices folded together once there are
/// more than `GROUP_AFTER`.
fn listed_rows(devices: &[DeviceOption], listed: Vec<usize>) -> Vec<DeviceRow> {
    if listed.len() <= GROUP_AFTER {
        return listed.into_iter().map(DeviceRow::Device).collect();
    }
    let mut runs: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
    for index in listed {
        let device = &devices[index];
        let card = device.card.as_deref();
        match runs
            .iter_mut()
            .find(|(seen, _)| card.is_some() && *seen == card)
        {
            Some((_, members)) => members.push(index),
            None => runs.push((card, vec![index])),
        }
    }
    runs.into_iter()
        .flat_map(|(card, members)| match (card, members.len()) {
            (Some(card), 2..) => vec![DeviceRow::Group {
                group: Group::Card(card.to_owned()),
                members,
            }],
            _ => members.into_iter().map(DeviceRow::Device).collect(),
        })
        .collect()
}
