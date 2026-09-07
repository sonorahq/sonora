//! User preferences and runtime state
//!
//! Preferences a user sets on purpose go in `settings.json` as [`Values`]. Everything the app
//! changes on its own while running, such as the window frame or the volume, goes in
//! `state.sqlite` as [`StateValues`]. [`AppSettings`] holds both and saves each on its own
//! debounce, so a sidebar drag never rewrites the preferences file. A pre-v2 `settings.json`
//! carried both kinds of value. The first start after the split moves the runtime half into
//! SQLite and rewrites the JSON without it.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui::WindowDecorations;
use gpui::{
    App, Bounds, Context, Pixels, Size, Subscription, Task, Window, WindowBounds, point, px, size,
};
use music::WritingSystem;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use storage::Database;
use ui::{
    Layout, Look, Mode, Pace, Pin, Rounding, Saver, Sorting, Stillness, ThemeKind, ThemeOverrides,
};

use crate::queue::{Resume, gap_target};
use crate::{Repeat, SeekStep, Sonora};

/// Which panel the right sidebar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideTab {
    #[default]
    Queue,
    Lyrics,
}

/// The writing systems lyrics romanization applies to. Only CJK are enabled by default.
/// A partial object in `settings.json` keeps the defaults for the scripts it leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RomanizationScripts {
    japanese: bool,
    chinese: bool,
    korean: bool,
    cyrillic: bool,
    greek: bool,
    arabic: bool,
    other: bool,
}

impl RomanizationScripts {
    pub fn contains(self, writing_system: WritingSystem) -> bool {
        match writing_system {
            WritingSystem::Japanese => self.japanese,
            WritingSystem::Chinese => self.chinese,
            WritingSystem::Korean => self.korean,
            WritingSystem::Cyrillic => self.cyrillic,
            WritingSystem::Greek => self.greek,
            WritingSystem::Arabic => self.arabic,
            WritingSystem::Other => self.other,
        }
    }

    fn set(&mut self, writing_system: WritingSystem, enabled: bool) {
        match writing_system {
            WritingSystem::Japanese => self.japanese = enabled,
            WritingSystem::Chinese => self.chinese = enabled,
            WritingSystem::Korean => self.korean = enabled,
            WritingSystem::Cyrillic => self.cyrillic = enabled,
            WritingSystem::Greek => self.greek = enabled,
            WritingSystem::Arabic => self.arabic = enabled,
            WritingSystem::Other => self.other = enabled,
        }
    }
}

impl Default for RomanizationScripts {
    fn default() -> Self {
        Self {
            japanese: true,
            chinese: true,
            korean: true,
            cyrillic: false,
            greek: false,
            arabic: false,
            other: false,
        }
    }
}

/// A window's saved position and size in logical pixels, plus whether it was maximized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Frame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    maximized: bool,
}

impl Frame {
    fn of(window: &Window) -> Self {
        let placement = window.window_bounds();
        let bounds = placement.get_bounds();
        Self {
            x: bounds.origin.x / px(1.),
            y: bounds.origin.y / px(1.),
            width: bounds.size.width / px(1.),
            height: bounds.size.height / px(1.),
            maximized: matches!(placement, WindowBounds::Maximized(_)),
        }
    }

    /// Rejects what a corrupt write could produce: a non-finite coordinate or an empty size.
    fn sane(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|it| it.is_finite())
            && self.width > 0.
            && self.height > 0.
    }

    /// Turns the frame back into a gpui placement, never smaller than `least`.
    fn placement(self, least: Size<Pixels>) -> WindowBounds {
        let bounds = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(
                px(self.width).max(least.width),
                px(self.height).max(least.height),
            ),
        };
        match self.maximized {
            true => WindowBounds::Maximized(bounds),
            false => WindowBounds::Windowed(bounds),
        }
    }
}

fn system_font() -> String {
    SYSTEM_FONT.to_owned()
}

/// How long a save waits after the last change, so a slider drag lands as one write.
const SAVE_DELAY: Duration = Duration::from_millis(300);
const DEFAULT_VOLUME: f32 = 0.7;
const DEFAULT_SIDEBAR_WIDTH: f32 = 195.;
const DEFAULT_SIDEBAR_RIGHT_WIDTH: f32 = 254.;
const DEFAULT_FONT_SIZE: f32 = 14.;
const DEFAULT_LYRICS_SCALE: f32 = 1.;
const DEFAULT_STARTUP: &str = "home";
/// The shape of `settings.json`. For example, v2 moved runtime state out into `state.sqlite`.
const SETTINGS_VERSION: u32 = 2;

/// "Whatever the platform uses".
pub const SYSTEM_FONT: &str = "auto";

/// Pins grouped by provider slug, deprecated since v2.
type Groups = HashMap<String, Vec<Pin>>;

/// A pin together with the provider it belongs to. The pin is flattened so the stored JSON
/// reads as a pin with one extra `slug` key.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Held {
    slug: String,
    #[serde(flatten)]
    pin: Pin,
}

/// Everything `settings.json` holds. Preferences a user sets on purpose, safe to edit by hand.
/// Missing keys take their defaults, unknown keys are ignored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Values {
    version: u32,
    normalisation: bool,
    gapless: bool,
    seek_step: u16,
    lyrics_for_local_files: bool,
    karaoke_lyrics: bool,
    blur_lyrics: bool,
    romanized_lyrics: bool,
    panel_lyrics_scale: f32,
    fullscreen_lyrics_scale: f32,
    romanization_scripts: RomanizationScripts,
    adaptive_menu: bool,
    check_updates: bool,
    close_to_tray: bool,
    language: String,
    #[serde(default = "system_font")]
    font: String,
    startup: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_folder: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hidden_nav: Vec<String>,
    appearance: Appearance,
}

