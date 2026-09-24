//! Global system-wide hotkeys for Windows.
//!
//! Registers shortcuts via Win32 `RegisterHotKey` on a dedicated background
//! listener thread so playback can be controlled from games, full-screen apps,
//! or when Sonora is running in the background.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use state::Sonora;
use tokio::sync::mpsc::{self, UnboundedSender};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey, VIRTUAL_KEY, VK_END, VK_LEFT, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
    VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_RIGHT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT, WM_USER,
};

const WM_HOTKEYS_DISABLE: u32 = WM_USER + 1;
const WM_HOTKEYS_RELOAD: u32 = WM_USER + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Stop,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
}

#[derive(Clone, Debug)]
struct HotkeyDef {
    id: i32,
    modifiers: HOT_KEY_MODIFIERS,
    vk: u32,
    action: Action,
    name: String,
}

pub fn parse_hotkey(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key_str = None;

    for part in s.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "super" | "cmd" | "meta" => modifiers |= MOD_WIN,
            other => {
                if key_str.is_some() {
                    return None;
                }
                key_str = Some(other.to_ascii_lowercase());
            }
        }
    }

    let key = key_str?;
    let vk = match key.as_str() {
        k if k.starts_with('f') && k.len() > 1 => {
            let num: u32 = k[1..].parse().ok()?;
            if (1..=24).contains(&num) {
                0x70 + (num - 1)
            } else {
                return None;
            }
        }
        k if k.len() == 1 => {
            let ch = k.chars().next()?;
            match ch {
                'a'..='z' => ch.to_ascii_uppercase() as u32,
                '0'..='9' => ch as u32,
                _ => return None,
            }
        }
        "space" => VK_SPACE.0 as u32,
        "left" | "arrowleft" => VK_LEFT.0 as u32,
        "right" | "arrowright" => VK_RIGHT.0 as u32,
        "up" | "arrowup" => 0x26,
        "down" | "arrowdown" => 0x28,
        "home" => 0x24,
        "end" => VK_END.0 as u32,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdown" => 0x22,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "mediaplaypause" | "media_play_pause" => VK_MEDIA_PLAY_PAUSE.0 as u32,
        "mediastop" | "media_stop" => VK_MEDIA_STOP.0 as u32,
        "medianext" | "media_next" => VK_MEDIA_NEXT_TRACK.0 as u32,
        "mediaprev" | "mediaprevious" | "media_prev" => VK_MEDIA_PREV_TRACK.0 as u32,
        _ => return None,
    };

    let is_bare_typing_key =
        (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) || vk == VK_SPACE.0 as u32;

    if is_bare_typing_key && (modifiers.0 & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_WIN.0)) == 0 {
        return None;
    }

    Some((modifiers, vk))
}

fn build_hotkey_definitions(
    play_pause: &str,
    stop: &str,
    next: &str,
    prev: &str,
    vol_up: &str,
    vol_down: &str,
) -> Vec<HotkeyDef> {
    let mut defs = Vec::new();
    let mut id = 1;

    struct Binding<'a> {
        shortcut: &'a str,
        action: Action,
        media: Option<(VIRTUAL_KEY, &'static str)>,
    }

    let entries = [
        Binding {
            shortcut: play_pause,
            action: Action::Toggle,
            media: Some((VK_MEDIA_PLAY_PAUSE, "Media Play/Pause")),
        },
        Binding {
            shortcut: next,
            action: Action::Next,
            media: Some((VK_MEDIA_NEXT_TRACK, "Media Next Track")),
        },
        Binding {
            shortcut: prev,
            action: Action::Previous,
            media: Some((VK_MEDIA_PREV_TRACK, "Media Previous Track")),
        },
        Binding {
            shortcut: stop,
            action: Action::Stop,
            media: Some((VK_MEDIA_STOP, "Media Stop")),
        },
        Binding {
            shortcut: vol_up,
            action: Action::VolumeUp,
            media: None,
        },
        Binding {
            shortcut: vol_down,
            action: Action::VolumeDown,
            media: None,
        },
    ];

    for entry in entries {
        if let Some((mods, vk)) = parse_hotkey(entry.shortcut) {
            defs.push(HotkeyDef {
                id,
                modifiers: mods,
                vk,
                action: entry.action,
                name: format!("{} ({:?})", entry.shortcut, entry.action),
            });
            id += 1;
        }
        if let Some((media_vk, media_name)) = entry.media {
            defs.push(HotkeyDef {
                id,
                modifiers: HOT_KEY_MODIFIERS(0),
                vk: media_vk.0 as u32,
                action: entry.action,
                name: media_name.to_string(),
            });
            id += 1;
        }
    }

    defs
}

