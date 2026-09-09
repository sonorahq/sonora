//! The thumbnail toolbar Windows draws under the taskbar preview.
//!
//! Hovering Sonora's taskbar button opens a thumbnail of the window, and this
//! puts previous, play/pause and next underneath it, so playback can be driven
//! without raising the window. They are the same three commands the tray menu
//! carries and they reach [`state::Playback`] the same way, through an
//! unbounded channel the window procedure writes to.
//!
//! Two Windows rules shape the module. The buttons cannot be added until the
//! shell has created the taskbar button, which it announces with the registered
//! `TaskbarButtonCreated` message, and they can be added only once per window —
//! every later change is an update. Both mean the module has to see the window's
//! messages, so it subclasses the window rather than owning one.

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use i18n::t;
use state::{PlaybackState, Sonora};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Controls::{
    HIMAGELIST, ILC_COLOR32, ImageList_Create, ImageList_Destroy, ImageList_ReplaceIcon,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, THB_BITMAP, THB_FLAGS,
    THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, RegisterWindowMessageW,
    SM_CXSMICON, WM_COMMAND,
};
use windows::core::{BOOL, w};

/// Command ids for the three buttons. They travel in the low word of the
/// `WM_COMMAND` parameter, so they only have to be unique within this window.
const PREVIOUS: u32 = 1;
const TOGGLE: u32 = 2;
const NEXT: u32 = 3;

/// `WM_COMMAND` notification code for a thumbnail toolbar button, from
/// `shobjidl_core.h`. The `windows` crate does not carry it.
const THBN_CLICKED: u32 = 0x1800;

/// Our slot in the window's subclass chain. Any value unique to this crate does.
const SUBCLASS: usize = 0x736f_6e6f;

/// Glyph names, resolved through the active icon pack like every other icon.
const GLYPHS: [&str; 4] = ["skip-back", "play-filled", "pause-filled", "skip-forward"];
const BACK: usize = 0;
const PLAY: usize = 1;
const PAUSE: usize = 2;
const FORWARD: usize = 3;

/// White reads on the dark thumbnail flyout, near-black on the light one.
const ON_DARK: &str = "#ffffff";
const ON_LIGHT: &str = "#1a1a1a";

/// What a button click asks of playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Previous,
    Toggle,
    Next,
}

/// The labels and state the toolbar is currently showing. Compared whole, so a
/// playback change that alters nothing visible costs no Windows call.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    previous: String,
    toggle: String,
    next: String,
    playing: bool,
    idle: bool,
    look: Look,
}

/// What the glyphs are drawn from. A change here means redrawing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    pack: &'static str,
    light: bool,
    size: i32,
}

struct Installed {
    hwnd: isize,
    _bar: Entity<ThumbBar>,
}

impl Global for Installed {}

/// Puts the toolbar under `hwnd`'s taskbar preview, replacing one installed on
/// an earlier window. Returns `false` when the shell will not hand out a
/// taskbar list, which is normal on a machine with no taskbar at all.
pub fn install(hwnd: *mut c_void, cx: &mut App) -> bool {
    let hwnd = HWND(hwnd);
    if let Some(installed) = cx.try_global::<Installed>()
        && installed.hwnd == hwnd.0 as isize
    {
        return true;
    }

    let (sender, receiver) = mpsc::unbounded_channel();
    let Some(bar) = Bar::new(hwnd, sender) else {
        return false;
    };
    let entity = cx.new(|cx| ThumbBar::new(bar, receiver, cx));
    cx.set_global(Installed {
        hwnd: hwnd.0 as isize,
        _bar: entity,
    });
    true
}

/// Owns the toolbar and keeps it in step with playback, the way `Tray` does for
/// the tray menu.
pub struct ThumbBar {
    bar: Bar,
    shown: Shown,
    _events: Task<()>,
}

