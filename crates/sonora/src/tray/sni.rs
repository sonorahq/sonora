use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Category, ToolTip};
use tokio::sync::mpsc::UnboundedSender;

use super::{Event, Shown};

const ID: &str = "sonora";
const ICON_NAME: &str = "sonora";
const PNG: &[u8] = include_bytes!("../../../../assets/tray/sonora.png");
/// Flatpak writes this file into every sandbox it starts.
const FLATPAK_INFO: &str = "/.flatpak-info";

pub struct Icon {
    handle: Handle<Item>,
}

impl Icon {
    pub fn new(sender: UnboundedSender<Event>) -> Option<Self> {
        let pixmap = match image::load_from_memory(PNG) {
            Ok(image) => {
                let image = image.into_rgba8();
                let (width, height) = image.dimensions();
                let mut data = image.into_raw();
                for pixel in data.chunks_exact_mut(4) {
                    pixel.rotate_right(1);
                }
                vec![ksni::Icon {
                    width: width as i32,
                    height: height as i32,
                    data,
                }]
            }
            Err(error) => {
                log::warn!("tray: cannot decode the tray icon: {error:#}");
                Vec::new()
            }
        };
        let item = Item {
            sender,
            pixmap,
            shown: None,
        };
        // A sandbox cannot own `org.kde.StatusNotifierItem-<pid>-<n>`, and a manifest cannot
        // grant it: flatpak's own-name wildcard only matches a `.*` suffix. The watcher
        // accepts the unique bus name instead.
        let sandboxed = std::path::Path::new(FLATPAK_INFO).exists();
        match item.disable_dbus_name(sandboxed).spawn() {
            Ok(handle) => Some(Self { handle }),
            Err(error) => {
                log::warn!("tray: cannot reach the status notifier host: {error}");
                None
            }
        }
    }

    pub fn show(&mut self, shown: &Shown) {
        let shown = shown.clone();
        self.handle.update(|item| item.shown = Some(shown));
    }
}

struct Item {
    sender: UnboundedSender<Event>,
    pixmap: Vec<ksni::Icon>,
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
        ICON_NAME.to_owned()
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
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            self.entry(&shown.toggle, Event::Toggle),
            self.entry(&shown.previous, Event::Previous),
            self.entry(&shown.next, Event::Next),
            MenuItem::Separator,
            self.entry(&shown.show, Event::Show),
            self.entry(&shown.quit, Event::Quit),
        ]
    }
}