/// The `appearance` block of `settings.json`. Fixed choices are stored by id, removed variant
/// falls back to the default instead of failing the parse.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Appearance {
    theme: String,
    adaptive_theme: bool,
    visualizer: bool,
    icons: String,
    rounding: String,
    font_size: f32,
    transparent: bool,
    transparency: f32,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    server_side_decorations: bool,
    window_controls: bool,
    controls_on_left: bool,
    reduce_motion: String,
    motion_pace: String,
    battery_saver: String,
    theme_overrides: ThemeOverrides,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            normalisation: false,
            gapless: true,
            seek_step: SeekStep::default().secs(),
            lyrics_for_local_files: true,
            karaoke_lyrics: true,
            blur_lyrics: true,
            romanized_lyrics: true,
            panel_lyrics_scale: DEFAULT_LYRICS_SCALE,
            fullscreen_lyrics_scale: DEFAULT_LYRICS_SCALE,
            romanization_scripts: RomanizationScripts::default(),
            adaptive_menu: false,
            check_updates: cfg!(target_os = "windows"),
            close_to_tray: true,
            language: i18n::AUTO.to_owned(),
            font: system_font(),
            startup: DEFAULT_STARTUP.to_owned(),
            local_folder: None,
            hidden_nav: Vec::new(),
            appearance: Appearance::default(),
        }
    }
}

/// Everything `state.sqlite` holds under the `runtime` key: values the app changes on its own
/// while running, so saving them never rewrites `settings.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct StateValues {
    volume: f32,
    sidebar_width: f32,
    sidebar_open: bool,
    sidebar_right_width: f32,
    sidebar_right_open: bool,
    sidebar_right_tab: SideTab,
    shuffle: bool,
    repeat: Repeat,
    radio: bool,
    provider: String,
    tables: HashMap<String, Layout>,
    sorting: HashMap<String, Option<Sorting>>,
    views: HashMap<String, Mode>,
    pinned: Vec<Held>,
    resume: Option<Resume>,
    window: Option<Frame>,
    system_theme: String,
}

impl Default for StateValues {
    fn default() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_open: true,
            sidebar_right_width: DEFAULT_SIDEBAR_RIGHT_WIDTH,
            sidebar_right_open: false,
            sidebar_right_tab: SideTab::Queue,
            shuffle: false,
            repeat: Repeat::Off,
            radio: false,
            provider: "spotify".to_owned(),
            tables: HashMap::new(),
            sorting: HashMap::new(),
            views: HashMap::new(),
            pinned: Vec::new(),
            resume: None,
            window: None,
            system_theme: ThemeKind::Dark.id().to_owned(),
        }
    }
}

/// The runtime keys a pre-v2 `settings.json` stored in a shape [`StateValues`] no longer has.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Legacy {
    hidden_columns: HashMap<String, Vec<String>>,
    pins: Groups,
    appearance: LegacyAppearance,
}

/// `system_theme` used to live inside the `appearance` block.
#[derive(Default, Deserialize)]
#[serde(default)]
struct LegacyAppearance {
    system_theme: Option<String>,
}

impl StateValues {
    /// Reads runtime state out of a pre-v2 `settings.json`. Runs once, while `state.sqlite` has no
    /// runtime row yet. An unparsable file yields the defaults.
    fn inherited(bytes: &[u8]) -> Self {
        let mut state: Self = serde_json::from_slice(bytes).unwrap_or_default();
        let legacy: Legacy = serde_json::from_slice(bytes).unwrap_or_default();
        state.adopt(legacy);
        state
    }

    /// Hidden column lists become table layouts, pin groups flatten into one list with the active
    /// provider's pins first, and the nested system theme moves to the top level.
    fn adopt(&mut self, legacy: Legacy) {
        for (table, hidden) in legacy.hidden_columns {
            self.tables.entry(table).or_insert_with(|| Layout {
                hidden,
                ..Layout::default()
            });
        }

        let mut pins = legacy.pins;
        let mut slugs: Vec<String> = pins.keys().cloned().collect();
        slugs.sort_by_key(|slug| (*slug != self.provider, slug.clone()));
        for slug in slugs {
            let Some(group) = pins.remove(&slug) else {
                continue;
            };
            self.pinned.extend(group.into_iter().map(|pin| Held {
                slug: slug.clone(),
                pin,
            }));
        }

        if let Some(system_theme) = legacy.appearance.system_theme {
            self.system_theme = system_theme;
        }
    }
}

/// The `runtime` row of the `app_state` table: one JSON blob, replaced whole on every save.
#[derive(Clone)]
struct StateStore {
    database: Database,
}

impl StateStore {
    fn new(database: Database) -> Self {
        Self { database }
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        self.database.open()
    }

    fn load(&self) -> Result<Option<StateValues>> {
        let encoded: Option<String> = self
            .open()?
            .query_row(
                "SELECT value FROM app_state WHERE key = 'runtime'",
                [],
                |row| row.get(0),
            )
            .optional()
            .context("cannot read app state")?;
        encoded
            .map(|encoded| serde_json::from_str(&encoded).context("cannot decode app state"))
            .transpose()
    }

    fn save(&self, state: &StateValues) -> Result<()> {
        let encoded = serde_json::to_string(state).context("cannot encode app state")?;
        self.open()?
            .execute(
                "INSERT INTO app_state (key, value) VALUES ('runtime', ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![encoded],
            )
            .context("cannot save app state")?;
        Ok(())
    }

