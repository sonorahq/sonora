use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Context, ElementId, Entity, Pixels, px};
use i18n::t;
use music::output::DeviceOption;
use ui::{Menu, MenuItem, Scrollbar, SubmenuState};

/// How tall a device menu grows before it scrolls.
const MENU_HEIGHT: Pixels = px(360.);
/// How wide a device menu is. A longer name is cut short rather than widening the menu.
const MENU_WIDTH: Pixels = px(320.);
/// Past this many devices, a card's devices are folded into a submenu, since a card with many
/// outputs otherwise floods the list.
const GROUP_AFTER: usize = 10;

/// The menu of output devices that both the output picker and the device rules open. Devices
/// are listed in the order the system gives them, a card's devices fold into a submenu once the
/// list is long, and the sound cards' own devices follow in one "Direct to hardware" submenu.
/// The caller says what each device's rows are and what picking one does.
pub(crate) struct DeviceMenu {
    /// Scrolls the menu once it outgrows `MENU_HEIGHT`.
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

/// A row of a device menu: one device by index, or a group of devices by index.
pub(crate) enum DeviceRow {
    Device(usize),
    Group { group: Group, members: Vec<usize> },
}

/// What a submenu of a device menu holds.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    /// One ALSA card's devices, by card name.
    Card(String),
    /// The sound cards' own devices, which play past the sound server.
    Direct,
}

impl DeviceMenu {
    pub(crate) fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        let me = cx.entity_id();
        Self {
            scrollbar: cx.new(|_| Scrollbar::inset().watching(me)),
            groups: HashMap::new(),
        }
    }

    /// Readies a submenu's state for each group in `rows` while the menu is `open`, and lets
    /// them all go while it is shut. Call it every frame before `items`.
    pub(crate) fn prepare<T: 'static>(
        &mut self,
        rows: &[DeviceRow],
        open: bool,
        cx: &mut Context<T>,
    ) {
        if !open {
            self.groups.clear();
            return;
        }
        let me = cx.entity_id();
        for row in rows {
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

    /// The menu's items for `rows`. `item` gives a device's rows by its index, under the label it
    /// goes by where it is listed, and `holds` says whether a device is the one chosen, so the
    /// submenu holding it is marked.
    pub(crate) fn items(
        &self,
        rows: Vec<DeviceRow>,
        devices: &[DeviceOption],
        holds: impl Fn(&DeviceOption) -> bool,
        item: impl Fn(usize, String) -> Vec<MenuItem>,
    ) -> Vec<MenuItem> {
        rows.into_iter()
            .enumerate()
            .flat_map(|(place, row)| match row {
                DeviceRow::Device(index) => item(index, devices[index].name.clone()),
                DeviceRow::Group { group, members } => {
                    let title = match &group {
                        Group::Card(card) => card.clone().into(),
                        Group::Direct => t!("settings-output-device-direct"),
                    };
                    let Some(state) = self.groups.get(&group) else {
                        // the menu is shut, so the row is never shown
                        return vec![MenuItem::new(("output-device-group", place), title)];
                    };
                    let holds_chosen = members.iter().any(|index| holds(&devices[*index]));
                    let direct = group == Group::Direct;
                    let menu = Menu::new(("output-device-submenu", place))
                        .w(MENU_WIDTH)
                        .max_h(MENU_HEIGHT)
                        .scrollbar(state.scrollbar.clone())
                        .items(members.into_iter().flat_map(|index| {
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
            })
            .collect()
    }

    /// The menu the items go in, for a picker's `menu`.
    pub(crate) fn menu(&self, id: impl Into<ElementId>) -> Menu {
        Menu::new(id)
            .w(MENU_WIDTH)
            .max_h(MENU_HEIGHT)
            .scrollbar(self.scrollbar.clone())
    }
}

/// Lays out the devices `keep` lets through as menu rows, in the order the system lists them,
/// with the sound cards' own devices last, in one submenu. Up to `GROUP_AFTER` other devices
/// each get a row. Past that, a card with more than one device becomes a submenu, placed where
/// its first device was.
pub(crate) fn device_rows(
    devices: &[DeviceOption],
    keep: impl Fn(&DeviceOption) -> bool,
) -> Vec<DeviceRow> {
    let (direct, listed): (Vec<usize>, Vec<usize>) = (0..devices.len())
        .filter(|&index| keep(&devices[index]))
        .partition(|&index| devices[index].direct);
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
