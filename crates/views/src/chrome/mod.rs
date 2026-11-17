mod aside;
mod downloads;
mod jam;
mod members;
mod notice;
mod output_picker;
mod player_bar;
mod session;
mod sessions_button;
mod sidebar_left;
mod sidebar_right;
mod swipe_cue;
mod theme_invite;
mod timeline;
mod title_bar;
mod toasts;
mod toolbar;
pub(crate) mod tools;

pub(crate) use aside::{Aside, VerseLook, verse, verse_gap, verse_sizes};
pub(crate) use jam::{JamCode, JamInvite, JamJoin};
pub(crate) use members::Members;
pub(crate) use notice::AppNotice;
pub(crate) use output_picker::OutputPicker;
pub(crate) use player_bar::PlayerBar;
pub(crate) use session::{SessionCode, SessionJoin};
pub(crate) use sidebar_left::SidebarLeft;
pub(crate) use sidebar_right::SidebarRight;
pub(crate) use swipe_cue::SwipeCue;
pub(crate) use theme_invite::ThemeInvite;
pub(crate) use timeline::Timeline;
#[cfg(target_os = "macos")]
pub use title_bar::traffic_lights;
pub(crate) use title_bar::{Busy, TitleBar, TitleBarEvent, TitleBarOptions};
pub use toasts::ToastStack;
pub(crate) use toolbar::{Searchable, Toolbar, Tooled};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, Div, Entity, Global, Pixels, SharedString, Window, div};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use state::AppSettings;
use state::{PageWidth, Sonora};
use ui::{ActiveTheme as _, MIN_CONTENT, Room, eyebrow};

/// The radius every element touching a corner of the window rounds that corner with, or `None`
/// under server-side decorations or `Rounding::Square`. It never exceeds half the title bar's
/// height, since GPUI clamps a quad's radius to half its shorter side and the title bar would
/// otherwise draw a tighter corner than the rest of the chrome.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) fn window_radius(settings: &AppSettings, cx: &App, window: &Window) -> Option<Pixels> {
    if window.is_fullscreen() || settings.server_side_decorations() {
        return None;
    }
    match settings.window_rounding() {
        ui::Rounding::Square => None,
        rounding => Some(rounding.radius().min(cx.theme().metrics.title_bar / 2.)),
    }
}

pub(crate) fn section_label(key: &'static str, cx: &App) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(format!("queue-section-{key}")))
        .role(Role::Heading)
        .aria_label(i18n::lookup(key, None))
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

/// The width a page takes out of a content area of `content`. The centred layout stops at
/// `page`, and the wide layout takes the whole of `content`.
fn page_fit(content: Pixels, page: Pixels, width: PageWidth) -> Pixels {
    match width {
        PageWidth::Centered => content.min(page),
        PageWidth::Wide => content,
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Chrome {
    sidebar_left: Pixels,
    sidebar_right: Pixels,
    /// The width the left sidebar leaves free for an open right sidebar, counted even while
    /// the window is too narrow to show it.
    reserved_right: Pixels,
    /// The part of a page covered by the translucent player bar.
    player_bottom: Pixels,
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

    pub(crate) fn publish(left: Pixels, right: Pixels, reserved_right: Pixels, cx: &mut App) {
        let next = Self {
            sidebar_left: left,
            sidebar_right: right,
            reserved_right,
            player_bottom: Self::get(cx).player_bottom,
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

    pub fn reserved_right(cx: &App) -> Pixels {
        Self::get(cx).reserved_right
    }

    /// The extra scroll space needed to bring a page's last item above the player bar.
    pub fn bottom(cx: &App) -> Pixels {
        Self::get(cx).player_bottom
    }

    /// Publishes the player bar's overlap after its responsive height has been resolved.
    pub(crate) fn set_bottom(bottom: Pixels, cx: &mut App) {
        Self::entity(cx).update(cx, |chrome, cx| {
            if chrome.player_bottom != bottom {
                chrome.player_bottom = bottom;
                cx.notify();
                cx.refresh_windows();
            }
        });
    }

    pub fn content(window: &Window, cx: &App) -> Pixels {
        let chrome = Self::get(cx);
        (window.viewport_size().width - chrome.sidebar_left - chrome.sidebar_right).max(MIN_CONTENT)
    }

    pub fn room(window: &Window, cx: &App) -> Room {
        Room::of(Self::content(window, cx))
    }

    /// The width a page lays itself out in. The centred layout stops it at `metrics.page`, and
    /// the wide layout lets it fill the content area. This is the one place the choice is read.
    pub fn page(window: &Window, cx: &App) -> Pixels {
        let width = Sonora::global(cx).settings.read(cx).page_width();
        page_fit(Self::content(window, cx), cx.theme().metrics.page, width)
    }

    /// How much wider than its page the content area is on each side. A page adds it to its
    /// side padding, so on a wide window it stands centred while its scrollbar and any
    /// background it bleeds still reach the edges.
    pub fn margin(window: &Window, cx: &App) -> Pixels {
        ((Self::content(window, cx) - Self::page(window, cx)) / 2.).max(Pixels::ZERO)
    }

    /// Classifies chrome that spans the whole window, including the space under sidebars.
    pub fn window_room(window: &Window) -> Room {
        Room::of(window.viewport_size().width)
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;
    use state::PageWidth;

    use super::page_fit;

    #[test]
    fn centred_pages_stop_at_the_page_width_on_a_wide_window() {
        assert_eq!(
            page_fit(px(2400.), px(1600.), PageWidth::Centered),
            px(1600.)
        );
    }

    #[test]
    fn centred_pages_fill_a_window_narrower_than_the_page_width() {
        assert_eq!(
            page_fit(px(1200.), px(1600.), PageWidth::Centered),
            px(1200.)
        );
    }

    #[test]
    fn wide_pages_fill_the_content_area_even_past_the_page_width() {
        assert_eq!(page_fit(px(2400.), px(1600.), PageWidth::Wide), px(2400.));
    }
}