    /// Writes freshly migrated state and reports whether it landed, so a failed write keeps the
    /// legacy files for the next start.
    fn adopt(&self, state: StateValues) -> (StateValues, bool) {
        let ready = match self.save(&state) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("settings: cannot migrate app state: {error:#}");
                false
            }
        };
        (state, ready)
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "dark".to_owned(),
            adaptive_theme: true,
            visualizer: true,
            icons: icons::BASE.to_owned(),
            rounding: Rounding::Rounded.id().to_owned(),
            font_size: DEFAULT_FONT_SIZE,
            transparent: false,
            transparency: 0.15,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            server_side_decorations: true,
            window_controls: true,
            controls_on_left: false,
            reduce_motion: Stillness::default().id().to_owned(),
            motion_pace: Pace::default().id().to_owned(),
            battery_saver: Saver::default().id().to_owned(),
            theme_overrides: ThemeOverrides::default(),
        }
    }
}

/// Preferences from `settings.json` and runtime state from `state.sqlite`, each saved on its own
/// debounce. `writable` is false when the JSON exists but cannot be parsed, so a broken file is
/// never overwritten with defaults.
pub struct AppSettings {
    values: Values,
    state: StateValues,
    path: PathBuf,
    store: StateStore,
    save: Option<Task<()>>,
    save_state: Option<Task<()>>,
    watch: Option<Subscription>,
    writable: bool,
}

impl AppSettings {
    /// Loads from the standard config and data paths.
    pub fn load(database: Database) -> Self {
        Self::load_from(
            settings_path(),
            StateStore::new(database),
            legacy_local_path(),
        )
    }

    /// State already in SQLite wins. Otherwise the runtime half of a pre-v2 `settings.json` and the
    /// folder from `local-music.json` are migrated, and the legacy files are only rewritten or
    /// removed once the SQLite write has succeeded.
    fn load_from(path: PathBuf, store: StateStore, legacy_local_path: PathBuf) -> Self {
        let (bytes, existed, writable) = match fs::read(&path) {
            Ok(bytes) => (Some(bytes), true, true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, false, true),
            Err(error) => {
                log::warn!("settings: cannot read {}: {error}", path.display());
                (None, true, false)
            }
        };
        let (mut values, writable) = match bytes.as_deref().map(serde_json::from_slice::<Values>) {
            Some(Ok(values)) => (values, writable),
            Some(Err(error)) => {
                log::warn!("settings: cannot parse {}: {error}", path.display());
                (Values::default(), false)
            }
            None => (Values::default(), writable),
        };
        let old_version = bytes.as_deref().map_or(SETTINGS_VERSION, settings_version);
        let legacy_local = legacy_local_folder(&legacy_local_path);
        if values.local_folder.is_none() {
            values.local_folder = legacy_local.clone();
        }

        let inherited = || {
            bytes
                .as_deref()
                .map(StateValues::inherited)
                .unwrap_or_default()
        };
        let (state, state_ready) = match store.load() {
            Ok(Some(saved)) => (saved, true),
            Ok(None) => store.adopt(inherited()),
            Err(error) => {
                log::warn!("settings: cannot load app state: {error:#}");
                store.adopt(inherited())
            }
        };
        values.version = SETTINGS_VERSION;

        let settings = Self {
            values,
            state,
            path,
            store,
            save: None,
            save_state: None,
            watch: None,
            writable,
        };
        let cleanup = existed && old_version < SETTINGS_VERSION || legacy_local.is_some();
        if cleanup && state_ready && settings.save_now() {
            remove_legacy_local_folder(&legacy_local_path);
        }
        settings
    }

    pub fn volume(&self) -> f32 {
        self.state.volume.clamp(0., 1.)
    }

    pub fn normalisation(&self) -> bool {
        self.values.normalisation
    }

    pub fn gapless(&self) -> bool {
        self.values.gapless
    }

    pub fn seek_step(&self) -> SeekStep {
        SeekStep::from_secs(self.values.seek_step)
    }

    pub fn lyrics_for_local_files(&self) -> bool {
        self.values.lyrics_for_local_files
    }

    pub fn karaoke_lyrics(&self) -> bool {
        self.values.karaoke_lyrics
    }

    pub fn blur_lyrics(&self) -> bool {
        self.values.blur_lyrics
    }

    pub fn romanized_lyrics(&self) -> bool {
        self.values.romanized_lyrics
    }

    pub fn panel_lyrics_scale(&self) -> f32 {
        self.values
            .panel_lyrics_scale
            .clamp(ui::MIN_LYRICS_SCALE, ui::MAX_LYRICS_SCALE)
    }

    pub fn fullscreen_lyrics_scale(&self) -> f32 {
        self.values
            .fullscreen_lyrics_scale
            .clamp(ui::MIN_LYRICS_SCALE, ui::MAX_LYRICS_SCALE)
    }

    pub fn romanization_scripts(&self) -> RomanizationScripts {
        self.values.romanization_scripts
    }

    pub fn adaptive_menu(&self) -> bool {
        self.values.adaptive_menu
    }

    pub fn check_updates(&self) -> bool {
        self.values.check_updates
    }

    pub fn close_to_tray(&self) -> bool {
        self.values.close_to_tray
    }

    pub fn sidebar_width(&self) -> f32 {
        self.state.sidebar_width
    }

    pub fn sidebar_open(&self) -> bool {
        self.state.sidebar_open
    }

    pub fn sidebar_right_width(&self) -> f32 {
        self.state.sidebar_right_width
    }

    pub fn sidebar_right_open(&self) -> bool {
        self.state.sidebar_right_open
    }

    pub fn sidebar_right_tab(&self) -> SideTab {
        self.state.sidebar_right_tab
    }

    pub fn shuffle(&self) -> bool {
        self.state.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.state.repeat
    }

    pub fn radio(&self) -> bool {
        self.state.radio
    }

    pub fn language(&self) -> &str {
        &self.values.language
    }

    pub fn font(&self) -> &str {
        &self.values.font
    }

    pub fn provider(&self) -> &str {
        &self.state.provider
    }

    pub fn local_folder(&self) -> Option<&std::path::Path> {
        self.values.local_folder.as_deref()
    }

