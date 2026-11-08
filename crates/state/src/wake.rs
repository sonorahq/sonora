use gpui::{Context, Entity};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use tokio::sync::watch;

use crate::{AppSettings, Io, Playback, PlaybackState};

/// How long the Linux inhibitor waits before it asks the portal again after a failed request.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// What the platform is asked to keep awake. The display is only wanted along with the system.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Want {
    system: bool,
    display: bool,
}

/// Why the Linux portal holds no inhibitor after a request.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
enum NotHeld {
    /// The backend refused the flags, as a GNOME portal without gnome-session does for anything
    /// but idle. Asking again for the same `Want` gets the same answer.
    Refused,
    /// The portal could not be reached or the call failed, which may pass.
    Failed,
}

/// Keeps the system awake while music plays, and the display on while it plays in the focused
/// fullscreen view. Linux asks the desktop portal from tokio, so the lock lands a moment later.
pub struct Wake {
    settings: Entity<AppSettings>,
    playback: Entity<Playback>,
    fullscreen: bool,
    focused: bool,
    applied: Want,
    #[cfg(target_os = "macos")]
    assertions: Assertions,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    sender: watch::Sender<Want>,
}

impl Wake {
    pub fn new(
        settings: Entity<AppSettings>,
        playback: Entity<Playback>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |this, _, cx| this.apply(cx)).detach();
        cx.observe(&playback, |this, _, cx| this.apply(cx)).detach();

        let this = cx.weak_entity();
        cx.on_window_closed(move |cx, _| {
            if !cx.windows().is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.fullscreen = false;
                this.focused = false;
                this.apply(cx);
            })
            .ok();
        })
        .detach();

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let sender = {
            let (sender, receiver) = watch::channel(Want::default());
            io.spawn(inhibit(receiver));
            sender
        };
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let _ = io;

        Self {
            settings,
            playback,
            fullscreen: false,
            focused: false,
            applied: Want::default(),
            #[cfg(target_os = "macos")]
            assertions: Assertions::default(),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            sender,
        }
    }

    /// Records whether the window shows the fullscreen view, which the display lock needs.
    pub fn set_fullscreen(&mut self, on: bool, cx: &mut Context<Self>) {
        self.fullscreen = on;
        self.apply(cx);
    }

    /// Records whether the window has focus, which the display lock needs.
    pub fn set_focused(&mut self, on: bool, cx: &mut Context<Self>) {
        self.focused = on;
        self.apply(cx);
    }

    /// Works out what should be held now and hands it to the platform when it changed. Loading
    /// counts as playing only when play was asked for, so a track change or a rebuffer keeps
    /// the lock while a restored track loading paused at launch never takes it.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let playing = self.settings.read(cx).stay_awake()
            && match playback.state() {
                PlaybackState::Playing => true,
                PlaybackState::Loading => playback.wants_playing(),
                _ => false,
            };
        let want = Want {
            system: playing,
            display: playing && self.fullscreen && self.focused,
        };
        if want == self.applied {
            return;
        }
        self.applied = want;
        self.hold(want);
    }

    /// Hands `want` to the platform. Windows ties the execution state to the calling thread, so
    /// this runs on the main thread.
    fn hold(&mut self, want: Want) {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::Power::{
                ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };

            let mut flags = ES_CONTINUOUS;
            if want.system {
                flags |= ES_SYSTEM_REQUIRED;
            }
            if want.display {
                flags |= ES_DISPLAY_REQUIRED;
            }
            // SAFETY: called only on the main thread and only the documented request flags are passed.
            if unsafe { SetThreadExecutionState(flags) } == 0 {
                log::warn!("wake: cannot set the execution state");
            }
        }

        #[cfg(target_os = "macos")]
        self.assertions.set(want);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        self.sender.send_replace(want);

        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux",
            target_os = "freebsd"
        )))]
        let _ = want;
    }
}

/// The two power assertions on macOS, each held by its id while it is on.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Assertions {
    system: Option<objc2_io_kit::IOPMAssertionID>,
    display: Option<objc2_io_kit::IOPMAssertionID>,
}

#[cfg(target_os = "macos")]
impl Assertions {
    fn set(&mut self, want: Want) {
        hold_one(&mut self.system, want.system, "PreventUserIdleSystemSleep");
        hold_one(
            &mut self.display,
            want.display,
            "PreventUserIdleDisplaySleep",
        );
    }
}

