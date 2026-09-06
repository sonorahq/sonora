use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Pixels, Size, Subscription, Task, Window, WindowBounds, point, px, size,
};
use music::WritingSystem;
use serde::{Deserialize, Serialize};
use ui::{
    Layout, Look, Mode, Pace, Pin, Rounding, Saver, Sorting, Stillness, ThemeKind, ThemeOverrides,
};

use crate::queue::{Resume, gap_target};
use crate::{Repeat, Sonora};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideTab {
    #[default]
    Queue,
    Lyrics,
}

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

    fn sane(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|it| it.is_finite())
            && self.width > 0.
            && self.height > 0.
    }

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

const SAVE_DELAY: Duration = Duration::from_millis(300);
const DEFAULT_VOLUME: f32 = 0.7;
const DEFAULT_SIDEBAR_WIDTH: f32 = 195.;
const DEFAULT_SIDEBAR_RIGHT_WIDTH: f32 = 254.;
const DEFAULT_FONT_SIZE: f32 = 14.;
const DEFAULT_LYRICS_SCALE: f32 = 1.;
const DEFAULT_STARTUP: &str = "home";

pub const SYSTEM_FONT: &str = "auto";

type Groups = HashMap<String, Vec<Pin>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Held {
    slug: String,
    #[serde(flatten)]
    pin: Pin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Values {
    version: u32,
    volume: f32,
    normalisation: bool,
    gapless: bool,
    discord_rich_presence: bool,
    discord_hide_details: bool,
    lyrics_for_local_files: bool,
    karaoke_lyrics: bool,
    romanized_lyrics: bool,
    panel_lyrics_scale: f32,
    fullscreen_lyrics_scale: f32,
    romanization_scripts: RomanizationScripts,
    adaptive_menu: bool,
    check_updates: bool,
    close_to_tray: bool,
    sidebar_width: f32,
    sidebar_open: bool,
    sidebar_right_width: f32,
    sidebar_right_open: bool,
    sidebar_right_tab: SideTab,
    shuffle: bool,
    repeat: Repeat,
    radio: bool,
    language: String,
    #[serde(default = "system_font")]
    font: String,
    provider: String,
    startup: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hidden_nav: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    hidden_columns: HashMap<String, Vec<String>>,
    tables: HashMap<String, Layout>,
    sorting: HashMap<String, Option<Sorting>>,
    views: HashMap<String, Mode>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pins: Groups,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pinned: Vec<Held>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume: Option<Resume>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<Frame>,
    appearance: Appearance,
}

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
    window_controls: bool,
    controls_on_left: bool,
    reduce_motion: String,
    motion_pace: String,
    battery_saver: String,
    system_theme: String,
    theme_overrides: ThemeOverrides,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            version: 1,
            volume: DEFAULT_VOLUME,
            normalisation: false,
            gapless: true,
            discord_rich_presence: false,
            discord_hide_details: false,
            lyrics_for_local_files: true,
            karaoke_lyrics: true,
            romanized_lyrics: true,
            panel_lyrics_scale: DEFAULT_LYRICS_SCALE,
            fullscreen_lyrics_scale: DEFAULT_LYRICS_SCALE,
            romanization_scripts: RomanizationScripts::default(),
            adaptive_menu: false,
            check_updates: cfg!(target_os = "windows"),
            close_to_tray: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_open: true,
            sidebar_right_width: DEFAULT_SIDEBAR_RIGHT_WIDTH,
            sidebar_right_open: false,
            sidebar_right_tab: SideTab::Queue,
            shuffle: false,
            repeat: Repeat::Off,
            radio: false,
            language: i18n::AUTO.to_owned(),
            font: system_font(),
            provider: "spotify".to_owned(),
            startup: DEFAULT_STARTUP.to_owned(),
            hidden_nav: Vec::new(),
            hidden_columns: HashMap::new(),
            tables: HashMap::new(),
            sorting: HashMap::new(),
            views: HashMap::new(),
            pins: Groups::new(),
            pinned: Vec::new(),
            resume: None,
            window: None,
            appearance: Appearance::default(),
        }
    }
}

impl Values {
    fn migrate(&mut self) {
        for (table, hidden) in self.hidden_columns.drain() {
            self.tables.entry(table).or_insert_with(|| Layout {
                hidden,
                ..Layout::default()
            });
        }

        let mut slugs: Vec<String> = self.pins.keys().cloned().collect();
        slugs.sort_by_key(|slug| (*slug != self.provider, slug.clone()));
        for slug in slugs {
            let Some(group) = self.pins.remove(&slug) else {
                continue;
            };
            self.pinned.extend(group.into_iter().map(|pin| Held {
                slug: slug.clone(),
                pin,
            }));
        }
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
            window_controls: true,
            controls_on_left: false,
            reduce_motion: Stillness::default().id().to_owned(),
            motion_pace: Pace::default().id().to_owned(),
            battery_saver: Saver::default().id().to_owned(),
            system_theme: ThemeKind::Dark.id().to_owned(),
            theme_overrides: ThemeOverrides::default(),
        }
    }
}