    pub fn startup(&self) -> &str {
        &self.values.startup
    }

    pub fn theme(&self) -> &str {
        &self.values.appearance.theme
    }

    pub fn adaptive_theme(&self) -> bool {
        self.values.appearance.adaptive_theme
    }

    pub fn visualizer(&self) -> bool {
        self.values.appearance.visualizer
    }

    pub fn icons(&self) -> &str {
        &self.values.appearance.icons
    }

    pub fn rounding(&self) -> &str {
        &self.values.appearance.rounding
    }

    pub fn stillness(&self) -> Stillness {
        Stillness::from_id(&self.values.appearance.reduce_motion)
    }

    pub fn pace(&self) -> Pace {
        Pace::from_id(&self.values.appearance.motion_pace)
    }

    pub fn saver(&self) -> Saver {
        Saver::from_id(&self.values.appearance.battery_saver)
    }

    pub fn system_theme(&self) -> ThemeKind {
        ThemeKind::from_id(&self.state.system_theme)
    }

    pub fn look(&self) -> Look {
        Look {
            kind: ThemeKind::from_id(self.theme()),
            rounding: Rounding::from_id(self.rounding()),
            font: self.font_size(),
            transparent: self.transparent(),
            transparency: self.transparency(),
            tint: None,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn server_side_decorations(&self) -> bool {
        self.values.appearance.server_side_decorations
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn window_decorations(&self) -> WindowDecorations {
        match self.server_side_decorations() {
            true => WindowDecorations::Server,
            false => WindowDecorations::Client,
        }
    }

    pub fn window_controls(&self) -> bool {
        self.values.appearance.window_controls
    }

    pub fn controls_on_left(&self) -> bool {
        self.values.appearance.controls_on_left
    }

    pub fn font_size(&self) -> f32 {
        self.values
            .appearance
            .font_size
            .clamp(ui::MIN_FONT, ui::MAX_FONT)
    }

    pub fn transparent(&self) -> bool {
        self.values.appearance.transparent
    }

    pub fn transparency(&self) -> f32 {
        self.values
            .appearance
            .transparency
            .clamp(0., ui::MAX_TRANSPARENCY)
    }

    pub fn theme_overrides(&self) -> &ThemeOverrides {
        &self.values.appearance.theme_overrides
    }

    /// The path of `settings.json`, written first if it does not exist yet.
    pub fn ensure_file(&self) -> PathBuf {
        if !self.path.exists() {
            self.save_now();
        }
        self.path.clone()
    }

    pub fn set_local_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.values.local_folder == folder {
            return;
        }
        self.values.local_folder = folder;
        self.schedule_save(cx);
    }

    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.state.volume = volume.clamp(0., 1.);
        self.schedule_state_save(cx);
    }

    pub fn set_normalisation(&mut self, normalisation: bool, cx: &mut Context<Self>) {
        self.values.normalisation = normalisation;
        self.schedule_save(cx);
    }

    pub fn set_gapless(&mut self, gapless: bool, cx: &mut Context<Self>) {
        self.values.gapless = gapless;
        self.schedule_save(cx);
    }

    pub fn set_seek_step(&mut self, step: SeekStep, cx: &mut Context<Self>) {
        if self.seek_step() == step {
            return;
        }
        self.values.seek_step = step.secs();
        self.schedule_save(cx);
    }

    pub fn set_lyrics_for_local_files(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.values.lyrics_for_local_files = enabled;
        self.schedule_save(cx);
    }

    pub fn set_karaoke_lyrics(&mut self, karaoke: bool, cx: &mut Context<Self>) {
        self.values.karaoke_lyrics = karaoke;
        self.schedule_save(cx);
    }

    pub fn set_blur_lyrics(&mut self, blur: bool, cx: &mut Context<Self>) {
        self.values.blur_lyrics = blur;
        self.schedule_save(cx);
    }

    pub fn set_romanized_lyrics(&mut self, romanized: bool, cx: &mut Context<Self>) {
        self.values.romanized_lyrics = romanized;
        self.schedule_save(cx);
    }

    pub fn set_panel_lyrics_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.values.panel_lyrics_scale = scale.clamp(ui::MIN_LYRICS_SCALE, ui::MAX_LYRICS_SCALE);
        self.schedule_save(cx);
    }

