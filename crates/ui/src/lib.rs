//! Sonora's design system on Rune: the theme and its tokens, the layout ladder, and the elements
//! every screen is built from. Nothing here knows about music, state or playback.

mod artwork;
mod button;
mod checkbox;
mod color;
mod explicit;
mod glass;
mod info_card;
mod input;
mod kbd;
mod label;
mod layout;
mod menu;
mod metrics;
pub mod motion;
mod notice;
mod overlay;
mod paint;
mod palette;
mod pin;
mod radio;
mod scroller;
mod separator;
mod shield;
mod skeleton;
mod spinner;
mod switch;
mod table;
mod theme;
mod time;
mod tooltip;
mod vacancy;
mod view;
mod visualizer;

pub use artwork::{
    Art, Artwork, Avatar, Cover, Fetch, PREFETCH_REACH, Spawn, avatar, cover,
};
pub use button::{Button, SMALL_INSET};
pub use checkbox::Checkbox;
pub use color::{Hsla, Rgba, black, hsla, mix, rgb, rgba, transparent, white};
pub use explicit::ExplicitBadge;
pub use glass::{
    GLASS_BLUR, GLASS_FILL, bar_surface, blurring, frost, glass, popover_fill, popover_surface,
};
pub use info_card::{Fact, InfoCard};
pub use input::Input;
pub use kbd::Kbd;
pub use label::{
    Weight, eyebrow, faint, font, heading, spoken, spoken_eyebrow, spoken_heading, spoken_vacant,
    upper, vacant,
};
pub use layout::{ALWAYS, MIN_CONTENT, ROOMY, Room, SNUG, VAST, WIDE};
pub use menu::{MenuItem, Menu, MenuSearch, TRIGGER_GAP};
pub use metrics::{LEADING, Metrics, Rounding, Text, tucked};
pub use motion::{Motion, PRESS_SCALE, PRESS_SCALE_CARD, Pace, Saver, Springs, Stillness};
pub use notice::Notice;
pub use overlay::{Placement, anchor, bloom, rise, root, scrim};
pub use palette::CoverPalette;
pub use pin::{Pin, PinKind, Spot};
pub use radio::Radio;
pub use scroller::{
    ScrollMemory, ScrollWatch, Shelf, perch_room, perched, return_reach, return_top,
};
pub use separator::Separator;
pub use shield::shield;
pub use skeleton::{Initials, Skeleton};
pub use spinner::Spinner;
pub use switch::Switch;
pub use table::{
    ColumnSpec, Layout, Pending, Sort, Sorting, Table, TableCell, TableState, TextAlign, Toggle,
    Width, rank,
};
pub use theme::{
    BACKDROP_TRANSPARENCY, Backdrop, CoverFill, Look, MAX_FONT, MAX_LYRICS_SCALE,
    MAX_TRANSPARENCY, MIN_FONT, MIN_LYRICS_SCALE, Theme, ThemeKind, ThemeOverrides, WINDOW_BLUR,
    backdrop,
};
pub use time::{TABULAR, clock, runtime};
pub use tooltip::{Perch, Tipped, help_look};
pub use vacancy::Vacancy;
pub use view::Mode;
pub use visualizer::{Levels, VisualizerStyle};
