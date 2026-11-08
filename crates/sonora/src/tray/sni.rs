use anyhow::{Context as _, Result};
use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use ksni::{Category, ToolTip};
use tokio::sync::mpsc::UnboundedSender;

use super::{Art, Event, Shown};

const ID: &str = "sonora";
const ICON_NAME: &str = "sonora";
/// Flatpak writes this file into every sandbox it starts.
const FLATPAK_INFO: &str = "/.flatpak-info";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

pub struct Icon {
    /// What a fresh service is spawned from when the icon goes into the tray. It only follows
    /// `show` while the icon is out, since `Tray::place` publishes again right after the spawn.
    item: Item,
    handle: Option<Handle<Item>>,
}

impl Icon {
    pub fn new(sender: UnboundedSender<Event>) -> Option<Self> {
        let pixmap = crate::assets::LINUX_ICONS
            .iter()
            .filter(|(size, _)| *size <= 64)
            .filter_map(|(_, png)| match image::load_from_memory(png) {
                Ok(image) => {
                    let image = image.into_rgba8();
                    let (width, height) = image.dimensions();
                    let mut data = image.into_raw();
                    for pixel in data.as_chunks_mut::<4>().0 {
                        pixel.rotate_right(1);
                    }
                    Some(ksni::Icon {
                        width: width as i32,
                        height: height as i32,
                        data,
                    })
                }
                Err(error) => {
                    log::warn!("tray: cannot decode the tray icon: {error:#}");
                    None
                }
            })
            .collect();
        let item = Item {
            sender,
            pixmap,
            icon_name: icon_name(sandboxed()),
            shown: None,
        };
        Some(Self { item, handle: None })
    }

    /// Whether a status notifier watcher is on the session bus, so there is a tray to bring Sonora
    /// back from. It asks the bus, not the watcher, so it works while the icon is out.
    pub fn hosted() -> bool {
        match watched() {
            Ok(hosted) => hosted,
            Err(error) => {
                log::warn!("tray: cannot ask the session bus for a tray: {error:#}");
                false
            }
        }
    }

    /// Puts the icon in the tray, or takes it out. A host draws every item that is registered, so
    /// leaving the bus is the only way out.
    pub fn place(&mut self, placed: bool) -> Result<()> {
        match (self.handle.take(), placed) {
            // the request is sent, not awaited: the item leaves the bus either way
            (Some(handle), false) => drop(handle.shutdown()),
            (None, true) => self.handle = Some(spawn(self.item.clone())?),
            (handle, _) => self.handle = handle,
        }
        Ok(())
    }

    pub fn show(&mut self, shown: &Shown) {
        let shown = shown.clone();
        match &self.handle {
            Some(handle) => {
                handle.update(|item| item.shown = Some(shown));
            }
            None => self.item.shown = Some(shown),
        }
    }
}

fn spawn(item: Item) -> Result<Handle<Item>> {
    // A sandbox cannot own `org.kde.StatusNotifierItem-<pid>-<n>`, and a manifest cannot
    // grant it: flatpak's own-name wildcard only matches a `.*` suffix. The watcher
    // accepts the unique bus name instead.
    item.disable_dbus_name(sandboxed())
        .spawn()
        .context("cannot reach the status notifier host")
}

#[derive(Clone)]
struct Item {
    sender: UnboundedSender<Event>,
    pixmap: Vec<ksni::Icon>,
    icon_name: String,
    shown: Option<Shown>,
}

impl Item {
    fn send(&self, event: Event) {
        self.sender.send(event).ok();
    }

    fn entry(&self, label: &str, event: Event) -> MenuItem<Self> {
        StandardItem {
            label: label.to_owned(),
            activate: Box::new(move |this: &mut Self| this.send(event)),
            ..Default::default()
        }
        .into()
    }

    fn checkmark(&self, label: &str, checked: bool, event: Event) -> MenuItem<Self> {
        CheckmarkItem {
            label: label.to_owned(),
            checked,
            activate: Box::new(move |this: &mut Self| this.send(event)),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        ID.to_owned()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn title(&self) -> String {
        "Sonora".to_owned()
    }

    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmap.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Sonora".to_owned(),
            description: self
                .shown
                .as_ref()
                .map(|shown| shown.caption.clone())
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Show);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let Some(shown) = &self.shown else {
            return Vec::new();
        };
        vec![
            StandardItem {
                label: shown.caption.clone(),
                icon_data: cover(shown.artwork.as_deref()).unwrap_or_default(),
                enabled: shown.song,
                activate: Box::new(|this: &mut Self| this.send(Event::Song)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            self.entry(&shown.toggle, Event::Toggle),
            self.entry(&shown.previous, Event::Previous),
            self.entry(&shown.next, Event::Next),
            MenuItem::Separator,
            self.checkmark(&shown.shuffle, shown.shuffle_on, Event::Shuffle),
            self.checkmark(&shown.repeat, shown.repeat_on, Event::Repeat),
            MenuItem::Separator,
            self.entry(&shown.show, Event::Show),
            self.entry(&shown.quit, Event::Quit),
        ]
    }
}

fn watched() -> Result<bool> {
    let bus = zbus::blocking::Connection::session().context("cannot reach the session bus")?;
    let name = zbus::names::BusName::try_from(WATCHER).context("cannot name the watcher")?;
    zbus::blocking::fdo::DBusProxy::new(&bus)
        .context("cannot reach the bus daemon")?
        .name_has_owner(name)
        .context("cannot ask for the watcher")
}

fn sandboxed() -> bool {
    std::path::Path::new(FLATPAK_INFO).exists()
}

/// The themed icon to ask the host for, or none when the host's theme cannot be counted on to
/// have it. A Flatpak or AppImage install puts no `sonora` icon where the host looks, and hosts
/// such as Quickshell draw a placeholder for a name they cannot find rather than the pixmap.
fn icon_name(sandboxed: bool) -> String {
    let packaged = sandboxed || std::env::var_os("APPIMAGE").is_some();
    match packaged {
        true => String::new(),
        false => ICON_NAME.to_owned(),
    }
}

/// The cover as the png a menu item carries. A cover that cannot be encoded is simply left off
/// the row.
fn cover(art: Option<&Art>) -> Option<Vec<u8>> {
    let art = art?;
    let image = image::RgbaImage::from_raw(art.width, art.height, art.data.clone())?;

    let mut png = Vec::new();
    let written = image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    match written {
        Ok(()) => Some(png),
        Err(error) => {
            log::warn!("tray: cannot encode the cover: {error:#}");
            None
        }
    }
}