    pub fn set_fullscreen_lyrics_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.values.fullscreen_lyrics_scale =
            scale.clamp(ui::MIN_LYRICS_SCALE, ui::MAX_LYRICS_SCALE);
        self.schedule_save(cx);
    }

    pub fn set_romanization_script(
        &mut self,
        writing_system: WritingSystem,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.values
            .romanization_scripts
            .set(writing_system, enabled);
        self.schedule_save(cx);
    }

    pub fn set_adaptive_menu(&mut self, adaptive_menu: bool, cx: &mut Context<Self>) {
        self.values.adaptive_menu = adaptive_menu;
        self.schedule_save(cx);
    }

    pub fn set_check_updates(&mut self, check_updates: bool, cx: &mut Context<Self>) {
        self.values.check_updates = check_updates;
        self.schedule_save(cx);
    }

    pub fn set_close_to_tray(&mut self, close_to_tray: bool, cx: &mut Context<Self>) {
        self.values.close_to_tray = close_to_tray;
        self.schedule_save(cx);
    }

    pub fn table(&self, table: &str) -> Layout {
        self.state.tables.get(table).cloned().unwrap_or_default()
    }

    pub fn set_table(&mut self, table: &str, layout: Layout, cx: &mut Context<Self>) {
        if self.state.tables.get(table) == Some(&layout) {
            return;
        }
        self.state.tables.insert(table.to_owned(), layout);
        self.schedule_state_save(cx);
    }

    pub fn view_or(&self, table: &str, fallback: Mode) -> Mode {
        self.state.views.get(table).copied().unwrap_or(fallback)
    }

    pub fn set_view(&mut self, table: &str, mode: Mode, cx: &mut Context<Self>) {
        if self.state.views.get(table) == Some(&mode) {
            return;
        }
        self.state.views.insert(table.to_owned(), mode);
        self.schedule_state_save(cx);
    }

    pub fn sorting(&self, table: &str) -> Option<Option<Sorting>> {
        self.state.sorting.get(table).cloned()
    }

    pub fn set_sorting(&mut self, table: &str, sorting: Option<Sorting>, cx: &mut Context<Self>) {
        if self.state.sorting.get(table) == Some(&sorting) {
            return;
        }
        self.state.sorting.insert(table.to_owned(), sorting);
        self.schedule_state_save(cx);
    }

    pub fn pinned(&self, slugs: &[&str]) -> Vec<Pin> {
        gather(&self.state.pinned, slugs)
    }

    pub fn resume(&self) -> Option<&Resume> {
        self.state.resume.as_ref()
    }

    /// Records where playback picks up on the next launch. See `carry` for what survives.
    pub fn set_resume(&mut self, resume: Option<Resume>, cx: &mut Context<Self>) {
        let mut resume = resume;
        if let Some(next) = resume.as_mut() {
            carry(self.state.resume.as_ref(), next);
        }
        if self.state.resume == resume {
            return;
        }
        self.state.resume = resume;
        self.schedule_state_save(cx);
    }

    /// Saves quietly. Nothing renders from the stored copy, the live queue is the source of truth.
    pub fn set_resume_origin(&mut self, origin: Option<crate::Origin>, cx: &mut Context<Self>) {
        let Some(resume) = self.state.resume.as_mut() else {
            return;
        };
        if resume.origin == origin {
            return;
        }
        resume.origin = origin;
        self.save_state_quietly(cx);
    }

    /// Saves quietly, since playback calls this on every position tick.
    pub fn set_resume_position(&mut self, position: f32, cx: &mut Context<Self>) {
        let Some(resume) = self.state.resume.as_mut() else {
            return;
        };
        if resume.position == position {
            return;
        }
        resume.position = position;
        self.save_state_quietly(cx);
    }

    /// Pins at `gap`, a slot counted among the pins of the providers in `slugs`. An existing pin
    /// moves instead of duplicating, and nothing is saved when it already sits there.
    pub fn pin(
        &mut self,
        slug: &str,
        pin: Pin,
        gap: Option<usize>,
        slugs: &[&str],
        cx: &mut Context<Self>,
    ) {
        if !place(&mut self.state.pinned, slug, pin, gap, slugs) {
            return;
        }
        self.schedule_state_save(cx);
    }

    pub fn unpin(&mut self, slug: &str, pin: &Pin, cx: &mut Context<Self>) {
        if !take(&mut self.state.pinned, slug, pin) {
            return;
        }
        self.schedule_state_save(cx);
    }

    pub fn set_sidebar(&mut self, width: f32, open: bool, cx: &mut Context<Self>) {
        self.state.sidebar_width = width;
        self.state.sidebar_open = open;
        self.schedule_state_save(cx);
    }

    pub fn set_sidebar_right_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.state.sidebar_right_width = width;
        self.schedule_state_save(cx);
    }

    pub fn set_sidebar_right_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.state.sidebar_right_open = open;
        self.schedule_state_save(cx);
    }

    pub fn set_sidebar_right_tab(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if self.state.sidebar_right_tab == tab {
            return;
        }
        self.state.sidebar_right_tab = tab;
        self.schedule_state_save(cx);
    }

    pub fn set_shuffle(&mut self, shuffle: bool, cx: &mut Context<Self>) {
        self.state.shuffle = shuffle;
        self.schedule_state_save(cx);
    }

    pub fn set_repeat(&mut self, repeat: Repeat, cx: &mut Context<Self>) {
        self.state.repeat = repeat;
        self.schedule_state_save(cx);
    }

    pub fn set_radio(&mut self, radio: bool, cx: &mut Context<Self>) {
        self.state.radio = radio;
        self.schedule_state_save(cx);
    }

    pub fn set_language(&mut self, language: impl Into<String>, cx: &mut Context<Self>) {
        self.values.language = language.into();
        i18n::set(i18n::resolve(&self.values.language));
        cx.refresh_windows();
        self.schedule_save(cx);
    }

    pub fn set_font(&mut self, font: impl Into<String>, cx: &mut Context<Self>) {
        let font = font.into();
        if self.values.font == font {
            return;
        }
        self.values.font = font;
        cx.refresh_windows();
        self.schedule_save(cx);
    }

    pub fn nav_shown(&self, entry: &str) -> bool {
        !self.values.hidden_nav.iter().any(|hidden| hidden == entry)
    }

    pub fn set_nav_shown(&mut self, entry: &str, shown: bool, cx: &mut Context<Self>) {
        if self.nav_shown(entry) == shown {
            return;
        }
        match shown {
            true => self.values.hidden_nav.retain(|hidden| hidden != entry),
            false => self.values.hidden_nav.push(entry.to_owned()),
        }
        self.schedule_save(cx);
    }

    pub fn set_startup(&mut self, screen: impl Into<String>, cx: &mut Context<Self>) {
        let screen = screen.into();
        if self.values.startup == screen {
            return;
        }
        self.values.startup = screen;
        self.schedule_save(cx);
    }

    pub fn set_provider(&mut self, provider: impl Into<String>, cx: &mut Context<Self>) {
        let provider = provider.into();
        if self.state.provider == provider {
            return;
        }
        self.state.provider = provider;
        self.schedule_state_save(cx);
    }

    pub fn set_theme(&mut self, theme: impl Into<String>, cx: &mut Context<Self>) {
        self.values.appearance.theme = theme.into();
        self.schedule_save(cx);
    }

    pub fn set_adaptive_theme(&mut self, adaptive: bool, cx: &mut Context<Self>) {
        self.values.appearance.adaptive_theme = adaptive;
        self.schedule_save(cx);
    }

    pub fn set_visualizer(&mut self, visualizer: bool, cx: &mut Context<Self>) {
        self.values.appearance.visualizer = visualizer;
        self.schedule_save(cx);
    }

    pub fn set_icons(&mut self, pack: impl Into<String>, cx: &mut Context<Self>) {
        let pack = pack.into();
        if self.values.appearance.icons == pack {
            return;
        }
        icons::set(&pack);
        self.values.appearance.icons = pack;
        cx.refresh_windows();
        self.schedule_save(cx);
    }

    pub fn set_rounding(&mut self, rounding: impl Into<String>, cx: &mut Context<Self>) {
        self.values.appearance.rounding = rounding.into();
        self.schedule_save(cx);
    }

    pub fn set_stillness(&mut self, stillness: Stillness, cx: &mut Context<Self>) {
        if self.stillness() == stillness {
            return;
        }
        self.values.appearance.reduce_motion = stillness.id().to_owned();
        ui::motion::apply(stillness, self.pace(), cx);
        self.schedule_save(cx);
    }

    pub fn set_pace(&mut self, pace: Pace, cx: &mut Context<Self>) {
        if self.pace() == pace {
            return;
        }
        self.values.appearance.motion_pace = pace.id().to_owned();
        ui::motion::apply(self.stillness(), pace, cx);
        self.schedule_save(cx);
    }

    pub fn set_system_theme(&mut self, kind: ThemeKind, cx: &mut Context<Self>) {
        if self.system_theme() == kind {
            return;
        }
        self.state.system_theme = kind.id().to_owned();
        self.schedule_state_save(cx);
    }

    pub fn set_saver(&mut self, saver: Saver, cx: &mut Context<Self>) {
        if self.saver() == saver {
            return;
        }
        self.values.appearance.battery_saver = saver.id().to_owned();
        self.schedule_save(cx);
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn set_server_side_decorations(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.values.appearance.server_side_decorations = shown;
        self.schedule_save(cx);
    }

    pub fn set_window_controls(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.values.appearance.window_controls = shown;
        self.schedule_save(cx);
    }

    pub fn set_controls_on_left(&mut self, left: bool, cx: &mut Context<Self>) {
        self.values.appearance.controls_on_left = left;
        self.schedule_save(cx);
    }

    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.values.appearance.font_size = size.clamp(ui::MIN_FONT, ui::MAX_FONT);
        self.schedule_save(cx);
    }

    pub fn set_transparent(&mut self, transparent: bool, cx: &mut Context<Self>) {
        self.values.appearance.transparent = transparent;
        self.schedule_save(cx);
    }

    pub fn set_transparency(&mut self, transparency: f32, cx: &mut Context<Self>) {
        self.values.appearance.transparency = transparency.clamp(0., ui::MAX_TRANSPARENCY);
        self.schedule_save(cx);
    }

    /// Saves the window frame now and on every move or resize.
    pub fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_frame(window, cx);
        self.watch = Some(cx.observe_window_bounds(window, |this, window, cx| {
            this.keep_frame(window, cx);
        }));
    }

    fn keep_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let frame = Frame::of(window);
        if !frame.sane() || self.state.window == Some(frame) {
            return;
        }
        self.state.window = Some(frame);
        self.schedule_state_save(cx);
    }

    /// Wakes observers and debounces a `settings.json` write.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.save_quietly(cx);
    }

    /// Debounces a `settings.json` write. Replacing the task restarts the delay.
    fn save_quietly(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |this, _| this.save_now()).ok();
        }));
    }

    /// Wakes observers and debounces a `state.sqlite` write.
    fn schedule_state_save(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.save_state_quietly(cx);
    }

    /// Debounces a `state.sqlite` write without waking observers, for state no view renders.
    fn save_state_quietly(&mut self, cx: &mut Context<Self>) {
        self.save_state = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |this, _| this.save_state_now()).ok();
        }));
    }

    fn save_state_now(&self) {
        if let Err(error) = self.store.save(&self.state) {
            log::error!("settings: cannot save app state: {error:#}");
        }
    }

    /// Writes `settings.json` now. Returns false and logs why when it cannot.
    fn save_now(&self) -> bool {
        if !self.writable {
            return false;
        }
        let Some(parent) = self.path.parent() else {
            return false;
        };
        if let Err(error) = fs::create_dir_all(parent) {
            log::error!("settings: cannot create {}: {error}", parent.display());
            return false;
        }

        let bytes = match serde_json::to_vec_pretty(&self.values) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::error!("settings: cannot serialize values: {error}");
                return false;
            }
        };
        if let Err(error) = fs::write(&self.path, bytes) {
            log::error!("settings: cannot write {}: {error}", self.path.display());
            return false;
        }
        true
    }
}

