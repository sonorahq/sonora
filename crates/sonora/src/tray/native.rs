use anyhow::{Context as _, Result};
use tokio::sync::mpsc::UnboundedSender;
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, MenuItem,
    PredefinedMenuItem,
};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{Art, Event, Shown};

const TOOLTIP: &str = "Sonora";
#[cfg(target_os = "macos")]
const TEMPLATES: [&[u8]; 2] = [
    include_bytes!("../../../../assets/tray/template-18.png"),
    include_bytes!("../../../../assets/tray/template-36.png"),
];
const MENU_ON_CLICK: bool = cfg!(target_os = "macos");
#[cfg(target_os = "macos")]
const TEMPLATE_POINTS: f64 = 18.;

/// The tray icon and its menu. The status item is built the first time it goes into the tray, so
/// a Sonora started with the icon off never shows it, not even for a frame.
pub struct Icon {
    /// The Sonora logo the status item is built with.
    logo: tray_icon::Icon,
    menu: Menu,
    icon: Option<TrayIcon>,
    /// The last caption, for the tooltip of a status item built after it was shown.
    tooltip: String,
    caption: IconMenuItem,
    toggle: MenuItem,
    previous: MenuItem,
    next: MenuItem,
    shuffle: CheckMenuItem,
    repeat: CheckMenuItem,
    show: MenuItem,
    quit: MenuItem,
}

impl Icon {
    pub fn new(sender: UnboundedSender<Event>) -> Option<Self> {
        let logo = match logo() {
            Ok(logo) => logo,
            Err(error) => {
                log::warn!("tray: cannot build the tray icon: {error:#}");
                return None;
            }
        };

        let caption = IconMenuItem::with_id("caption", "", false, None, None);
        let toggle = MenuItem::with_id("toggle", "", true, None);
        let previous = MenuItem::with_id("previous", "", true, None);
        let next = MenuItem::with_id("next", "", true, None);
        let shuffle = CheckMenuItem::with_id("shuffle", "", true, false, None);
        let repeat = CheckMenuItem::with_id("repeat", "", true, false, None);
        let show = MenuItem::with_id("show", "", true, None);
        let quit = MenuItem::with_id("quit", "", true, None);
        let menu = Menu::new();
        if let Err(error) = menu.append_items(&[
            &caption,
            &PredefinedMenuItem::separator(),
            &toggle,
            &previous,
            &next,
            &PredefinedMenuItem::separator(),
            &shuffle,
            &repeat,
            &PredefinedMenuItem::separator(),
            &show,
            &quit,
        ]) {
            log::warn!("tray: cannot build the tray menu: {error:#}");
            return None;
        }

        let menus = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(event) = translate(&event.id) {
                menus.send(event).ok();
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            else {
                return;
            };
            if !MENU_ON_CLICK {
                sender.send(Event::Show).ok();
            }
        }));

        Some(Self {
            logo,
            menu,
            icon: None,
            tooltip: TOOLTIP.to_owned(),
            caption,
            toggle,
            previous,
            next,
            shuffle,
            repeat,
            show,
            quit,
        })
    }

    /// Whether there is a tray to bring Sonora back from, which Windows and macOS always have.
    pub fn hosted() -> bool {
        true
    }

    /// Puts the icon in the tray, or takes it out. The status item stays alive once built, so the
    /// menu and its handlers survive a round trip.
    pub fn place(&mut self, placed: bool) -> Result<()> {
        match &self.icon {
            Some(icon) => icon
                .set_visible(placed)
                .context("cannot show or hide the status item")?,
            None if placed => self.icon = Some(self.build()?),
            None => {}
        }
        Ok(())
    }

    /// Builds the status item, which puts it in the tray.
    fn build(&self) -> Result<TrayIcon> {
        let icon = TrayIconBuilder::new()
            .with_icon(self.logo.clone())
            .with_icon_as_template(true)
            .with_tooltip(&self.tooltip)
            .with_menu(Box::new(self.menu.clone()))
            .with_menu_on_left_click(MENU_ON_CLICK)
            .build()
            .context("cannot build the status item")?;
        #[cfg(target_os = "macos")]
        {
            set_template(&icon)?;
            keep_cover(&icon);
        }
        Ok(icon)
    }

    pub fn show(&mut self, shown: &Shown) {
        // the status notifier hosts read the caption off the tooltip themselves; here it has to
        // be pushed, or hovering the icon only ever says Sonora
        self.tooltip.clone_from(&shown.caption);
        if let Some(icon) = &self.icon
            && let Err(error) = icon.set_tooltip(Some(&shown.caption))
        {
            log::warn!("tray: cannot set the tooltip: {error:#}");
        }
        self.caption.set_text(&shown.caption);
        self.caption.set_icon(cover(shown.artwork.as_deref()));
        self.caption.set_enabled(shown.song);
        self.toggle.set_text(&shown.toggle);
        self.previous.set_text(&shown.previous);
        self.next.set_text(&shown.next);
        self.shuffle.set_text(&shown.shuffle);
        self.shuffle.set_checked(shown.shuffle_on);
        self.repeat.set_text(&shown.repeat);
        self.repeat.set_checked(shown.repeat_on);
        self.show.set_text(&shown.show);
        self.quit.set_text(&shown.quit);
    }
}