struct Installed {
    _hotkeys: Entity<Hotkeys>,
}

impl Global for Installed {}

pub fn install(cx: &mut App) {
    if cx.has_global::<Installed>() {
        return;
    }

    let hotkeys = cx.new(Hotkeys::new);
    cx.set_global(Installed { _hotkeys: hotkeys });
}

struct Hotkeys {
    _events: Task<()>,
    thread_id: Arc<AtomicU32>,
    defs: Arc<std::sync::RwLock<Vec<HotkeyDef>>>,
    config: (bool, String, String, String, String, String, String),
}

impl Hotkeys {
    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Action>();
        let thread_id = Arc::new(AtomicU32::new(0));
        let tid_for_thread = thread_id.clone();

        let s = Sonora::global(cx).settings.read(cx);
        let config = (
            s.global_hotkeys(),
            s.hotkey_play_pause().to_owned(),
            s.hotkey_stop().to_owned(),
            s.hotkey_next().to_owned(),
            s.hotkey_previous().to_owned(),
            s.hotkey_volume_up().to_owned(),
            s.hotkey_volume_down().to_owned(),
        );

        let initial_defs = build_hotkey_definitions(
            &config.1, &config.2, &config.3, &config.4, &config.5, &config.6,
        );
        let defs = Arc::new(std::sync::RwLock::new(initial_defs));
        let defs_for_thread = defs.clone();
        let initial_enabled = config.0;

        std::thread::Builder::new()
            .name("sonora-global-hotkeys".into())
            .spawn(move || {
                listener(sender, tid_for_thread, initial_enabled, defs_for_thread);
            })
            .expect("hotkeys: cannot spawn background listener thread");