/// The saved window frame as a placement, if it still lands on a connected display.
pub fn window_placement(least: Size<Pixels>, cx: &App) -> Option<WindowBounds> {
    let frame = Sonora::global(cx).settings.read(cx).state.window?;
    if !frame.sane() {
        return None;
    }

    let placement = frame.placement(least);
    let bounds = placement.get_bounds();
    cx.displays()
        .iter()
        .any(|display| display.bounds().intersects(&bounds))
        .then_some(placement)
}

/// Starts saving the window frame for the next launch.
pub fn remember_window(window: &mut Window, cx: &mut App) {
    let settings = Sonora::global(cx).settings.clone();
    settings.update(cx, |settings, cx| settings.watch_window(window, cx));
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sonora")
        .join("settings.json")
}

fn legacy_local_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sonora")
        .join("local-music.json")
}

/// The `version` key of a `settings.json`, without parsing the rest. Missing counts as 0.
fn settings_version(bytes: &[u8]) -> u32 {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("version")?.as_u64())
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(0)
}

/// The folder from the old `local-music.json`, which now lives in `settings.json`.
fn legacy_local_folder(path: &std::path::Path) -> Option<PathBuf> {
    #[derive(Deserialize)]
    struct Local {
        path: Option<PathBuf>,
    }

    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Local>(&bytes).ok())
        .and_then(|stored| stored.path)
}