/// Loads the Windows artwork from the executable at the system tray's current pixel size.
#[cfg(windows)]
fn logo() -> Result<tray_icon::Icon> {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{SM_CXSMICON, SM_CYSMICON};

    let size = unsafe {
        let dpi = GetDpiForSystem();
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi) as u32,
            GetSystemMetricsForDpi(SM_CYSMICON, dpi) as u32,
        )
    };
    tray_icon::Icon::from_resource(1, Some(size)).context("cannot load the tray icon resource")
}

#[cfg(target_os = "macos")]
fn logo() -> Result<tray_icon::Icon> {
    let image = image::load_from_memory(TEMPLATES[1])
        .context("cannot decode the tray template")?
        .into_rgba8();
    let (width, height) = image.dimensions();
    tray_icon::Icon::from_rgba(image.into_raw(), width, height)
        .context("cannot build the tray template")
}

/// Installs both pixel densities at 18 points so AppKit can choose without resampling.
#[cfg(target_os = "macos")]
fn set_template(icon: &TrayIcon) -> Result<()> {
    use objc2::AnyThread as _;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSData, NSSize};

    let mtm =
        MainThreadMarker::new().context("cannot set the tray template off the main thread")?;
    let button = icon
        .ns_status_item()
        .and_then(|item| item.button(mtm))
        .context("cannot find the status item button")?;
    let size = NSSize::new(TEMPLATE_POINTS, TEMPLATE_POINTS);
    let template = NSImage::initWithSize(NSImage::alloc(), size);
    for png in TEMPLATES {
        let data = NSData::with_bytes(png);
        let image = NSImage::initWithData(NSImage::alloc(), &data)
            .context("cannot decode the tray template representation")?;
        for representation in image.representations() {
            representation.setSize(size);
            template.addRepresentation(&representation);
        }
    }
    template.setTemplate(true);
    button.setImage(Some(&template));
    Ok(())
}

/// The cover as a menu icon. A cover that cannot be built is simply left off the row.
fn cover(art: Option<&Art>) -> Option<MenuIcon> {
    let art = art?;
    match MenuIcon::from_rgba(art.data.clone(), art.width, art.height) {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("tray: cannot build the cover icon: {error:#}");
            None
        }
    }
}

/// Keeps the cover on the caption row. From macOS 27 AppKit decides on its own whether a menu
/// item shows its image and typically hides it, so the caption asks for its image to stay. Older
/// systems do not know the selector and always show it.
#[cfg(target_os = "macos")]
fn keep_cover(icon: &TrayIcon) {
    use objc2::runtime::NSObjectProtocol as _;
    use objc2::{MainThreadMarker, msg_send, sel};

    const VISIBLE: isize = 1;
    let Some(mtm) = MainThreadMarker::new() else {
        return log::warn!("tray: the tray menu was built off the main thread");
    };
    let Some(caption) = icon
        .ns_status_item()
        .and_then(|item| item.menu(mtm))
        .and_then(|menu| menu.itemAtIndex(0))
    else {
        return log::warn!("tray: the caption row is missing from the tray menu");
    };
    if !caption.respondsToSelector(sel!(setPreferredImageVisibility:)) {
        return;
    }
    unsafe { msg_send![&*caption, setPreferredImageVisibility: VISIBLE] }
}

fn translate(id: &MenuId) -> Option<Event> {
    Some(match id.0.as_str() {
        "caption" => Event::Song,
        "toggle" => Event::Toggle,
        "previous" => Event::Previous,
        "next" => Event::Next,
        "shuffle" => Event::Shuffle,
        "repeat" => Event::Repeat,
        "show" => Event::Show,
        "quit" => Event::Quit,
        _ => return None,
    })
}