/// Creates or releases one assertion of `kind` so that it is held exactly when `on` is true.
#[cfg(target_os = "macos")]
fn hold_one(held: &mut Option<objc2_io_kit::IOPMAssertionID>, on: bool, kind: &str) {
    use objc2_core_foundation::CFString;
    use objc2_io_kit::{
        IOPMAssertionCreateWithName, IOPMAssertionRelease, kIOPMAssertionLevelOn, kIOReturnSuccess,
    };

    if on == held.is_some() {
        return;
    }
    if on {
        let kind = CFString::from_str(kind);
        let name = CFString::from_str(&i18n::t!("wake-reason"));
        let mut id = 0;
        // SAFETY: both strings outlive the call and `id` is a valid out pointer.
        let result = unsafe {
            IOPMAssertionCreateWithName(Some(&kind), kIOPMAssertionLevelOn, Some(&name), &mut id)
        };
        match result == kIOReturnSuccess {
            true => *held = Some(id),
            false => log::warn!("wake: cannot hold a power assertion: {result}"),
        }
    } else if let Some(id) = held.take() {
        let _ = IOPMAssertionRelease(id);
    }
}

/// Holds the portal inhibitor that matches the latest `Want` until the sender drops. The new
/// inhibitor is taken before the old one is released. A failed request is retried every `RETRY`
/// while the old one stays held, and a refused one waits for the next `Want`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
async fn inhibit(mut receiver: watch::Receiver<Want>) {
    let mut proxy = None;
    let mut held = None;
    loop {
        let want = *receiver.borrow_and_update();
        let mut failed = false;
        match want.system || want.display {
            true => match acquire(&mut proxy, want).await {
                Ok(request) => release(held.replace(request)).await,
                Err(NotHeld::Refused) => {}
                Err(NotHeld::Failed) => failed = true,
            },
            false => release(held.take()).await,
        }
        let changed = match failed {
            true => tokio::select! {
                changed = receiver.changed() => changed,
                _ = tokio::time::sleep(RETRY) => Ok(()),
            },
            false => receiver.changed().await,
        };
        if changed.is_err() {
            break;
        }
    }
    release(held).await;
}

/// Asks the portal for an inhibitor matching `want`, reaching the portal first if `proxy` is
/// empty. A failed call clears `proxy` so the next attempt reconnects. The portal reports a
/// refusal in the response signal rather than as a call error, so the response is checked too.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
async fn acquire(
    proxy: &mut Option<ashpd::desktop::inhibit::InhibitProxy>,
    want: Want,
) -> Result<ashpd::desktop::Request<()>, NotHeld> {
    use ashpd::desktop::inhibit::{InhibitFlags, InhibitOptions, InhibitProxy};
    use ashpd::enumflags2::BitFlag;

    if proxy.is_none() {
        match InhibitProxy::new().await {
            Ok(reached) => *proxy = Some(reached),
            Err(error) => {
                log::warn!("wake: cannot reach the inhibit portal: {error}");
                return Err(NotHeld::Failed);
            }
        }
    }
    let Some(reached) = proxy.as_ref() else {
        return Err(NotHeld::Failed);
    };
    let mut flags = InhibitFlags::empty();
    if want.system {
        flags.insert(InhibitFlags::Suspend);
    }
    if want.display {
        flags.insert(InhibitFlags::Idle);
    }
    let reason = i18n::t!("wake-reason");
    let options = InhibitOptions::default().set_reason(reason.as_ref());
    let request = match reached.inhibit(None, flags, options).await {
        Ok(request) => request,
        Err(error) => {
            log::warn!("wake: cannot inhibit: {error}");
            *proxy = None;
            return Err(NotHeld::Failed);
        }
    };
    match request.response() {
        Ok(()) => Ok(request),
        Err(error) => {
            log::warn!("wake: the portal refused to inhibit {flags:?}: {error}");
            Err(NotHeld::Refused)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
async fn release(request: Option<ashpd::desktop::Request<()>>) {
    let Some(request) = request else {
        return;
    };
    if let Err(error) = request.close().await {
        log::warn!("wake: cannot release the inhibitor: {error}");
    }
}
