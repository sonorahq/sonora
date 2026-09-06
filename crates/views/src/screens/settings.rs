use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::shared::local;
use crate::shared::popups::{AccountPicker, CookiePrompt, SearchPopup, matches_query};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Pixels, Render, SharedString, TextRun, Window,
    div, font, px,
};
use gpui::{ScrollHandle, prelude::*, svg};
use i18n::{Language, t};
use music::{AccountChoice, SignIn, SignInPrompt, WritingSystem};
use router::{NavEntry, Screen, SettingsTab};
use state::{AppSettings, Failure, Playback, SYSTEM_FONT, Session, SessionState, Sonora};
use ui::{ActiveTheme as _, Scrollbar, Scroller, eyebrow};
use ui::{
    Avatar, Button, InfoCard, Initials, Input, Look, MAX_FONT, MAX_LYRICS_SCALE, MAX_TRANSPARENCY,
    MIN_FONT, MIN_LYRICS_SCALE, MenuItem, Pace, Picker, Popovers, Rounding, Saver, Scrubber,
    ScrubberState, Separator, Skeleton, Stillness, Switch, Text, Theme, ThemeKind,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
const SOURCE_URL: &str = "https://github.com/nolight132/sonora";

const THEMES: &str = "themes";
const PACKS: &str = "packs";
const CORNERS: &str = "corners";
const LANGUAGES: &str = "languages";
const TYPEFACES: &str = "typefaces";
const TYPEFACE_LIMIT: usize = 200;
const TYPEFACE_LEAD: usize = 2;
// faces previewed before a measurement
const TYPEFACE_GUESS: usize = 24;
// faces loaded per frame
const TYPEFACE_BATCH: usize = 3;
const STARTUP: &str = "startup";
const ENTRIES: &str = "entries";
const MOTION: &str = "motion";
const PACE: &str = "pace";
const SAVER: &str = "saver";

enum Row {
    Item(AnyElement),
    Title(AnyElement),
}

impl Row {
    fn into_element(self) -> AnyElement {
        match self {
            Self::Item(element) | Self::Title(element) => element,
        }
    }
}

struct Account {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    stored: bool,
    active: bool,
    guest: bool,
    cancel: bool,
    error: Option<Failure>,
}

fn offered(method: &SignIn, stored: bool, guest: bool) -> bool {
    match method {
        SignIn::Default | SignIn::Anonymous => !stored,
        SignIn::Secret => !stored || guest,
        SignIn::Path(_) => false,
    }
}

#[derive(Clone, Copy)]
struct Member {
    login: &'static str,
    avatar: &'static str,
    profile: &'static str,
    role: Role,
}

#[derive(Clone, Copy)]
enum Role {
    LeadMaintainer,
    Maintainer,
    Contributor,
}

impl Role {
    fn label(self) -> SharedString {
        match self {
            Self::LeadMaintainer => t!("settings-role-lead-maintainer"),
            Self::Maintainer => t!("settings-role-maintainer"),
            Self::Contributor => t!("settings-role-contributor"),
        }
    }
}

macro_rules! member {
    ($login:literal, $role:expr) => {
        Member {
            login: $login,
            avatar: concat!("https://github.com/", $login, ".png"),
            profile: concat!("https://github.com/", $login),
            role: $role,
        }
    };
}

const MEMBERS: [Member; 5] = [
    member!("nolight132", Role::LeadMaintainer),
    member!("zxsleebu", Role::Maintainer),
    member!("fx-got", Role::Maintainer),
    member!("Makakashan", Role::Contributor),
    member!("imizgun", Role::Contributor),
];

pub struct SettingsView {
    session: Entity<Session>,
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    tab: SettingsTab,
    scrollbar: Entity<Scrollbar>,
    opacity: ScrubberState,
    popovers: Popovers,
    secret: Entity<Input>,
    languages: SearchPopup,
    typefaces: SearchPopup,
    typeface_faced: RefCell<HashSet<SharedString>>,
    installed: Option<Vec<SharedString>>,
}

impl SettingsView {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let me = cx.entity_id();
        let languages = SearchPopup::new("settings-language-search", me, cx);
        cx.observe(&languages.input(), |this, _, cx| {
            this.languages.changed(cx);
            cx.notify();
        })
        .detach();
        let typefaces = SearchPopup::new("settings-typeface-search", me, cx);
        cx.observe(&typefaces.input(), |this, _, cx| {
            this.typefaces.changed(cx);
            cx.notify();
        })
        .detach();
        Self {
            session,
            playback,
            settings,
            tab: SettingsTab::General,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            opacity: ScrubberState::new("opacity"),
            popovers: Popovers::default(),
            secret: cx.new(|cx| Input::new("login-cookie-hint", cx)),
            languages,
            typefaces,
            typeface_faced: RefCell::new(HashSet::new()),
            installed: None,
        }
    }

    pub(crate) fn select(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.popovers.close();
        cx.notify();
    }

    fn panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<Row> = match self.tab {
            SettingsTab::General => vec![
                Row::Item(self.startup_row(cx).into_any_element()),
                Row::Item(self.entries_row(cx).into_any_element()),
                Row::Item(self.language_row(cx).into_any_element()),
                self.title("settings-group-window", cx),
                Row::Item(self.tray_row(cx).into_any_element()),
                self.title("settings-group-accounts", cx),
                Row::Item(self.accounts_row(cx).into_any_element()),
                self.title("settings-group-library", cx),
                Row::Item(self.local_folder_row(cx).into_any_element()),
            ],
            SettingsTab::Appearance => vec![
                Row::Item(self.theme_row(cx).into_any_element()),
                Row::Item(self.adaptive_row(cx).into_any_element()),
                Row::Item(self.visualizer_row(cx).into_any_element()),
                Row::Item(self.icons_row(cx).into_any_element()),
                Row::Item(self.opacity_row(cx).into_any_element()),
                Row::Item(self.corners_row(cx).into_any_element()),
                self.title("settings-group-lyrics", cx),
                Row::Item(self.panel_lyrics_size_row(cx).into_any_element()),
                Row::Item(self.fullscreen_lyrics_size_row(cx).into_any_element()),
                Row::Item(self.blur_lyrics_row(cx).into_any_element()),
                self.title("settings-group-text", cx),
                Row::Item(self.font_row(cx).into_any_element()),
                Row::Item(self.typeface_row(cx).into_any_element()),
                self.title("settings-group-motion", cx),
                Row::Item(self.motion_row(cx).into_any_element()),
                Row::Item(self.pace_row(cx).into_any_element()),
                Row::Item(self.saver_row(cx).into_any_element()),
            ]
            .into_iter()
            .chain(self.decoration_rows(cx))
            .chain([
                self.title("settings-advanced", cx),
                Row::Item(self.adaptive_menu_row(cx).into_any_element()),
            ])
            .collect(),
            SettingsTab::Playback => vec![
                Row::Item(self.playback_row(cx).into_any_element()),
                Row::Item(self.gapless_row(cx).into_any_element()),
                self.title("settings-group-lyrics", cx),
                Row::Item(self.karaoke_lyrics_row(cx).into_any_element()),
                Row::Item(self.romanized_lyrics_row(cx).into_any_element()),
            ],
            SettingsTab::Privacy => vec![Row::Item(
                self.lyrics_for_local_files_row(cx).into_any_element(),
            )],
            SettingsTab::About => vec![
                Row::Item(self.version_row(cx).into_any_element()),
                Row::Item(self.updates_row(cx).into_any_element()),
                self.title("settings-group-project", cx),
                Row::Item(self.license_row(cx).into_any_element()),
                Row::Item(self.source_row(cx).into_any_element()),
            ],
        };

        let mut panel = div().flex().flex_col();
        let mut parted = false;
        for row in rows {
            let titled = matches!(row, Row::Title(_));
            if parted && !titled {
                panel = panel.child(Separator::horizontal().w_full());
            }
            parted = !titled;
            panel = panel.child(row.into_element());
        }
        panel
    }

    fn decoration_rows(&self, cx: &mut Context<Self>) -> Vec<Row> {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let rows = vec![
            self.title("settings-group-window-style", cx),
            Row::Item(self.server_side_decorations_row(cx).into_any_element()),
            Row::Item(self.side_row(cx).into_any_element()),
        ];
        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
        let rows = vec![
            self.title("settings-group-title-bar", cx),
            Row::Item(self.decorations_row(cx).into_any_element()),
            Row::Item(self.side_row(cx).into_any_element()),
        ];
        #[cfg(target_os = "macos")]
        let rows = Vec::<Row>::new();
        rows
    }

    fn title(&self, key: &'static str, cx: &App) -> Row {
        Row::Title(
            div()
                .pt_5()
                .pb_1()
                .child(eyebrow(i18n::lookup(key, None), cx))
                .into_any_element(),
        )
    }

    fn look(&self, cx: &Context<Self>) -> Look {
        Look {
            tint: cx.theme().tint,
            ..self.settings.read(cx).look()
        }
    }

    fn startup_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = Screen::from_id(self.settings.read(cx).startup()).unwrap_or(Screen::Home);
        let current = i18n::lookup(chosen.key(), None);

        let picker = Picker::new(STARTUP, &self.popovers, current)
            .width(Picker::NARROW)
            .items(Screen::ALL.map(|screen| {
                MenuItem::new(screen.id(), i18n::lookup(screen.key(), None))
                    .selected(screen == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_startup(screen.id(), cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-startup"),
            t!("settings-startup-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn entries_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let picker = Picker::new(ENTRIES, &self.popovers, t!("settings-entries-pick"))
            .width(Picker::REGULAR)
            .sticky()
            .items(NavEntry::ALL.map(|entry| {
                let shown = self.settings.read(cx).nav_shown(entry.id());

                MenuItem::new(entry.id(), i18n::lookup(entry.key(), None))
                    .selected(shown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_nav_shown(entry.id(), !shown, cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-entries"),
            t!("settings-entries-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn language_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).language().to_owned();
        let current = match Language::from_id(&chosen) {
            Some(language) => SharedString::from(language.label()),
            None => t!("settings-language-system"),
        };

        let asked = self.languages.query();
        let entries = std::iter::once((i18n::AUTO, t!("settings-language-system")))
            .chain(
                Language::ALL
                    .into_iter()
                    .map(|language| (language.id(), SharedString::from(language.label()))),
            )
            .filter(|(id, label)| matches_query(id, label, &asked))
            .collect::<Vec<_>>();
        let barren = entries.is_empty();
        let count = entries.len();
        let cursor = self.languages.cursor(count);
        let submitted = entries.clone();

        let picker = Picker::new(LANGUAGES, &self.popovers, current)
            .width(Picker::WIDE)
            .menu(self.languages.menu("languages-menu", Picker::WIDE))
            .items(entries.into_iter().enumerate().map(|(place, (id, label))| {
                MenuItem::new(id, label)
                    .selected(place == cursor)
                    .checked(chosen == id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_language(id, cx));
                        cx.notify();
                    }))
            }))
            .when(barren, |picker| {
                picker
                    .item(MenuItem::new("language-empty", t!("settings-language-none")).disabled())
            });
        let picker = self.languages.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some((id, _)) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_language(*id, cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(
            t!("settings-language"),
            t!("settings-language-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn typeface_entries(&self) -> Vec<SharedString> {
        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();

        std::iter::once(SharedString::from(SYSTEM_FONT))
            .chain(installed.iter().cloned())
            .filter(|name| {
                let label = match name.as_ref() == SYSTEM_FONT {
                    true => t!("settings-typeface-system"),
                    false => name.clone(),
                };
                matches_query(name, &label, &asked)
            })
            .take(TYPEFACE_LIMIT)
            .collect()
    }

    fn typeface_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).font().to_owned();
        let bundled = chosen == SYSTEM_FONT;
        let current = match bundled {
            true => t!("settings-typeface-system"),
            false => SharedString::from(chosen.clone()),
        };

        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();
        let entries = std::iter::once((
            SharedString::from(SYSTEM_FONT),
            t!("settings-typeface-system"),
        ))
        .chain(installed.iter().map(|name| (name.clone(), name.clone())))
        .filter(|(id, label)| matches_query(id, label, &asked))
        .take(TYPEFACE_LIMIT)
        .collect::<Vec<_>>();
        let barren = entries.is_empty();
        let count = entries.len();
        let cursor = self.typefaces.cursor(count);
        let submitted = entries
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        // a face costs a font load
        let scroll = self.typefaces.scroll(cx);
        let row = scroll
            .bounds_for_item(0)
            .map(|item| item.size.height)
            .filter(|height| *height > px(0.));
        let first = row.map_or(cursor, |row| {
            ((-scroll.offset().y) / row).floor().max(0.) as usize
        });
        let shown = row.map_or(TYPEFACE_GUESS, |row| {
            (self.typefaces.height() / row).ceil() as usize
        });
        let previewed = first.saturating_sub(TYPEFACE_LEAD)..first + shown + TYPEFACE_LEAD;
        let picking = self.popovers.shows(TYPEFACES);

        let mut budget = TYPEFACE_BATCH;
        let mut waiting = false;
        let mut faced = self.typeface_faced.borrow_mut();
        let items = entries
            .into_iter()
            .enumerate()
            .map(|(place, (id, label))| {
                let name = id.clone();
                let preview = name.clone();
                let wanted = picking && name.as_ref() != SYSTEM_FONT && previewed.contains(&place);
                let shows = match (wanted, faced.contains(&name)) {
                    (false, _) => false,
                    (true, true) => true,
                    (true, false) => match budget {
                        0 => {
                            waiting = true;
                            false
                        }
                        _ => {
                            budget -= 1;
                            faced.insert(name.clone());
                            true
                        }
                    },
                };
                MenuItem::new(id, label)
                    .selected(place == cursor)
                    .checked(chosen == name.as_ref())
                    .when(shows, |item| item.face(preview))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let name = name.to_string();
                        this.settings
                            .update(cx, |settings, cx| settings.set_font(name, cx));
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        drop(faced);
        if waiting {
            cx.notify();
        }

        let picker = Picker::new(TYPEFACES, &self.popovers, current)
            .width(Picker::WIDE)
            .menu(self.typefaces.menu("typefaces-menu", Picker::WIDE))
            .items(items)
            .when(barren, |picker| {
                picker
                    .item(MenuItem::new("typeface-empty", t!("settings-typeface-none")).disabled())
            });

        let keys = self.typefaces.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some(name) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_font(name.to_string(), cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(
            t!("settings-typeface"),
            t!("settings-typeface-detail"),
            muted,
            small,
            keys.into_any_element(),
        )
    }

    fn corners_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let picker = Picker::new(CORNERS, &self.popovers, look.rounding.label())
            .width(Picker::NARROW)
            .items(Rounding::ALL.into_iter().map(|rounding| {
                let overrides = overrides.clone();
                MenuItem::new(rounding.id(), rounding.label())
                    .selected(look.rounding == rounding)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_rounding(rounding.id(), cx);
                        });
                        Theme::set(Look { rounding, ..look }, &overrides, cx);
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-corners"),
            t!("settings-corners-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn font_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let step = move |id: &'static str, label: &'static str, delta: f32| {
            let overrides = overrides.clone();
            let wanted = (look.font + delta).clamp(MIN_FONT, MAX_FONT);

            Button::new(id)
                .label(label)
                .small()
                .outline()
                .disabled(wanted == look.font)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_font_size(wanted, cx));
                    Theme::set(
                        Look {
                            font: wanted,
                            ..look
                        },
                        &overrides,
                        cx,
                    );
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("font-smaller", "−", -1.))
            .child(div().child(t!("settings-font-value", size = look.font.round() as i64)))
            .child(step("font-larger", "+", 1.));

        self.row(
            t!("settings-font"),
            t!("settings-font-detail"),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn server_side_decorations_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let enabled = self.settings.read(cx).server_side_decorations();

        self.row(
            t!("settings-server-side-decorations"),
            t!("settings-server-side-decorations-detail"),
            muted,
            small,
            Switch::new("server-side-decorations", enabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let decorations = this.settings.update(cx, |settings, cx| {
                        settings.set_server_side_decorations(!enabled, cx);
                        settings.window_decorations()
                    });
                    window.request_decorations(decorations);
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
    fn decorations_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).window_controls();

        self.row(
            t!("settings-window-controls"),
            t!("settings-window-controls-detail"),
            muted,
            small,
            Switch::new("window-controls", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_window_controls(!on, cx));
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn side_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let left = settings.controls_on_left();
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let shown = !settings.server_side_decorations();
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let shown = settings.window_controls();

        self.row(
            t!("settings-controls-side"),
            t!("settings-controls-side-detail"),
            muted,
            small,
            Button::new("controls-side")
                .label(match left {
                    true => t!("common-left"),
                    false => t!("common-right"),
                })
                .small()
                .outline()
                .disabled(!shown)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_controls_on_left(!left, cx));
                }))
                .into_any_element(),
        )
    }

    fn profile(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        div()
            .flex()
            .items_center()
            .gap_4()
            .child(match self.session.read(cx).state() {
                SessionState::SignedIn(profile) => {
                    Initials::new(profile.display_name.clone(), px(64.)).into_any_element()
                }
                _ => Skeleton::new().size(px(64.)).circle().into_any_element(),
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => div()
                            .child(profile.display_name.clone())
                            .text_size(theme.text(Text::Large))
                            .font_weight(FontWeight::SEMIBOLD)
                            .into_any_element(),
                        _ => Skeleton::new().w(px(140.)).h(px(14.)).into_any_element(),
                    })
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => div()
                            .child(profile.id.clone())
                            .text_color(muted)
                            .text_size(theme.text(Text::Small))
                            .into_any_element(),
                        _ => Skeleton::new().w(px(90.)).h(px(10.)).into_any_element(),
                    }),
            )
    }

    fn theme_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let current = look.kind;
        let adaptive = self.settings.read(cx).adaptive_theme();
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let picker = Picker::new(THEMES, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(ThemeKind::ALL.into_iter().map(|kind| {
                let item = MenuItem::new(kind.id(), kind.label()).selected(current == kind);
                match adaptive
                    && !matches!(kind, ThemeKind::System | ThemeKind::Dark | ThemeKind::Light)
                {
                    true => item.disabled(),
                    false => {
                        let overrides = overrides.clone();
                        item.on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.update(cx, |settings, cx| {
                                settings.set_theme(kind.id(), cx);
                            });
                            Theme::fade(Look { kind, ..look }, &overrides, cx);
                            cx.notify();
                        }))
                    }
                }
            }));

        let settings = self.settings.clone();
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("open-theme-config")
                    .label(t!("settings-theme-config"))
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        let path = settings.update(cx, |settings, _| settings.ensure_file());
                        if let Err(error) = open_settings_file(&path) {
                            eprintln!("sonora: cannot open {}: {error}", path.display());
                        }
                    }),
            )
            .child(picker);

        self.row(
            t!("settings-theme"),
            t!("settings-theme-detail"),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    fn icons_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).icons().to_owned();
        let current = icons::pack(&chosen).unwrap_or_else(icons::active);

        let picker = Picker::new(PACKS, &self.popovers, current.title())
            .width(Picker::REGULAR)
            .items(icons::packs().map(|pack| {
                MenuItem::new(pack.id, pack.title())
                    .selected(pack.id == current.id)
                    .detail(samples(pack, muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_icons(pack.id, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-icons"),
            t!("settings-icons-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn opacity_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();
        let transparency = match look.transparent {
            true => look.transparency,
            false => 0.,
        };
        let value = 1. - transparency / MAX_TRANSPARENCY;
        let percent = ((1. - transparency) * 100.).round() as i64;

        let control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().w(theme.metrics.cover).child(
                    Scrubber::new(&self.opacity, value)
                        .colors(theme.progress_bar, theme.muted, theme.foreground)
                        .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                            let transparency = (1. - *fraction) * MAX_TRANSPARENCY;
                            let transparent = transparency > 0.;
                            this.settings.update(cx, |settings, cx| {
                                settings.set_transparent(transparent, cx);
                                settings.set_transparency(transparency, cx);
                            });
                            Theme::set(
                                Look {
                                    transparent,
                                    transparency,
                                    ..look
                                },
                                &overrides,
                                cx,
                            );
                        })),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.control * 1.5)
                    .whitespace_nowrap()
                    .text_right()
                    .child(t!("settings-opacity-value", percent = percent)),
            );

        self.row(
            t!("settings-opacity"),
            t!("settings-opacity-detail"),
            muted,
            small,
            control.into_any_element(),
        )
    }

    fn adaptive_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).adaptive_theme();
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        self.row(
            t!("settings-adaptive"),
            t!("settings-adaptive-detail"),
            muted,
            small,
            Switch::new("adaptive-theme", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let adaptive = !on;
                    let kind = match adaptive
                        && !matches!(
                            look.kind,
                            ThemeKind::System | ThemeKind::Dark | ThemeKind::Light
                        ) {
                        true => ThemeKind::Dark,
                        false => look.kind,
                    };
                    this.settings.update(cx, |settings, cx| {
                        settings.set_adaptive_theme(adaptive, cx);
                        if kind != look.kind {
                            settings.set_theme(kind.id(), cx);
                        }
                    });
                    if kind != look.kind {
                        Theme::fade(Look { kind, ..look }, &overrides, cx);
                    }
                }))
                .into_any_element(),
        )
    }

    fn visualizer_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).visualizer();

        self.row(
            t!("settings-visualizer"),
            t!("settings-visualizer-detail"),
            muted,
            small,
            Switch::new("visualizer", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_visualizer(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn motion_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).stillness();

        let picker = Picker::new(MOTION, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Stillness::ALL.into_iter().map(|stillness| {
                MenuItem::new(stillness.id(), stillness.label())
                    .selected(current == stillness)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_stillness(stillness, cx);
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-motion"),
            t!("settings-motion-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn pace_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).pace();

        let picker = Picker::new(PACE, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Pace::ALL.into_iter().map(|pace| {
                MenuItem::new(pace.id(), pace.label())
                    .selected(current == pace)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_pace(pace, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-pace"),
            t!("settings-pace-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn saver_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).saver();

        let picker = Picker::new(SAVER, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Saver::ALL.into_iter().map(|saver| {
                MenuItem::new(saver.id(), saver.label())
                    .selected(current == saver)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_saver(saver, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-saver"),
            t!("settings-saver-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn playback_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.playback.read(cx).normalisation();

        self.row(
            t!("settings-normalisation"),
            t!("settings-normalisation-detail"),
            muted,
            small,
            Switch::new("normalisation", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_normalisation(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn adaptive_menu_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).adaptive_menu();

        self.row(
            t!("settings-adaptive-menu"),
            t!("settings-adaptive-menu-detail"),
            muted,
            small,
            Switch::new("adaptive-menu", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_adaptive_menu(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn tray_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).close_to_tray();

        self.row(
            t!("settings-close-to-tray"),
            t!("settings-close-to-tray-detail"),
            muted,
            small,
            Switch::new("close-to-tray", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_close_to_tray(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn gapless_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.playback.read(cx).gapless();

        self.row(
            t!("settings-gapless"),
            t!("settings-gapless-detail"),
            muted,
            small,
            Switch::new("gapless", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_gapless(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn updates_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).check_updates();

        self.row(
            t!("settings-check-updates"),
            t!("settings-check-updates-detail"),
            muted,
            small,
            Switch::new("check-updates", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_check_updates(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn panel_lyrics_size_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.settings.read(cx).panel_lyrics_scale();

        self.lyrics_size_row(
            "panel-lyrics-size",
            "settings-panel-lyrics-size",
            "settings-panel-lyrics-size-detail",
            scale,
            |settings, scale, cx| settings.set_panel_lyrics_scale(scale, cx),
            cx,
        )
    }

    fn fullscreen_lyrics_size_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.settings.read(cx).fullscreen_lyrics_scale();

        self.lyrics_size_row(
            "fullscreen-lyrics-size",
            "settings-fullscreen-lyrics-size",
            "settings-fullscreen-lyrics-size-detail",
            scale,
            |settings, scale, cx| settings.set_fullscreen_lyrics_scale(scale, cx),
            cx,
        )
    }

    fn lyrics_size_row(
        &self,
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        scale: f32,
        apply: fn(&mut AppSettings, f32, &mut Context<AppSettings>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let step = move |suffix: &'static str, label: &'static str, delta: f32| {
            let wanted = (scale + delta).clamp(MIN_LYRICS_SCALE, MAX_LYRICS_SCALE);

            Button::new(SharedString::from(format!("{id}-{suffix}")))
                .label(label)
                .small()
                .outline()
                .disabled(wanted == scale)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| apply(settings, wanted, cx));
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("smaller", "\u{2212}", -0.1))
            .child(div().child(t!(
                "settings-lyrics-size-value",
                size = (scale * 100.).round() as i64
            )))
            .child(step("larger", "+", 0.1));

        self.row(
            i18n::lookup(title, None),
            i18n::lookup(detail, None),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    fn lyrics_for_local_files_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).lyrics_for_local_files();

        self.row(
            t!("settings-lyrics-for-local-files"),
            t!("settings-lyrics-for-local-files-detail"),
            muted,
            small,
            Switch::new("lyrics-for-local-files", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_lyrics_for_local_files(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    fn karaoke_lyrics_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).karaoke_lyrics();

        self.row(
            t!("settings-karaoke-lyrics"),
            t!("settings-karaoke-lyrics-detail"),
            muted,
            small,
            Switch::new("karaoke-lyrics", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_karaoke_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn blur_lyrics_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).blur_lyrics();

        self.row(
            t!("settings-blur-lyrics"),
            t!("settings-blur-lyrics-detail"),
            muted,
            small,
            Switch::new("blur-lyrics", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn romanized_lyrics_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let on = settings.romanized_lyrics();
        let scripts = settings.romanization_scripts();
        let picker = Picker::new(
            "romanization-scripts",
            &self.popovers,
            t!("settings-romanization-writing-systems"),
        )
        .width(Picker::REGULAR)
        .sticky()
        .items(WritingSystem::ALL.map(|writing_system| {
            let (id, label) = romanization_script_copy(writing_system);
            let selected = scripts.contains(writing_system);
            MenuItem::new(id, i18n::lookup(label, None))
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_romanization_script(writing_system, !selected, cx);
                    });
                }))
        }));
        let action = div()
            .flex()
            .items_center()
            .gap_2()
            .when(on, |this| this.child(picker))
            .child(Switch::new("romanized-lyrics", on).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_romanized_lyrics(!on, cx);
                    });
                },
            )));

        self.row(
            t!("settings-romanized-lyrics"),
            t!("settings-romanized-lyrics-detail"),
            muted,
            small,
            action.into_any_element(),
        )
    }

    fn local_folder_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let path = self.session.read(cx).local_path();
        let detail = match &path {
            Some(path) => SharedString::from(path.clone()),
            None => t!("settings-local-folder-empty"),
        };

        let choose = local::choose_button("choose-local-folder")
            .small()
            .outline();

        let rescan = path.is_some().then(|| {
            Button::new("rescan-local-folder")
                .label(t!("settings-rescan"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| this.rescan_local_folder(cx)))
        });

        let clear = path.is_some().then(|| {
            Button::new("clear-local-folder")
                .label(t!("settings-clear-folder"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| this.clear_local_folder(cx)))
        });

        self.row(
            t!("settings-local-folder"),
            detail,
            muted,
            small,
            div()
                .flex()
                .gap_2()
                .child(choose)
                .children(rescan)
                .children(clear)
                .into_any_element(),
        )
    }

    fn rescan_local_folder(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.session.read(cx).local_path() else {
            return;
        };
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| {
                library.rescan_local(PathBuf::from(path), cx)
            });
    }

    fn clear_local_folder(&mut self, cx: &mut Context<Self>) {
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| library.forget_local(cx));
    }

    fn accounts_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let session = self.session.read(cx);
        let pending = session.is_pending();
        let signed_out = matches!(session.state(), SessionState::SignedOut);
        let guest = !session.authenticated();
        let waiting = match session.state() {
            SessionState::Authorizing(prompt) => !matches!(
                prompt,
                Some(SignInPrompt::Secret | SignInPrompt::Accounts(_))
            ),
            _ => false,
        };
        let accounts: Vec<Account> = session
            .providers()
            .map(|info| Account {
                slug: info.slug,
                name: info.name,
                options: info.options,
                stored: info.stored,
                active: info.active && !signed_out,
                guest: info.active && !signed_out && guest,
                cancel: waiting && info.pending,
                error: info.error,
            })
            .collect();
        let mut cards = Vec::new();
        for account in accounts {
            cards.push(self.account_card(account, pending, cx).into_any_element());
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(t!("settings-accounts"))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .text_size(theme.text(Text::Small))
                            .child(t!("settings-accounts-detail")),
                    ),
            )
            .children(cards)
    }

    fn account_card(
        &self,
        account: Account,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let Account {
            slug,
            name,
            options,
            stored,
            active,
            guest,
            cancel,
            error,
        } = account;
        let status = match (active, guest, stored) {
            (true, true, _) => t!("settings-provider-guest"),
            (true, false, _) => t!("settings-provider-current"),
            (false, _, true) => t!("settings-provider-connected"),
            (false, _, false) => t!("settings-provider-none"),
        };
        let methods: Vec<SignIn> = options
            .into_iter()
            .filter(|option| offered(option, stored, guest))
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .p(theme.metrics.pad)
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pl_2()
                    .child(
                        svg()
                            .path(icons::path(crate::shared::provider_logo(slug)))
                            .size(theme.metrics.control_small)
                            .flex_none()
                            .text_color(theme.foreground),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().font_weight(FontWeight::MEDIUM).child(name))
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .text_size(theme.text(Text::Small))
                                    .child(status),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_2()
                            .when(stored && !active, |this| {
                                this.child(
                                    Button::new(SharedString::from(format!("switch-{slug}")))
                                        .label(t!("settings-provider-switch"))
                                        .small()
                                        .outline()
                                        .disabled(pending)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.session
                                                .update(cx, |session, cx| session.switch(slug, cx));
                                        })),
                                )
                            })
                            .when(stored, |this| {
                                this.child(
                                    Button::new(SharedString::from(format!("sign-out-{slug}")))
                                        .label(t!("settings-sign-out"))
                                        .small()
                                        .ghost()
                                        .icon("icons/log-out.svg")
                                        .disabled(pending)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.session
                                                .update(cx, |session, cx| session.forget(slug, cx));
                                        })),
                                )
                            }),
                    ),
            )
            .when_some(error, |this, error| {
                this.child(crate::shared::trouble::trouble(error, false))
            })
            .when(!methods.is_empty(), |this| {
                this.child(
                    div().flex().flex_wrap().items_start().gap_2().children(
                        methods
                            .into_iter()
                            .map(|method| self.method(slug, name, method, pending, cx)),
                    ),
                )
            })
            .when(cancel, |this| {
                this.child(
                    div().child(
                        Button::new(SharedString::from(format!("cancel-{slug}")))
                            .label(t!("common-cancel"))
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| this.abandon(cx))),
                    ),
                )
            })
    }

    fn abandon(&mut self, cx: &mut Context<Self>) {
        self.secret.update(cx, |input, cx| input.set_text("", cx));
        self.session
            .update(cx, |session, cx| session.cancel_sign_in(cx));
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.secret.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.secret.update(cx, |input, cx| input.set_text("", cx));
        self.session
            .update(cx, |session, cx| session.submit_input(text, cx));
    }

    fn secret_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let slug = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.pending)
            .map(|info| info.slug)
            .unwrap_or("youtube");
        let hint = match slug {
            "deezer" => "login-arl-hint",
            _ => "login-cookie-hint",
        };
        self.secret
            .update(cx, |input, cx| input.set_hint(hint, cx));
        CookiePrompt::new(self.secret.clone())
            .provider(slug)
            .on_submit(cx.listener(|this, _, _, cx| this.submit(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    fn method(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.method_button(slug, provider, method, pending, cx)
            .into_any_element()
    }

    fn method_button(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (id, label) = match &method {
            SignIn::Default => (
                format!("connect-{slug}"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Anonymous => (format!("connect-{slug}-guest"), t!("login-guest-use")),
            SignIn::Secret => (
                format!("connect-{slug}-cookies"),
                match slug {
                    "deezer" => t!("login-connect-arl"),
                    _ => t!("login-connect-cookies"),
                },
            ),
            SignIn::Path(_) => (
                format!("connect-{slug}-path"),
                t!("login-sign-in", provider = provider),
            ),
        };

        Button::new(SharedString::from(id))
            .label(label)
            .small()
            .outline()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                let method = method.clone();
                this.session
                    .update(cx, |session, cx| session.sign_in(slug, method, cx));
            }))
    }

    fn version_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        self.row(
            t!("settings-version"),
            t!("settings-version-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            div().child(VERSION).into_any_element(),
        )
    }

    fn license_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        self.row(
            t!("settings-license"),
            t!("settings-license-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            Button::new("license")
                .label(t!("settings-license-view"))
                .small()
                .outline()
                .icon("icons/link.svg")
                .on_click(|_, _, cx| cx.open_url(LICENSE_URL))
                .into_any_element(),
        )
    }

    fn source_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        self.row(
            t!("settings-source"),
            t!("settings-source-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            Button::new("source")
                .label(t!("settings-source-view"))
                .small()
                .outline()
                .icon("icons/link.svg")
                .on_click(|_, _, cx| cx.open_url(SOURCE_URL))
                .into_any_element(),
        )
    }

    fn notice(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .text_color(theme.muted_foreground)
            .text_size(theme.text(Text::Small))
            .child(t!("settings-notice"))
    }

    fn team(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        InfoCard::new(t!("settings-team")).flex_none().child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(MEMBERS.into_iter().enumerate().map(|(index, member)| {
                    div()
                        .id(("team-member", index))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px(theme.metrics.pad)
                        .py(theme.metrics.pad / 2.)
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.secondary_hover))
                        .on_click(move |_, _, cx| cx.open_url(member.profile))
                        .child(Avatar::new(Some(member.avatar)).size(theme.metrics.thumb))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().font_weight(FontWeight::MEDIUM).child(member.login))
                                .child(
                                    div()
                                        .text_size(theme.text(Text::Small))
                                        .text_color(theme.muted_foreground)
                                        .child(t!("settings-team-github")),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(theme.text(Text::Small))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground)
                                .child(member.role.label()),
                        )
                })),
        )
    }

    fn row(
        &self,
        title: SharedString,
        detail: SharedString,
        muted: gpui::Hsla,
        small: Pixels,
        action: gpui::AnyElement,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(muted)
                            .text_size(small)
                            .child(detail),
                    ),
            )
            .child(div().flex_none().child(action))
    }

    fn account_modal(
        &self,
        accounts: Vec<AccountChoice>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        AccountPicker::new(accounts)
            .on_pick(cx.listener(|this, id: &SharedString, _, cx| {
                let id = id.to_string();
                this.session
                    .update(cx, |session, cx| session.submit_input(id, cx));
            }))
            .on_cancel(cx.listener(|this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.cancel_sign_in(cx));
            }))
    }
}