impl ThumbBar {
    fn new(mut bar: Bar, mut receiver: UnboundedReceiver<Event>, cx: &mut Context<Self>) -> Self {
        let _events = cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                if this.upgrade().is_none() {
                    break;
                }
                cx.update(|cx| {
                    let playback = Sonora::global(cx).playback.clone();
                    playback.update(cx, |playback, cx| match event {
                        Event::Previous => playback.previous(cx),
                        Event::Toggle => playback.toggle_play(cx),
                        Event::Next => playback.next(cx),
                    });
                });
            }
        });

        let playback = Sonora::global(cx).playback.clone();
        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |this, _, cx| this.publish(cx))
            .detach();

        let shown = shown(cx);
        bar.show(&shown);
        Self {
            bar,
            shown,
            _events,
        }
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let shown = shown(cx);
        if shown == self.shown {
            return;
        }
        self.bar.show(&shown);
        self.shown = shown;
    }
}

/// The Win32 half: the taskbar list, the buttons and the window subclass.
///
/// The subclass procedure and this struct share one [`Shared`], because both
/// have to reach the buttons — the procedure to add them when the shell says the
/// taskbar button exists, this struct to update them when playback moves. Both
/// only ever run on the window's own thread, which is why an `Rc` is enough.
pub struct Bar {
    shared: Rc<RefCell<Shared>>,
    hwnd: HWND,
}

impl Bar {
    fn new(hwnd: HWND, sender: UnboundedSender<Event>) -> Option<Self> {
        // GPUI has already initialised COM on this thread; asking for an
        // apartment it is not in answers RPC_E_CHANGED_MODE, which is not a
        // failure for us. Deliberately unpaired with CoUninitialize.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

        let taskbar: ITaskbarList3 =
            match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
                Ok(taskbar) => taskbar,
                Err(error) => {
                    log::warn!("thumbbar: cannot reach the taskbar: {error:#}");
                    return None;
                }
            };
        if let Err(error) = unsafe { taskbar.HrInit() } {
            log::warn!("thumbbar: cannot start the taskbar list: {error:#}");
            return None;
        }

        let created = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
        let shared = Rc::new(RefCell::new(Shared {
            taskbar,
            hwnd,
            buttons: buttons(),
            images: None,
            look: None,
            created,
            added: false,
            sender,
        }));

        // The procedure borrows this reference for the life of the subclass;
        // `Drop` takes it back.
        let refdata = Rc::into_raw(shared.clone()) as usize;
        let hooked = unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, refdata) };
        if !hooked.as_bool() {
            drop(unsafe { Rc::from_raw(refdata as *const RefCell<Shared>) });
            log::warn!("thumbbar: cannot listen for taskbar button clicks");
            return None;
        }

        log::debug!("thumbbar: watching {hwnd:?} for taskbar message {created}");
        Some(Self { shared, hwnd })
    }

    /// Applies `shown` to the buttons, redrawing the glyphs first if the icon
    /// pack or the system's light/dark choice moved under them.
    fn show(&mut self, shown: &Shown) {
        let mut shared = self.shared.borrow_mut();
        shared.dress(shown);
        shared.push();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        let _ = unsafe { RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS) };
        let refdata = Rc::as_ptr(&self.shared);
        drop(unsafe { Rc::from_raw(refdata) });
        self.shared.borrow_mut().clear();
    }
}

/// State the window procedure and [`Bar`] both reach.
struct Shared {
    taskbar: ITaskbarList3,
    hwnd: HWND,
    buttons: [THUMBBUTTON; 3],
    images: Option<HIMAGELIST>,
    look: Option<Look>,
    created: u32,
    added: bool,
    sender: UnboundedSender<Event>,
}