        let _events = cx.spawn(async move |this, cx| {
            while let Some(action) = receiver.recv().await {
                if this.upgrade().is_none() {
                    break;
                }
                cx.update(|cx| {
                    let playback = Sonora::global(cx).playback.clone();
                    playback.update(cx, |playback, cx| match action {
                        Action::Toggle => playback.toggle_play(cx),
                        Action::Stop => playback.pause(cx),
                        Action::Next => playback.next(cx),
                        Action::Previous => playback.previous(cx),
                        Action::VolumeUp => playback.set_volume(playback.volume() + 0.05, cx),
                        Action::VolumeDown => playback.set_volume(playback.volume() - 0.05, cx),
                    });
                });
            }
        });

        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |this, _, cx| {
            let s = Sonora::global(cx).settings.read(cx);
            let next_config = (
                s.global_hotkeys(),
                s.hotkey_play_pause().to_owned(),
                s.hotkey_stop().to_owned(),
                s.hotkey_next().to_owned(),
                s.hotkey_previous().to_owned(),
                s.hotkey_volume_up().to_owned(),
                s.hotkey_volume_down().to_owned(),
            );

            if this.config != next_config {
                this.config = next_config.clone();
                let (enabled, play_pause, stop, next, prev, vol_up, vol_down) = next_config;

                *this.defs.write().unwrap() =
                    build_hotkey_definitions(&play_pause, &stop, &next, &prev, &vol_up, &vol_down);

                let tid = this.thread_id.load(Ordering::SeqCst);
                if tid != 0 {
                    let msg = if enabled {
                        WM_HOTKEYS_RELOAD
                    } else {
                        WM_HOTKEYS_DISABLE
                    };
                    unsafe {
                        let _ = PostThreadMessageW(tid, msg, WPARAM(0), LPARAM(0));
                    }
                }
            }
        })
        .detach();

        Self {
            _events,
            thread_id,
            defs,
            config,
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        let tid = self.thread_id.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn register_all(defs: &[HotkeyDef], registered: &mut Vec<i32>) {
    if !registered.is_empty() {
        return;
    }
    for def in defs {
        let res = unsafe { RegisterHotKey(None, def.id, def.modifiers | MOD_NOREPEAT, def.vk) };
        match res {
            Ok(()) => {
                log::info!("hotkeys: registered global shortcut: {}", def.name);
                registered.push(def.id);
            }
            Err(error) => {
                log::debug!(
                    "hotkeys: cannot register global shortcut {}: {error:#}",
                    def.name
                );
            }
        }
    }
}

fn unregister_all(registered: &mut Vec<i32>) {
    for id in registered.drain(..) {
        unsafe {
            let _ = UnregisterHotKey(None, id);
        }
    }
    log::info!("hotkeys: unregistered all global shortcuts");
}

fn listener(
    sender: UnboundedSender<Action>,
    tid_target: Arc<AtomicU32>,
    initial_enabled: bool,
    defs_store: Arc<std::sync::RwLock<Vec<HotkeyDef>>>,
) {
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    tid_target.store(tid, Ordering::SeqCst);

    let mut msg = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }

    let mut defs = defs_store.read().unwrap().clone();
    let mut registered_ids = Vec::new();

    if initial_enabled {
        register_all(&defs, &mut registered_ids);
    } else {
        log::info!("hotkeys: initially disabled by user settings");
    }

    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        match msg.message {
            WM_HOTKEY => {
                let id = msg.wParam.0 as i32;
                if let Some(def) = defs.iter().find(|d| d.id == id) {
                    log::info!("hotkeys: triggered global shortcut: {}", def.name);
                    sender.send(def.action).ok();
                }
            }
            WM_HOTKEYS_DISABLE => {
                log::info!("hotkeys: disabling global shortcuts from settings");
                unregister_all(&mut registered_ids);
            }
            WM_HOTKEYS_RELOAD => {
                log::info!("hotkeys: reloading global shortcuts from settings");
                unregister_all(&mut registered_ids);
                defs = defs_store.read().unwrap().clone();
                register_all(&defs, &mut registered_ids);
            }
            _ => {}
        }
    }

    unregister_all(&mut registered_ids);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_f5() {
        let (mods, vk) = parse_hotkey("F5").expect("F5 should parse");
        assert_eq!(mods.0, 0);
        assert_eq!(vk, 0x74);
    }

    #[test]
    fn test_parse_f_keys() {
        for n in 1..=24 {
            let key = format!("F{n}");
            let (mods, vk) = parse_hotkey(&key).unwrap_or_else(|| panic!("{key} should parse"));
            assert_eq!(mods.0, 0);
            assert_eq!(vk, 0x70 + (n - 1));
        }
    }

    #[test]
    fn test_parse_ctrl_shift_space() {
        let (mods, vk) = parse_hotkey("Ctrl+Shift+Space").expect("Ctrl+Shift+Space should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(vk, VK_SPACE.0 as u32);
    }

    #[test]
    fn test_parse_ctrl_alt_s() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+S").expect("Ctrl+Alt+S should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_ALT).0);
        assert_eq!(vk, 0x53);
    }

    #[test]
    fn test_parse_arrows() {
        let (mods, vk) = parse_hotkey("Ctrl+Shift+Right").expect("Ctrl+Shift+Right should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(vk, VK_RIGHT.0 as u32);

        let (mods, vk) = parse_hotkey("Ctrl+Shift+Left").expect("Ctrl+Shift+Left should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(vk, VK_LEFT.0 as u32);

        let (mods, vk) = parse_hotkey("Ctrl+Shift+Up").expect("Ctrl+Shift+Up should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(vk, 0x26);

        let (mods, vk) = parse_hotkey("Ctrl+Shift+Down").expect("Ctrl+Shift+Down should parse");
        assert_eq!(mods.0, (MOD_CONTROL | MOD_SHIFT).0);
        assert_eq!(vk, 0x28);
    }

    #[test]
    fn test_safety_bare_typing_keys_rejected() {
        assert!(parse_hotkey("Space").is_none());
        assert!(parse_hotkey("A").is_none());
        assert!(parse_hotkey("v").is_none());
        assert!(parse_hotkey("1").is_none());
        assert!(parse_hotkey("Shift+A").is_none());
        assert!(parse_hotkey("Shift+Space").is_none());
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("Ctrl").is_none());
        assert!(parse_hotkey("Ctrl+").is_none());
        assert!(parse_hotkey("NonExistentKey123").is_none());
    }
}