fn remove_legacy_local_folder(path: &std::path::Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("settings: cannot remove {}: {error}", path.display());
    }
}

/// The pins of the given providers, in stored order.
fn gather(pinned: &[Held], slugs: &[&str]) -> Vec<Pin> {
    shown(pinned, slugs)
        .map(|(_, held)| held.pin.clone())
        .collect()
}

/// The pins of `slugs` with their index in the full list. Other providers' pins are skipped, not
/// removed, so they keep their place.
fn shown<'a>(
    pinned: &'a [Held],
    slugs: &'a [&str],
) -> impl Iterator<Item = (usize, &'a Held)> + 'a {
    pinned
        .iter()
        .enumerate()
        .filter(move |(_, held)| slugs.contains(&held.slug.as_str()))
}

/// Removes one pin and reports whether it was there.
fn take(pinned: &mut Vec<Held>, slug: &str, pin: &Pin) -> bool {
    let Some(index) = pinned
        .iter()
        .position(|held| held.slug == slug && held.pin.same(pin))
    else {
        return false;
    };
    pinned.remove(index);
    true
}

/// Carries position and origin over from the previous record. Position survives only while the
/// same track is current, origin as long as the provider is the same. Another provider inherits
/// nothing.
fn carry(previous: Option<&Resume>, next: &mut Resume) {
    let playing = |resume: &Resume| resume.current.as_ref().map(|stub| stub.id.clone());
    let same = previous.filter(|old| old.provider == next.provider);
    next.position = same
        .filter(|old| playing(old) == playing(next))
        .map_or(0., |old| old.position);
    next.origin = same.and_then(|old| old.origin.clone());
}

/// Inserts or moves `pin` into the `gap`th slot among the pins of `slugs`. `None` or a gap past
/// the end means the end. Returns false when nothing moved.
fn place(pinned: &mut Vec<Held>, slug: &str, pin: Pin, gap: Option<usize>, slugs: &[&str]) -> bool {
    let visible: Vec<usize> = shown(pinned, slugs).map(|(index, _)| index).collect();
    let gap = gap.unwrap_or(visible.len()).min(visible.len());
    let target = match gap {
        0 => visible.first().copied().unwrap_or(pinned.len()),
        gap => visible[gap - 1] + 1,
    };

    let Some(from) = pinned.iter().position(|held| held.pin.same(&pin)) else {
        pinned.insert(
            target,
            Held {
                slug: slug.to_owned(),
                pin,
            },
        );
        return true;
    };

    let to = gap_target(from, target, pinned.len());
    if to == from {
        return false;
    }

    let moved = pinned.remove(from);
    pinned.insert(to, moved);
    true
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::queue::Stub;
    use ui::PinKind;

    fn resume(provider: &str, playing: &str, position: f32) -> Resume {
        Resume {
            provider: provider.to_owned(),
            position,
            current: Some(Stub {
                id: playing.to_owned(),
                ..Stub::default()
            }),
            ..Resume::default()
        }
    }

    #[test]
    fn lyrics_start_karaoke_and_romanize_only_cjk() {
        let values: Values = serde_json::from_str("{}").expect("empty settings use defaults");

        assert!(values.karaoke_lyrics);
        assert!(values.romanized_lyrics);
        let romanized = [
            WritingSystem::Japanese,
            WritingSystem::Chinese,
            WritingSystem::Korean,
        ];
        for system in WritingSystem::ALL {
            assert_eq!(
                values.romanization_scripts.contains(system),
                romanized.contains(&system)
            );
        }
    }

    #[test]
    fn one_saved_romanization_choice_keeps_the_other_defaults() {
        let values: Values = serde_json::from_str(
            r#"{
                "romanization_scripts": { "japanese": false }
            }"#,
        )
        .expect("partial script preferences use defaults");

        assert!(
            !values
                .romanization_scripts
                .contains(WritingSystem::Japanese)
        );
        assert!(values.romanization_scripts.contains(WritingSystem::Chinese));
        assert!(!values.romanization_scripts.contains(WritingSystem::Other));
    }

    #[test]
    fn the_saved_position_follows_the_same_track() {
        let previous = resume("spotify", "abc", 42.);
        let mut next = resume("spotify", "abc", 0.);

        carry(Some(&previous), &mut next);

        assert_eq!(next.position, 42.);
    }

    #[test]
    fn a_new_track_starts_from_the_beginning() {
        let previous = resume("spotify", "abc", 42.);
        let mut next = resume("spotify", "def", 0.);

        carry(Some(&previous), &mut next);

        assert_eq!(next.position, 0.);
    }

    #[test]
    fn another_provider_never_inherits_a_position() {
        let previous = resume("spotify", "abc", 42.);
        let mut next = resume("youtube", "abc", 0.);

        carry(Some(&previous), &mut next);

        assert_eq!(next.position, 0.);
    }

    #[test]
    fn a_first_record_starts_from_the_beginning() {
        let mut next = resume("spotify", "abc", 42.);

        carry(None, &mut next);

        assert_eq!(next.position, 0.);
    }

    const SLUGS: [&str; 2] = ["spotify", "local"];

    fn pin(id: &str) -> Pin {
        Pin::new(PinKind::Album, id, id)
    }

    fn held(slug: &str, id: &str) -> Held {
        Held {
            slug: slug.to_owned(),
            pin: pin(id),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sonora-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn runtime_state_round_trips_through_sqlite() {
        let root = scratch("state");
        let path = root.join("state.sqlite");
        let store = StateStore::new(Database::at(path.clone()));
        let state = StateValues {
            volume: 0.2,
            provider: "youtube".to_owned(),
            sidebar_right_open: true,
            pinned: vec![held("spotify", "album")],
            ..StateValues::default()
        };

        store.save(&state).expect("state saves");
        let loaded = store.load().expect("state loads").expect("state exists");

        assert_eq!(loaded.volume, 0.2);
        assert_eq!(loaded.provider, "youtube");
        assert!(loaded.sidebar_right_open);
        assert_eq!(loaded.pinned.len(), 1);

        fs::remove_file(path).expect("test database is removed");
        fs::remove_dir(root).expect("test directory is removed");
    }

    #[test]
    fn first_load_moves_legacy_state_and_local_folder_then_cleans_json() {
        let root = scratch("migration");
        fs::create_dir_all(&root).expect("test directory is created");
        let settings_path = root.join("settings.json");
        let legacy_local = root.join("local-music.json");
        let database = root.join("state.sqlite");
        fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "volume": 0.2,
                "provider": "youtube",
                "sidebar_open": false,
                "appearance": { "system_theme": "light" }
            }))
            .expect("legacy settings serialize"),
        )
        .expect("legacy settings are written");
        fs::write(
            &legacy_local,
            serde_json::to_vec_pretty(&serde_json::json!({ "path": "/music" }))
                .expect("legacy local config serializes"),
        )
        .expect("legacy local config is written");

        let settings = AppSettings::load_from(
            settings_path.clone(),
            StateStore::new(Database::at(database.clone())),
            legacy_local.clone(),
        );

        assert_eq!(settings.volume(), 0.2);
        assert_eq!(settings.provider(), "youtube");
        assert!(!settings.sidebar_open());
        assert_eq!(
            settings.local_folder(),
            Some(std::path::Path::new("/music"))
        );
        assert!(!legacy_local.exists());

        let cleaned: serde_json::Value =
            serde_json::from_slice(&fs::read(&settings_path).expect("cleaned settings are read"))
                .expect("cleaned settings parse");
        let object = cleaned.as_object().expect("settings are an object");
        assert_eq!(object.get("version"), Some(&serde_json::json!(2)));
        assert_eq!(
            object.get("local_folder"),
            Some(&serde_json::json!("/music"))
        );
        for key in ["volume", "provider", "sidebar_open"] {
            assert!(!object.contains_key(key), "{key} survived cleanup");
        }
        assert!(
            !object["appearance"]
                .as_object()
                .expect("appearance is an object")
                .contains_key("system_theme")
        );

        let state = settings
            .store
            .load()
            .expect("migrated state loads")
            .expect("migrated state exists");
        assert_eq!(state.volume, 0.2);
        assert_eq!(state.provider, "youtube");
        assert!(!state.sidebar_open);
        assert_eq!(state.system_theme, "light");

        fs::remove_file(settings_path).expect("test settings are removed");
        fs::remove_file(database).expect("test database is removed");
        fs::remove_dir(root).expect("test directory is removed");
    }

    fn ids(pinned: &[Held]) -> Vec<&str> {
        pinned.iter().map(|held| held.pin.id.as_str()).collect()
    }

    #[test]
    fn a_fresh_pin_lands_at_the_gap() {
        let mut pinned = vec![held("spotify", "a"), held("spotify", "b")];

        assert!(place(&mut pinned, "spotify", pin("c"), Some(1), &SLUGS));
        assert_eq!(ids(&pinned), ["a", "c", "b"]);
    }

    #[test]
    fn no_gap_appends() {
        let mut pinned = vec![held("spotify", "a")];

        assert!(place(&mut pinned, "spotify", pin("b"), None, &SLUGS));
        assert_eq!(ids(&pinned), ["a", "b"]);
    }

    #[test]
    fn a_gap_past_the_end_still_appends() {
        let mut pinned = vec![held("spotify", "a")];

        assert!(place(&mut pinned, "spotify", pin("b"), Some(9), &SLUGS));
        assert_eq!(ids(&pinned), ["a", "b"]);
    }

    #[test]
    fn pinning_twice_moves_instead_of_duplicating() {
        let mut pinned = vec![
            held("spotify", "a"),
            held("spotify", "b"),
            held("spotify", "c"),
        ];

        assert!(place(&mut pinned, "spotify", pin("a"), Some(3), &SLUGS));
        assert_eq!(ids(&pinned), ["b", "c", "a"]);
    }

    #[test]
    fn a_move_backwards_keeps_the_gap() {
        let mut pinned = vec![
            held("spotify", "a"),
            held("spotify", "b"),
            held("spotify", "c"),
        ];

        assert!(place(&mut pinned, "spotify", pin("c"), Some(0), &SLUGS));
        assert_eq!(ids(&pinned), ["c", "a", "b"]);
    }

    #[test]
    fn the_gaps_around_an_item_are_no_ops() {
        let mut pinned = vec![
            held("spotify", "a"),
            held("spotify", "b"),
            held("spotify", "c"),
        ];

        assert!(!place(&mut pinned, "spotify", pin("b"), Some(1), &SLUGS));
        assert!(!place(&mut pinned, "spotify", pin("b"), Some(2), &SLUGS));
        assert_eq!(ids(&pinned), ["a", "b", "c"]);
    }

    #[test]
    fn kinds_with_the_same_id_stay_apart() {
        let mut pinned = vec![held("spotify", "x")];

        assert!(place(
            &mut pinned,
            "spotify",
            Pin::new(PinKind::Song, "x", "x"),
            None,
            &SLUGS
        ));
        assert_eq!(pinned.len(), 2);
    }
}
