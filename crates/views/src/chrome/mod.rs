//! The window's chrome: the title bar, both sidebars, the player bar and the toast stack.

pub(crate) mod aside;
pub(crate) mod downloads;
pub(crate) mod jam;
pub(crate) mod members;
pub(crate) mod notice;
pub(crate) mod output_picker;
pub(crate) mod player_bar;
pub(crate) mod session;
pub(crate) mod sessions_button;
pub(crate) mod sidebar_left;
pub(crate) mod sidebar_right;
pub(crate) mod swipe_cue;
pub(crate) mod theme_invite;
pub(crate) mod title_bar;
pub(crate) mod toasts;
pub(crate) mod toolbar;
pub(crate) mod tools;

use state::{AppSettings, SideTab};

/// The narrowest and widest the left sidebar may be dragged to.
pub(crate) const LEFT_MIN: f32 = 160.;
pub(crate) const LEFT_MAX: f32 = 400.;
/// The narrowest and widest the right sidebar may be dragged to.
pub(crate) const RIGHT_MIN: f32 = 240.;
pub(crate) const RIGHT_MAX: f32 = 560.;

/// Where the chrome stands: which panels are open and how wide, which groups are expanded.
pub struct ChromeState {
    pub left_open: bool,
    pub left_width: f32,
    pub right_open: bool,
    pub right_width: f32,
    pub right_tab: SideTab,
    /// Whether the sign-in page started an authorization, which keeps it up while it runs.
    pub signing_in: bool,
    /// The chrome's own menus and popovers.
    pub popovers: ui::Popovers,
    pub sidebar: sidebar_left::State,
    pub title_bar: title_bar::State,
    /// The shown page's search field in the title bar.
    pub toolbar: toolbar::State,
    pub player_bar: player_bar::State,
    pub toasts: toasts::State,
    pub sessions: sessions_button::State,
    pub output_picker: output_picker::State,
    /// The Jam's and the Sonora Session's dialogs.
    pub jam: jam::State,
    pub session: session::State,
    pub aside: sidebar_right::State,
    /// The arrow a touchpad swipe across the page draws.
    pub swipe: swipe_cue::State,
}

impl ChromeState {
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            left_open: settings.sidebar_open(),
            left_width: settings.sidebar_width().clamp(LEFT_MIN, LEFT_MAX),
            right_open: settings.sidebar_right_open(),
            right_width: settings.sidebar_right_width().clamp(RIGHT_MIN, RIGHT_MAX),
            right_tab: settings.sidebar_right_tab(),
            signing_in: false,
            popovers: ui::Popovers::default(),
            sidebar: sidebar_left::State::new(settings),
            title_bar: title_bar::State::default(),
            toolbar: toolbar::State::default(),
            player_bar: player_bar::State::default(),
            toasts: toasts::State::default(),
            sessions: sessions_button::State::default(),
            output_picker: output_picker::State::default(),
            jam: jam::State::default(),
            session: session::State::default(),
            aside: sidebar_right::State::default(),
            swipe: swipe_cue::State::default(),
        }
    }
}
