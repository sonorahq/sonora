//! What the title bar, the sidebar and the player bar hold, and in which order.
//!
//! The three bars draw from one set of [`Tool`]s. Each tool lives in at most one bar, except the
//! spacer and the separator, which can repeat. A tool that is in no bar waits in the customize
//! palette. [`Panels`] is the arrangement, kept in `settings.json` by tool id.

use serde::{Deserialize, Serialize};

/// One of the three bars a [`Tool`] can stand in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    TitleBar,
    Sidebar,
    PlayerBar,
}

impl Zone {
    pub const ALL: [Self; 3] = [Self::TitleBar, Self::Sidebar, Self::PlayerBar];

    /// The Fluent key naming the bar.
    pub fn key(self) -> &'static str {
        match self {
            Self::TitleBar => "panels-zone-title-bar",
            Self::Sidebar => "panels-zone-sidebar",
            Self::PlayerBar => "panels-zone-player-bar",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::TitleBar => "title-bar",
            Self::Sidebar => "sidebar",
            Self::PlayerBar => "player-bar",
        }
    }
}

/// Something the listener can put in a bar, move between bars or take out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Home,
    Search,
    History,
    Settings,
    Songs,
    Albums,
    Artists,
    Playlists,
    /// The "Library" heading over the library rows, a rule on the rail.
    Library,
    SidebarToggle,
    Back,
    Forward,
    Refresh,
    Downloads,
    /// The current screen's own controls, such as its sort, filter and search.
    PageTools,
    Account,
    /// The button that shows or hides the right panel with the lyrics and the queue.
    Panel,
    Lyrics,
    Queue,
    Sessions,
    /// A gap that takes the room left over, which pushes what follows it to the far end.
    Spacer,
    Separator,
}

impl Tool {
    pub const ALL: [Self; 22] = [
        Self::Home,
        Self::Search,
        Self::History,
        Self::Settings,
        Self::Library,
        Self::Songs,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::SidebarToggle,
        Self::Back,
        Self::Forward,
        Self::Refresh,
        Self::Downloads,
        Self::PageTools,
        Self::Account,
        Self::Panel,
        Self::Lyrics,
        Self::Queue,
        Self::Sessions,
        Self::Spacer,
        Self::Separator,
    ];

    /// The id the tool is stored under. A navigation tool shares its id with `router::NavEntry`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Search => "search",
            Self::History => "history",
            Self::Settings => "settings",
            Self::Songs => "songs",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Playlists => "playlists",
            Self::Library => "library",
            Self::SidebarToggle => "sidebar-toggle",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Refresh => "refresh",
            Self::Downloads => "downloads",
            Self::PageTools => "page-tools",
            Self::Account => "account",
            Self::Panel => "panel",
            Self::Lyrics => "lyrics",
            Self::Queue => "queue",
            Self::Sessions => "sessions",
            Self::Spacer => "spacer",
            Self::Separator => "separator",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// Whether the tool opens one of the app's pages.
    pub fn navigates(self) -> bool {
        matches!(
            self,
            Self::Home
                | Self::Search
                | Self::History
                | Self::Settings
                | Self::Songs
                | Self::Albums
                | Self::Artists
                | Self::Playlists
        )
    }

    /// Whether `zone` can hold the tool. Navigation goes anywhere, the bar-specific controls
    /// only where they work.
    pub fn fits(self, zone: Zone) -> bool {
        if self.navigates() {
            return true;
        }
        match self {
            Self::Library => zone == Zone::Sidebar,
            Self::SidebarToggle
            | Self::Refresh
            | Self::Downloads
            | Self::PageTools
            | Self::Account
            | Self::Spacer => zone == Zone::TitleBar,
            Self::Back | Self::Forward | Self::Panel => zone != Zone::Sidebar,
            Self::Sessions => zone == Zone::PlayerBar,
            _ => true,
        }
    }

    /// Whether the tool can move but never leave its bar. The account menu is the way back
    /// into this editor and to Settings, and the page tools are how a screen is sorted.
    pub fn anchored(self) -> bool {
        matches!(self, Self::Account | Self::PageTools)
    }

    /// Whether a bar can hold more than one of the tool.
    pub fn repeats(self) -> bool {
        matches!(self, Self::Spacer | Self::Separator)
    }
}

/// The tools of each bar in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panels {
    title_bar: Vec<Tool>,
    sidebar: Vec<Tool>,
    player_bar: Vec<Tool>,
}

