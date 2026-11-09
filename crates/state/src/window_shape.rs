//! The Windows and macOS hook that lets [`crate::AppSettings::window_rounding`] reach the real
//! window. The `DwmSetWindowAttribute` and AppKit toolbar calls are platform code only `sonora`
//! links against, so it installs the closure here once its window is open, capturing whatever
//! reaches that window (its raw handle). The host calls [`apply_window_rounding`] whenever the
//! setting changes, so `state` never depends on the platform bindings. Linux and FreeBSD round
//! their own chrome in `views` instead, since there it is ordinary element styling rather than a
//! platform call.

use std::cell::RefCell;

use ui::Rounding;

type Apply = Box<dyn Fn(Rounding)>;

thread_local! {
    /// The hook, kept on the thread that owns the window, since AppKit and the window handle
    /// both belong to it.
    static HOOK: RefCell<Option<Apply>> = const { RefCell::new(None) };
}

/// Registers the platform-specific corner-rounding hook for the window the closure reaches. A
/// no-op until this is called, so Linux and FreeBSD simply never install one. Call it on the
/// thread that owns the window, which is also the one that calls [`apply_window_rounding`].
pub fn install_rounded_window_hook(apply: impl Fn(Rounding) + 'static) {
    HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(apply)));
}

/// Applies the window-rounding preference to the live window, if a hook is installed.
pub fn apply_window_rounding(rounding: Rounding) {
    HOOK.with(|hook| {
        if let Some(apply) = hook.borrow().as_ref() {
            apply(rounding);
        }
    });
}