pub struct AppSettings {
    values: Values,
    path: PathBuf,
    save: Option<Task<()>>,
    watch: Option<Subscription>,
    writable: bool,
}

impl AppSettings {
    pub fn load() -> Self {
        let path = settings_path();
        let (mut values, writable) = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Values>(&bytes) {
                Ok(values) => (values, true),
                Err(error) => {
                    log::warn!("settings: cannot parse {}: {error}", path.display());
                    (Values::default(), false)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Values::default(), true),
            Err(error) => {
                log::warn!("settings: cannot read {}: {error}", path.display());
                (Values::default(), false)
            }
        };
        values.migrate();

        Self {
            values,
            path,
            save: None,
            watch: None,
            writable,
        }
    }

    pub fn volume(&self) -> f32 {
        self.values.volume.clamp(0., 1.)
    }

    pub fn normalisation(&self) -> bool {
        self.values.normalisation
    }

    pub fn gapless(&self) -> bool {
        self.values.gapless
    }

    pub fn discord_rich_presence(&self) -> bool {
        self.values.discord_rich_presence
    }

    pub fn discord_hide_details(&self) -> bool {
        self.values.discord_hide_details
    }

    pub fn lyrics_for_local_files(&self) -> bool {
        self.values.lyrics_for_local_files
    }

    pub fn karaoke_lyrics(&self) -> bool {
        self.values.karaoke_lyrics
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
        self.values.sidebar_width
    }

    pub fn sidebar_open(&self) -> bool {
        self.values.sidebar_open
    }

    pub fn sidebar_right_width(&self) -> f32 {
        self.values.sidebar_right_width
    }

    pub fn sidebar_right_open(&self) -> bool {
        self.values.sidebar_right_open
    }

    pub fn sidebar_right_tab(&self) -> SideTab {
        self.values.sidebar_right_tab
    }

    pub fn shuffle(&self) -> bool {
        self.values.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.values.repeat
    }

    pub fn radio(&self) -> bool {
        self.values.radio
    }

    pub fn language(&self) -> &str {
        &self.values.language
    }

    pub fn font(&self) -> &str {
        &self.values.font
    }

    pub fn provider(&self) -> &str {
        &self.values.provider
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
        ThemeKind::from_id(&self.values.appearance.system_theme)
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

    pub fn ensure_file(&self) -> PathBuf {
        if !self.path.exists() {
            self.save_now();
        }
        self.path.clone()
    }

    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.values.volume = volume.clamp(0., 1.);
        self.schedule_save(cx);
    }

    pub fn set_normalisation(&mut self, normalisation: bool, cx: &mut Context<Self>) {
        self.values.normalisation = normalisation;
        self.schedule_save(cx);
    }

    pub fn set_gapless(&mut self, gapless: bool, cx: &mut Context<Self>) {
        self.values.gapless = gapless;
        self.schedule_save(cx);
    }

    pub fn set_discord_rich_presence(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.values.discord_rich_presence = enabled;
        self.schedule_save(cx);
    }

    pub fn set_discord_hide_details(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.values.discord_hide_details = enabled;
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
        self.values.tables.get(table).cloned().unwrap_or_default()
    }

    pub fn set_table(&mut self, table: &str, layout: Layout, cx: &mut Context<Self>) {
        if self.values.tables.get(table) == Some(&layout) {
            return;
        }
        self.values.tables.insert(table.to_owned(), layout);
        self.schedule_save(cx);
    }

    pub fn view_or(&self, table: &str, fallback: Mode) -> Mode {
        self.values.views.get(table).copied().unwrap_or(fallback)
    }

    pub fn set_view(&mut self, table: &str, mode: Mode, cx: &mut Context<Self>) {
        if self.values.views.get(table) == Some(&mode) {
            return;
        }
        self.values.views.insert(table.to_owned(), mode);
        self.schedule_save(cx);
    }

    pub fn sorting(&self, table: &str) -> Option<Option<Sorting>> {
        self.values.sorting.get(table).cloned()
    }

    pub fn set_sorting(&mut self, table: &str, sorting: Option<Sorting>, cx: &mut Context<Self>) {
        if self.values.sorting.get(table) == Some(&sorting) {
            return;
        }
        self.values.sorting.insert(table.to_owned(), sorting);
        self.schedule_save(cx);
    }

    pub fn pinned(&self, slugs: &[&str]) -> Vec<Pin> {
        gather(&self.values.pinned, slugs)
    }

    pub fn resume(&self) -> Option<&Resume> {
        self.values.resume.as_ref()
    }

    pub fn set_resume(&mut self, resume: Option<Resume>, cx: &mut Context<Self>) {
        let mut resume = resume;
        if let Some(next) = resume.as_mut() {
            carry(self.values.resume.as_ref(), next);
        }
        if self.values.resume == resume {
            return;
        }
        self.values.resume = resume;
        self.schedule_save(cx);
    }