impl Shared {
    /// Writes `shown` into the button array, rebuilding the glyphs when the look
    /// changed. Does not talk to the shell.
    fn dress(&mut self, shown: &Shown) {
        if self.look != Some(shown.look) {
            self.redraw(shown.look);
        }

        let toggle = match shown.playing {
            true => PAUSE,
            false => PLAY,
        };
        let glyphs = [BACK, toggle, FORWARD];
        let tips = [&shown.previous, &shown.toggle, &shown.next];
        let flags = match shown.idle {
            true => THBF_DISABLED,
            false => THBF_ENABLED,
        };

        for (at, button) in self.buttons.iter_mut().enumerate() {
            button.dwFlags = flags;
            button.iBitmap = glyphs[at] as u32;
            tip(button, tips[at]);
        }
    }

    /// Hands the buttons to the shell. The first call has to be an add and every
    /// later one an update, and both fail harmlessly before the taskbar button
    /// exists — the `TaskbarButtonCreated` message is what makes the add stick.
    fn push(&mut self) {
        let outcome = match self.added {
            false => unsafe { self.taskbar.ThumbBarAddButtons(self.hwnd, &self.buttons) },
            true => unsafe { self.taskbar.ThumbBarUpdateButtons(self.hwnd, &self.buttons) },
        };
        match outcome {
            Ok(()) if self.added => {}
            Ok(()) => {
                log::debug!("thumbbar: buttons are on the taskbar");
                self.added = true;
            }
            Err(error) if self.added => {
                log::warn!("thumbbar: cannot update the taskbar buttons: {error:#}");
            }
            Err(error) => {
                log::debug!("thumbbar: the taskbar button is not ready yet: {error:#}");
            }
        }
    }

    /// Redraws the glyphs at `look` into a fresh image list and hands it to the
    /// shell.
    ///
    /// The buttons name their glyph by index rather than carrying an `HICON`,
    /// because a realised thumbnail toolbar keeps the icon handle it was first
    /// given: swapping `hIcon` under it leaves the primary taskbar drawing the
    /// old glyph while a second monitor, whose flyout is built fresh, shows the
    /// new one. An index into the image list is the part Windows does re-read.
    fn redraw(&mut self, look: Look) {
        let drawn: Vec<HICON> = GLYPHS.iter().filter_map(|name| glyph(name, look)).collect();
        let full = drawn.len() == GLYPHS.len();
        let images = match full {
            true => list(&drawn, look.size),
            false => None,
        };
        for icon in drawn {
            let _ = unsafe { DestroyIcon(icon) };
        }

        let Some(images) = images else {
            log::warn!("thumbbar: cannot draw the taskbar button glyphs");
            return;
        };
        if let Err(error) = unsafe { self.taskbar.ThumbBarSetImageList(self.hwnd, images) } {
            log::warn!("thumbbar: cannot hand over the button glyphs: {error:#}");
            let _ = unsafe { ImageList_Destroy(Some(images)) };
            return;
        }

        log::debug!("thumbbar: drew {} glyphs at {}px", GLYPHS.len(), look.size);
        // Only now that the shell holds the new list is the old one free.
        self.clear();
        self.images = Some(images);
        self.look = Some(look);
    }

    fn clear(&mut self) {
        if let Some(images) = self.images.take() {
            let _ = unsafe { ImageList_Destroy(Some(images)) };
        }
        self.look = None;
    }
}

/// The window procedure. It runs on the window's thread, ahead of GPUI's own,
/// and hands every message it does not own to the rest of the chain.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    let shared = unsafe { &*(refdata as *const RefCell<Shared>) };

    if let Ok(mut shared) = shared.try_borrow_mut() {
        if message == shared.created {
            // The taskbar button has just appeared, so the add can land now.
            log::debug!("thumbbar: the shell created the taskbar button");
            shared.added = false;
            shared.push();
        } else if message == WM_COMMAND && (wparam.0 as u32 >> 16) == THBN_CLICKED {
            let event = match wparam.0 as u32 & 0xffff {
                PREVIOUS => Some(Event::Previous),
                TOGGLE => Some(Event::Toggle),
                NEXT => Some(Event::Next),
                _ => None,
            };
            if let Some(event) = event {
                shared.sender.send(event).ok();
                return LRESULT(0);
            }
        }
    }

    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// The three buttons, with the parts that never change already filled in.