fn romanization_script_copy(writing_system: WritingSystem) -> (&'static str, &'static str) {
    match writing_system {
        WritingSystem::Japanese => ("romanization-japanese", "settings-romanization-japanese"),
        WritingSystem::Chinese => ("romanization-chinese", "settings-romanization-chinese"),
        WritingSystem::Korean => ("romanization-korean", "settings-romanization-korean"),
        WritingSystem::Cyrillic => ("romanization-cyrillic", "settings-romanization-cyrillic"),
        WritingSystem::Greek => ("romanization-greek", "settings-romanization-greek"),
        WritingSystem::Arabic => ("romanization-arabic", "settings-romanization-arabic"),
        WritingSystem::Other => ("romanization-other", "settings-romanization-other"),
    }
}

fn samples(pack: &'static icons::Pack, tint: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .children(icons::SAMPLES.iter().map(|name| {
            svg()
                .path(icons::shown(pack, name))
                .size(px(14.))
                .flex_none()
                .text_color(tint)
        }))
}

fn open_settings_file(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()?;

    #[cfg(target_os = "macos")]
    Command::new("open").arg(path).spawn()?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open").arg(path).spawn()?;

    Ok(())
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.installed.is_none() {
            self.installed = Some(usable_fonts(window, cx));
        }

        let chosen_language = self.settings.read(cx).language();
        let language_selected = Language::ALL
            .into_iter()
            .position(|language| language.id() == chosen_language)
            .map(|place| place + 1)
            .or(Some(0));
        self.languages.sync(
            self.popovers.shows(LANGUAGES),
            language_selected,
            window,
            cx,
        );

        let chosen_typeface = self.settings.read(cx).font();
        let typeface_selected = self
            .typeface_entries()
            .iter()
            .position(|name| name.as_ref() == chosen_typeface);
        self.typefaces.sync(
            self.popovers.shows(TYPEFACES),
            typeface_selected,
            window,
            cx,
        );

        let accounts = match self.session.read(cx).state() {
            SessionState::Authorizing(Some(SignInPrompt::Accounts(accounts))) => {
                Some(accounts.clone())
            }
            _ => None,
        };
        let secret = matches!(
            self.session.read(cx).state(),
            SessionState::Authorizing(Some(SignInPrompt::Secret))
        );

        div()
            .relative()
            .size_full()
            .child(
                Scroller::new("settings", &self.scrollbar)
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .w_full()
                            .max_w(px(640.))
                            .p_6()
                            .when(self.tab == SettingsTab::General, |this| {
                                this.child(self.profile(cx))
                                    .child(Separator::horizontal().w_full())
                            })
                            .child(self.panel(cx))
                            .when(self.tab == SettingsTab::About, |this| {
                                this.child(self.team(cx)).child(self.notice(cx))
                            }),
                    ),
            )
            .when_some(accounts, |this, accounts| {
                this.child(self.account_modal(accounts, cx).into_any_element())
            })
            .when(secret, |this| {
                this.child(self.secret_prompt(cx).into_any_element())
            })
    }
}

fn usable_fonts(window: &Window, cx: &App) -> Vec<SharedString> {
    let missing = resolved(window, "sonora-has-no-such-family");
    let mut names = cx.text_system().all_font_names();
    names.sort_unstable();
    names.dedup();

    names
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .filter(|name| resolved(window, name) != missing)
        .map(SharedString::from)
        .collect()
}

fn resolved(window: &Window, family: &str) -> Option<gpui::FontId> {
    let run = TextRun {
        len: 1,
        font: font(SharedString::from(family.to_owned())),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    window
        .text_system()
        .shape_line(SharedString::from("A"), px(12.), &[run], None)
        .runs
        .first()
        .map(|run| run.font_id)
}