    pub fn set_resume_origin(&mut self, origin: Option<crate::Origin>, cx: &mut Context<Self>) {
        let Some(resume) = self.values.resume.as_mut() else {
            return;
        };
        if resume.origin == origin {
            return;
        }
        resume.origin = origin;
        self.save_quietly(cx);
    }

    pub fn set_resume_position(&mut self, position: f32, cx: &mut Context<Self>) {
        let Some(resume) = self.values.resume.as_mut() else {
            return;
        };
        if resume.position == position {
            return;
        }
        resume.position = position;
        self.save_quietly(cx);
    }

    pub fn pin(
        &mut self,
        slug: &str,
        pin: Pin,
        gap: Option<usize>,
        slugs: &[&str],
        cx: &mut Context<Self>,
    ) {
        if !place(&mut self.values.pinned, slug, pin, gap, slugs) {
            return;
        }
        self.schedule_save(cx);
    }

    pub fn unpin(&mut self, slug: &str, pin: &Pin, cx: &mut Context<Self>) {
        if !take(&mut self.values.pinned, slug, pin) {
            return;
        }
        self.schedule_save(cx);
    }

    pub fn set_sidebar(&mut self, width: f32, open: bool, cx: &mut Context<Self>) {
        self.values.sidebar_width = width;
        self.values.sidebar_open = open;
        self.schedule_save(cx);
    }

    pub fn set_sidebar_right_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.values.sidebar_right_width = width;
        self.schedule_save(cx);
    }

    pub fn set_sidebar_right_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.values.sidebar_right_open = open;
        self.schedule_save(cx);
    }

    pub fn set_sidebar_right_tab(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if self.values.sidebar_right_tab == tab {
            return;
        }
        self.values.sidebar_right_tab = tab;
        self.schedule_save(cx);
    }

    pub fn set_shuffle(&mut self, shuffle: bool, cx: &mut Context<Self>) {
        self.values.shuffle = shuffle;
        self.schedule_save(cx);
    }

    pub fn set_repeat(&mut self, repeat: Repeat, cx: &mut Context<Self>) {
        self.values.repeat = repeat;
        self.schedule_save(cx);
    }

    pub fn set_radio(&mut self, radio: bool, cx: &mut Context<Self>) {
        self.values.radio = radio;
        self.schedule_save(cx);
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
        if self.values.provider == provider {
            return;
        }
        self.values.provider = provider;
        self.schedule_save(cx);
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
        self.values.appearance.system_theme = kind.id().to_owned();
        self.schedule_save(cx);
    }

    pub fn set_saver(&mut self, saver: Saver, cx: &mut Context<Self>) {
        if self.saver() == saver {
            return;
        }
        self.values.appearance.battery_saver = saver.id().to_owned();
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

    pub fn watch_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_frame(window, cx);
        self.watch = Some(cx.observe_window_bounds(window, |this, window, cx| {
            this.keep_frame(window, cx);
        }));
    }

    fn keep_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let frame = Frame::of(window);
        if !frame.sane() || self.values.window == Some(frame) {
            return;
        }
        self.values.window = Some(frame);
        self.schedule_save(cx);
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.save_quietly(cx);
    }

    /// Persists without waking observers, for values no view renders.
    fn save_quietly(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |this, _| this.save_now()).ok();
        }));
    }

    fn save_now(&self) {
        if !self.writable {
            return;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        if let Err(error) = fs::create_dir_all(parent) {
            log::error!("settings: cannot create {}: {error}", parent.display());
            return;
        }

        let bytes = match serde_json::to_vec_pretty(&self.values) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::error!("settings: cannot serialize values: {error}");
                return;
            }
        };
        if let Err(error) = fs::write(&self.path, bytes) {
            log::error!("settings: cannot write {}: {error}", self.path.display());
        }
    }
}

pub fn window_placement(least: Size<Pixels>, cx: &App) -> Option<WindowBounds> {
    let frame = Sonora::global(cx).settings.read(cx).values.window?;
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

fn gather(pinned: &[Held], slugs: &[&str]) -> Vec<Pin> {
    shown(pinned, slugs)
        .map(|(_, held)| held.pin.clone())
        .collect()
}

fn shown<'a>(
    pinned: &'a [Held],
    slugs: &'a [&str],
) -> impl Iterator<Item = (usize, &'a Held)> + 'a {
    pinned
        .iter()
        .enumerate()
        .filter(move |(_, held)| slugs.contains(&held.slug.as_str()))
}

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

fn carry(previous: Option<&Resume>, next: &mut Resume) {
    let playing = |resume: &Resume| resume.current.as_ref().map(|stub| stub.id.clone());
    let same = previous.filter(|old| old.provider == next.provider);
    next.position = same
        .filter(|old| playing(old) == playing(next))
        .map_or(0., |old| old.position);
    // the queue moving on does not change where it came from
    next.origin = same.and_then(|old| old.origin.clone());
}

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