/// [`Panels`] as `settings.json` keeps it, by tool id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredPanels {
    title_bar: Vec<String>,
    sidebar: Vec<String>,
    player_bar: Vec<String>,
}

impl Default for Panels {
    fn default() -> Self {
        use Tool::*;
        Self {
            title_bar: vec![
                SidebarToggle,
                Back,
                Forward,
                Refresh,
                Spacer,
                Downloads,
                PageTools,
                Separator,
                Account,
                Panel,
            ],
            sidebar: vec![
                Home, Search, History, Library, Songs, Albums, Artists, Playlists,
            ],
            player_bar: vec![Sessions, Lyrics, Queue],
        }
    }
}

impl Panels {
    /// The arrangement an older Sonora implies: the default bars without the sidebar rows the
    /// listener hid, with the optional rows they added, and without the Sessions button when
    /// they turned it off.
    pub(crate) fn seeded(hidden: &[String], added: &[String], sessions: bool) -> Self {
        let mut panels = Self::default();
        panels
            .sidebar
            .retain(|tool| !hidden.iter().any(|id| id == tool.id()));
        if added.iter().any(|id| id == Tool::Settings.id()) {
            let at = panels.sidebar_slot(Tool::Settings);
            panels.sidebar.insert(at, Tool::Settings);
        }
        if !panels.sidebar.iter().any(|tool| is_library(*tool)) {
            panels.sidebar.retain(|tool| *tool != Tool::Library);
        }
        if !sessions {
            panels.player_bar.retain(|tool| *tool != Tool::Sessions);
        }
        panels
    }

    /// Reads a stored arrangement leniently: an unknown id or a tool in a bar that cannot hold
    /// it is dropped, a second copy of a tool that cannot repeat is dropped, and an anchored
    /// tool that went missing comes back to its default bar.
    pub(crate) fn read(stored: &StoredPanels) -> Self {
        let mut panels = Self {
            title_bar: Vec::new(),
            sidebar: Vec::new(),
            player_bar: Vec::new(),
        };
        for zone in Zone::ALL {
            let ids = match zone {
                Zone::TitleBar => &stored.title_bar,
                Zone::Sidebar => &stored.sidebar,
                Zone::PlayerBar => &stored.player_bar,
            };
            for tool in ids.iter().filter_map(|id| Tool::from_id(id)) {
                if tool.fits(zone) && (tool.repeats() || panels.zone_of(tool).is_none()) {
                    panels.zone_mut(zone).push(tool);
                }
            }
        }
        let defaults = Self::default();
        for tool in Tool::ALL.into_iter().filter(|tool| tool.anchored()) {
            if panels.zone_of(tool).is_none()
                && let Some(zone) = defaults.zone_of(tool)
            {
                panels.zone_mut(zone).push(tool);
            }
        }
        panels
    }

    pub(crate) fn stored(&self) -> StoredPanels {
        let ids = |tools: &[Tool]| tools.iter().map(|tool| tool.id().to_owned()).collect();
        StoredPanels {
            title_bar: ids(&self.title_bar),
            sidebar: ids(&self.sidebar),
            player_bar: ids(&self.player_bar),
        }
    }

    pub fn zone(&self, zone: Zone) -> &[Tool] {
        match zone {
            Zone::TitleBar => &self.title_bar,
            Zone::Sidebar => &self.sidebar,
            Zone::PlayerBar => &self.player_bar,
        }
    }

    fn zone_mut(&mut self, zone: Zone) -> &mut Vec<Tool> {
        match zone {
            Zone::TitleBar => &mut self.title_bar,
            Zone::Sidebar => &mut self.sidebar,
            Zone::PlayerBar => &mut self.player_bar,
        }
    }

    /// The bar a tool that cannot repeat stands in, `None` while it waits in the palette.
    pub fn zone_of(&self, tool: Tool) -> Option<Zone> {
        Zone::ALL
            .into_iter()
            .find(|zone| self.zone(*zone).contains(&tool))
    }

    /// The tools the palette offers: every tool in no bar, then the spacer and the separator,
    /// which are always on offer.
    pub fn unplaced(&self) -> Vec<Tool> {
        Tool::ALL
            .into_iter()
            .filter(|tool| tool.repeats() || self.zone_of(*tool).is_none())
            .collect()
    }

