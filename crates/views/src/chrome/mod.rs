mod aside;
mod player_bar;
mod sidebar_left;
mod sidebar_right;
mod title_bar;
mod toasts;
mod toolbar;
pub(crate) mod tools;
mod update_notice;

pub(crate) use aside::Aside;
pub(crate) use player_bar::PlayerBar;
pub(crate) use sidebar_left::SidebarLeft;
pub(crate) use sidebar_right::SidebarRight;
pub(crate) use title_bar::{TitleBar, TitleBarEvent, TitleBarOptions};
pub(crate) use toasts::ToastStack;
pub(crate) use toolbar::{Searchable, Toolbar, Tooled};
pub(crate) use update_notice::UpdateNotice;

use gpui::prelude::*;
use gpui::{App, Div, Entity, Global, Pixels, Window, div};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use state::AppSettings;
use ui::{ActiveTheme as _, MIN_CONTENT, Room, eyebrow};

/// The window's own corner radius, or `None` when it shouldn't visibly round: server-side
/// decorations put the compositor in charge of the frame, `Rounding::Square` is the
/// explicit off state, and a fullscreen window always goes square no matter the setting.
/// Windows applies its rounding through DWM instead (see `state::apply_window_rounding`),
/// so this only matters for Linux/FreeBSD chrome that rounds its own corners to match —
/// GPUI has no way to clip a subtree to a rounded parent.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn window_radius(window: &Window, settings: &AppSettings) -> Option<Pixels> {
    if window.is_fullscreen() || settings.server_side_decorations() {
        return None;
    }
    match settings.window_rounding() {
        ui::Rounding::Square => None,
        rounding => Some(rounding.radius()),
    }
}

pub(crate) fn section_label(key: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_end()
        .h(cx.theme().metrics.list_row)
        .px_2()
        .pb_1()
        .child(eyebrow(i18n::lookup(key, None), cx))
}

pub(crate) fn cap(min: Pixels, max: Pixels, keep: Pixels, window: &Window) -> Pixels {
    let room = window.viewport_size().width - keep;
    max.min(room.max(min))
}

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Chrome {
    sidebar_left: Pixels,
    sidebar_right: Pixels,
}

struct Installed(Entity<Chrome>);

impl Global for Installed {}

impl Chrome {
    pub fn entity(cx: &mut App) -> Entity<Chrome> {
        if cx.try_global::<Installed>().is_none() {
            let chrome = cx.new(|_| Chrome::default());
            cx.set_global(Installed(chrome));
        }
        cx.global::<Installed>().0.clone()
    }

    pub(crate) fn publish(left: Pixels, right: Pixels, cx: &mut App) {
        let next = Self {
            sidebar_left: left,
            sidebar_right: right,
        };
        let chrome = Self::entity(cx);
        chrome.update(cx, |chrome, cx| {
            if *chrome != next {
                *chrome = next;
                cx.notify();
            }
        });
    }

    pub fn get(cx: &App) -> Self {
        cx.try_global::<Installed>()
            .map(|installed| *installed.0.read(cx))
            .unwrap_or_default()
    }

    pub fn sidebar_right(cx: &App) -> Pixels {
        Self::get(cx).sidebar_right
    }

    pub fn content(window: &Window, cx: &App) -> Pixels {
        let chrome = Self::get(cx);
        (window.viewport_size().width - chrome.sidebar_left - chrome.sidebar_right).max(MIN_CONTENT)
    }

    pub fn room(window: &Window, cx: &App) -> Room {
        Room::of(Self::content(window, cx))
    }
}