fn buttons() -> [THUMBBUTTON; 3] {
    [PREVIOUS, TOGGLE, NEXT].map(|id| THUMBBUTTON {
        dwMask: THB_BITMAP | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        dwFlags: THBF_ENABLED,
        ..Default::default()
    })
}

/// Copies `text` into a button's fixed tooltip buffer, truncated to fit with a
/// terminator, since Windows reads it as a C string.
fn tip(button: &mut THUMBBUTTON, text: &str) {
    let room = button.szTip.len() - 1;
    let mut written = text.encode_utf16().take(room);
    button.szTip.fill(0);
    for slot in button.szTip.iter_mut() {
        match written.next() {
            Some(unit) => *slot = unit,
            None => break,
        }
    }
}

/// Rasterises one pack glyph into an icon Windows can own.
fn glyph(name: &str, look: Look) -> Option<HICON> {
    let bytes = icons::asset(&icons::path(name))?;
    let colour = match look.light {
        true => ON_LIGHT,
        false => ON_DARK,
    };
    // Pack SVGs are stroked and filled with `currentColor`, which has no meaning
    // outside a document; give it one before parsing.
    let source = std::str::from_utf8(bytes)
        .ok()?
        .replace("currentColor", colour);

    let tree = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default()).ok()?;
    let size = look.size.max(1) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width().max(1.0);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    icon(pixmap.data(), size)
}

/// Gathers the glyphs into an image list, which is the handle the toolbar reads
/// its button images from. The icons stay the caller's to destroy.
fn list(icons: &[HICON], size: i32) -> Option<HIMAGELIST> {
    let images = unsafe { ImageList_Create(size, size, ILC_COLOR32, icons.len() as i32, 0) };
    if images.0 == 0 {
        return None;
    }
    for icon in icons {
        if unsafe { ImageList_ReplaceIcon(images, -1, *icon) } < 0 {
            let _ = unsafe { ImageList_Destroy(Some(images)) };
            return None;
        }
    }
    Some(images)
}

/// Turns premultiplied RGBA into an `HICON`. Windows wants the channels the
/// other way round and the rows top down, which the negative height asks for.
fn icon(rgba: &[u8], size: u32) -> Option<HICON> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut c_void = std::ptr::null_mut();
    let colour =
        unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        let _ = unsafe { DeleteObject(HGDIOBJ(colour.0)) };
        return None;
    }
    unsafe {
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), rgba.len());
        for (out, pixel) in pixels.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            out.copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }

    // A 32-bit colour bitmap carries its own alpha, so the mask only has to be
    // the right shape.
    let mask = unsafe { CreateBitmap(size as i32, size as i32, 1, 1, None) };
    let info = ICONINFO {
        fIcon: BOOL(1),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: colour,
    };
    let drawn = unsafe { CreateIconIndirect(&info) }.ok();
    unsafe {
        let _ = DeleteObject(HGDIOBJ(colour.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
    }
    drawn
}

/// Reads what the toolbar should be showing right now.
fn shown(cx: &App) -> Shown {
    let playback = Sonora::global(cx).playback.read(cx);
    let playing = matches!(
        playback.state(),
        PlaybackState::Playing | PlaybackState::Loading
    );
    Shown {
        previous: t!("player-previous").to_string(),
        toggle: match playing {
            true => t!("tray-pause"),
            false => t!("tray-play"),
        }
        .to_string(),
        next: t!("player-next").to_string(),
        playing,
        idle: playback.track().is_none(),
        look: look(),
    }
}

fn look() -> Look {
    Look {
        pack: icons::active().id,
        light: light_theme(),
        size: unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16),
    }
}

/// Whether the shell is drawing its chrome light, which decides the glyph
/// colour. Read afresh on every publish, so switching Windows themes catches up
/// at the next playback change rather than needing a restart.
fn light_theme() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    read.is_ok() && value == 1
}