    /// Puts `tool` into `zone` before the tool at `gap`, `gap` being the bar's length for its
    /// end. `from` is where a dragged tool came from, which it leaves. Answers whether anything
    /// changed. A tool that cannot repeat and is already in another bar moves out of it.
    pub fn place(
        &mut self,
        tool: Tool,
        from: Option<(Zone, usize)>,
        zone: Zone,
        gap: usize,
    ) -> bool {
        if !tool.fits(zone) {
            return false;
        }
        let mut gap = gap.min(self.zone(zone).len());
        let from = from
            .filter(|(zone, index)| self.zone(*zone).get(*index) == Some(&tool))
            .or_else(|| match tool.repeats() {
                true => None,
                false => self.zone_of(tool).and_then(|zone| {
                    let index = self.zone(zone).iter().position(|held| *held == tool)?;
                    Some((zone, index))
                }),
            });
        if let Some((left, index)) = from {
            if left == zone && (gap == index || gap == index + 1) {
                return false;
            }
            self.zone_mut(left).remove(index);
            if left == zone && gap > index {
                gap -= 1;
            }
        }
        self.zone_mut(zone).insert(gap, tool);
        true
    }

    /// Takes the tool at `index` out of `zone`, unless it is anchored there.
    pub fn remove(&mut self, zone: Zone, index: usize) -> bool {
        match self.zone(zone).get(index) {
            Some(tool) if !tool.anchored() => {
                self.zone_mut(zone).remove(index);
                true
            }
            _ => false,
        }
    }

    /// Adds `tool` to the first bar that holds it, in the place it has in the default
    /// arrangement. Answers the bar, or `None` when the tool is already placed and cannot repeat.
    pub fn add(&mut self, tool: Tool) -> Option<Zone> {
        if !tool.repeats() && self.zone_of(tool).is_some() {
            return None;
        }
        // Settings is in no default bar. The sidebar entries setting used to add it to the sidebar.
        let usual = match tool {
            Tool::Settings => Some(Zone::Sidebar),
            tool => Self::default().zone_of(tool),
        };
        let zone = usual
            .filter(|zone| tool.fits(*zone))
            .or_else(|| Zone::ALL.into_iter().find(|zone| tool.fits(*zone)))?;
        let at = match zone {
            Zone::Sidebar => self.sidebar_slot(tool),
            _ => self.zone(zone).len(),
        };
        self.zone_mut(zone).insert(at, tool);
        Some(zone)
    }

    /// Whether the sidebar shows `tool`.
    pub fn in_sidebar(&self, tool: Tool) -> bool {
        self.sidebar.contains(&tool)
    }

    /// Shows or hides `tool` among the sidebar's rows, the way the sidebar entries setting does.
    /// A shown tool leaves any other bar for the place it has in the default sidebar.
    pub fn set_in_sidebar(&mut self, tool: Tool, shown: bool) {
        if !tool.fits(Zone::Sidebar) || self.in_sidebar(tool) == shown {
            return;
        }
        if !shown {
            self.sidebar.retain(|held| *held != tool);
            return;
        }
        if let Some(zone) = self.zone_of(tool) {
            self.zone_mut(zone).retain(|held| *held != tool);
        }
        if is_library(tool) && !self.sidebar.contains(&Tool::Library) {
            let at = self.sidebar_slot(Tool::Library);
            self.sidebar.insert(at, Tool::Library);
        }
        let at = self.sidebar_slot(tool);
        self.sidebar.insert(at, tool);
    }

    /// Shows the tool in `zone`'s default place when `shown` and it is in no bar, and takes it
    /// out of whichever bar holds it otherwise.
    pub fn set_shown(&mut self, tool: Tool, shown: bool) {
        match (shown, self.zone_of(tool)) {
            (true, None) => {
                self.add(tool);
            }
            (false, Some(zone)) => self.zone_mut(zone).retain(|held| *held != tool),
            _ => {}
        }
    }

    /// Where `tool` goes in the sidebar: after the last of the rows that come before it in the
    /// default sidebar.
    fn sidebar_slot(&self, tool: Tool) -> usize {
        let order: Vec<Tool> = {
            let mut order = Self::default().sidebar;
            let at = order.iter().position(|held| *held == Tool::Library);
            if let Some(at) = at {
                order.insert(at, Tool::Settings);
            }
            order
        };
        let Some(rank) = order.iter().position(|held| *held == tool) else {
            return self.sidebar.len();
        };
        self.sidebar
            .iter()
            .rposition(|held| {
                order
                    .iter()
                    .position(|known| known == held)
                    .is_some_and(|known| known < rank)
            })
            .map_or(0, |index| index + 1)
    }
}

fn is_library(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Songs | Tool::Albums | Tool::Artists | Tool::Playlists
    )
}
