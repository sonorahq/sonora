//! The menu of output devices that both the output picker and the device rules open. Devices
//! are listed in the order the system gives them, a card's devices fold into a submenu once the
//! list is long, and the sound cards' own devices follow in one "Direct to hardware" submenu.
//! The caller says what each device's rows are and what picking one does.

use std::rc::Rc;

use i18n::t;
use music::output::DeviceOption;
use ui::{Menu, MenuItem};

use crate::Ui;

/// How tall a device menu grows before it scrolls.
const MENU_HEIGHT: f32 = 360.;
/// How wide a device menu is. A longer name is cut short rather than widening the menu.
const MENU_WIDTH: f32 = 320.;
/// Past this many devices, a card's devices are folded into a submenu, since a card with many
/// outputs otherwise floods the list.
const GROUP_AFTER: usize = 10;

/// Finds a device menu's state in the window's state.
type Access = Rc<dyn for<'a> Fn(&'a mut Ui) -> &'a mut DeviceMenu>;

/// Which of a device menu's submenus is open, kept by the caller beside the menu's own open
/// state. Clear it with [`DeviceMenu::reset`] as the menu opens, so none reopens already open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeviceMenu {
    open: Option<Group>,
}

/// A row of a device menu: one device by index, or a group of devices by index.
pub(crate) enum DeviceRow {
    Device(usize),
    Group { group: Group, members: Vec<usize> },
}

/// What a submenu of a device menu holds. Submenus are told apart by it, so a device plugged in
/// while the menu is open does not hand one card's submenu to another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    /// One ALSA card's devices, by card name.
    Card(String),
    /// The sound cards' own devices, which play past the sound server.
    Direct,
}

impl Group {
    /// A key for the group's row that stays put as devices come and go.
    fn key(&self) -> String {
        match self {
            Group::Card(card) => format!("output-device-group-{card}"),
            Group::Direct => "output-device-group-direct".to_owned(),
        }
    }
}

impl DeviceMenu {
    /// Shuts every submenu, for a menu about to open afresh.
    pub(crate) fn reset(&mut self) {
        self.open = None;
    }

    /// The menu's items for `rows`. `item` gives a device's rows by its index, under the label it
    /// goes by where it is listed, and `holds` says whether a device is the one chosen, so the
    /// submenu holding it is marked. `access` finds this state in the window's state.
    pub(crate) fn items(
        &self,
        rows: Vec<DeviceRow>,
        devices: &[DeviceOption],
        holds: impl Fn(&DeviceOption) -> bool,
        item: impl Fn(usize, String) -> Vec<MenuItem<Ui>>,
        access: impl for<'a> Fn(&'a mut Ui) -> &'a mut DeviceMenu + 'static,
    ) -> Vec<MenuItem<Ui>> {
        let access: Access = Rc::new(access);
        rows.into_iter()
            .flat_map(|row| match row {
                DeviceRow::Device(index) => item(index, devices[index].name.clone()),
                DeviceRow::Group { group, members } => {
                    let title = match &group {
                        Group::Card(card) => card.clone(),
                        Group::Direct => t!("settings-output-device-direct").into_owned(),
                    };
                    let holds_chosen = members.iter().any(|index| holds(&devices[*index]));
                    let direct = group == Group::Direct;
                    let key = group.key();
                    let menu = Menu::new(format!("{key}-submenu"))
                        .width(MENU_WIDTH)
                        .max_height(MENU_HEIGHT)
                        .items(members.into_iter().flat_map(|index| {
                            let device = &devices[index];
                            let label = match direct {
                                true => device.name.clone(),
                                false => device.label.clone(),
                            };
                            item(index, label)
                        }));
                    let open = self.open.as_ref() == Some(&group);
                    let access = access.clone();
                    let row = MenuItem::new(key.clone(), title)
                        .selected(holds_chosen)
                        .submenu(menu, open, move |ui: &mut Ui, open: bool| {
                            let state = access(ui);
                            match open {
                                true => state.open = Some(group.clone()),
                                false if state.open.as_ref() == Some(&group) => state.open = None,
                                false => {}
                            }
                        });
                    match direct {
                        true => vec![MenuItem::separator(format!("{key}-rule")), row],
                        false => vec![row],
                    }
                }
            })
            .collect()
    }
}

/// The menu the items go in, for a picker's `menu`.
pub(crate) fn menu(key: impl Into<String>) -> Menu<Ui> {
    Menu::new(key).width(MENU_WIDTH).max_height(MENU_HEIGHT)
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
