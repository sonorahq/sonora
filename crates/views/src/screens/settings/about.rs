//! The About page: the app's mark and name, its version and updates, the log, the licence and
//! the source, the people behind it, and the notice under them.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use i18n::t;
use rune::{Cursor, Cx, El, Transition, Tx, col, icon, row, text};
use state::Channel;
use ui::{Avatar, Text, Theme, Weight, font};

use super::{GROUPS, Layout, Row, group, heading, outward, segmented, switch, whole};
use crate::Ui;

/// How large the app's mark stands, in covers, and how much of the tile the mark takes.
const MARK_TILE: f32 = 0.5;
const MARK_SHARE: f32 = 0.56;
/// How round the tile's corners are, as a share of its side.
const MARK_ROUNDING: f32 = 0.22;
/// The space between the mark and the name.
const MARK_GAP: f32 = 20.;
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
const SOURCE_URL: &str = "https://github.com/sonorahq/sonora";
/// How long each member of the team waits after the one above to slide in, in seconds.
const STAGGER: f32 = 0.03;

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
    fn label(self) -> String {
        match self {
            Self::LeadMaintainer => t!("settings-role-lead-maintainer"),
            Self::Maintainer => t!("settings-role-maintainer"),
            Self::Contributor => t!("settings-role-contributor"),
        }
        .into_owned()
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

/// The About page: the mark, the rows, then the team and the notice.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>, _layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut rows = vec![
        Row::new(
            "version",
            t!("settings-version"),
            ui::spoken("settings-version-value", state::version(), theme)
                .color(theme.muted_foreground.color()),
        )
        .build(theme),
        switch(
            theme,
            "check-updates",
            t!("settings-check-updates"),
            settings.check_updates(),
            |ui, on| ui.store.settings.set_check_updates(on, &mut ui.store.cx),
        ),
    ];
    if state::updates_itself() {
        rows.push(
            Row::new(
                "update-channel",
                t!("settings-update-channel"),
                segmented(
                    theme,
                    "update-channel",
                    t!("settings-update-channel"),
                    Channel::ALL.map(|channel| (channel, channel.label().into_owned())),
                    settings.update_channel(),
                    |ui, channel| {
                        ui.store
                            .settings
                            .set_update_channel(channel, &mut ui.store.cx)
                    },
                ),
            )
            .build(theme),
        );
    }
    rows.extend([log_row(theme), license_row(theme), source_row(theme)]);
    vec![
        mark(theme),
        group(theme, None, None, rows),
        col()
            .key("settings-about-footer")
            .w_full()
            .gap(GROUPS)
            .child(team(ui))
            .child(notice(theme)),
    ]
}

/// The row that opens a copy of the current log file in whatever the system reads text with.
/// It is disabled when the platform names no state or cache folder to log into.
fn log_row(theme: &Theme) -> El {
    let path = state::log_file();
    let missing = path.is_none();
    whole(
        outward("log", t!("settings-log-open"), None)
            .disabled(missing)
            .on_press(move |_: &mut Ui| {
                let Some(path) = &path else { return };
                if let Err(error) = snapshot(path).and_then(open::that_detached) {
                    log::warn!(
                        "settings: cannot open a copy of {}: {error}",
                        path.display()
                    );
                }
            }),
        theme,
    )
}

fn license_row(theme: &Theme) -> El {
    whole(
        outward(
            "license",
            t!("settings-license"),
            Some(t!("settings-license-short").into_owned()),
        )
        .on_press_tx(|_: &mut Ui, tx: &mut Tx| open_url(tx, LICENSE_URL)),
        theme,
    )
}

fn source_row(theme: &Theme) -> El {
    whole(
        outward("source", t!("settings-source"), None)
            .on_press_tx(|_: &mut Ui, tx: &mut Tx| open_url(tx, SOURCE_URL)),
        theme,
    )
}

/// Opens About with the app's mark beside its name, aligned with the settings below: the
/// mark in black on white, as the app icon draws it whatever the theme.
fn mark(theme: &Theme) -> El {
    let tile = theme.metrics.cover * MARK_TILE;
    let name = t!("settings-about-name").into_owned();
    row()
        .key("settings-about-mark")
        .center_y()
        .gap(MARK_GAP)
        .child(
            row()
                .size(tile, tile)
                .center()
                .radius(tile * MARK_ROUNDING)
                .fill(ui::white().color())
                .a11y_hidden()
                .child(
                    icon(icons::get("icons/sonora.svg"), tile * MARK_SHARE)
                        .color(ui::black().color()),
                ),
        )
        .child(
            text(name.clone())
                .key("settings-about-name")
                .role("heading")
                .level(2)
                .label(name)
                .font(font(theme, Text::Title, Weight::SEMIBOLD))
                .color(theme.foreground.color()),
        )
}

/// The notice under the team.
fn notice(theme: &Theme) -> El {
    let words = t!("settings-notice").into_owned();
    text(words.clone())
        .key("settings-notice")
        .role("paragraph")
        .label(words)
        .wrap()
        .font(font(theme, Text::Small, Weight::NORMAL))
        .color(theme.muted_foreground.color())
}

/// The people behind Sonora, each opening their GitHub profile.
fn team(ui: &Ui) -> El {
    let theme = &ui.theme;
    let count = MEMBERS.len();
    col()
        .key("settings-team")
        .role("list")
        .label(t!("settings-team"))
        .w_full()
        .child(heading(
            theme,
            "settings-team",
            Some("icons/user-round.svg"),
        ))
        .children(MEMBERS.into_iter().enumerate().map(|(index, member)| {
            row()
                .key(format!("team-member-{}", member.login))
                .role("link")
                .label(member.login)
                .description(member.role.label())
                .position_in_set(index + 1, count)
                .focusable(true)
                .w_full()
                .center_y()
                .gap(theme.font_size * 0.75)
                .h(theme.metrics.list_row)
                .mx(-theme.font_size * 0.75)
                .px(theme.font_size * 0.75)
                .radius(theme.radius)
                .cursor(Cursor::Pointer)
                .hover(theme.secondary_hover.color())
                .press_scale(ui::PRESS_SCALE_CARD)
                .transition(Transition::offset(0., 6.).combined(Transition::opacity()))
                .enter_delay(STAGGER * index as f32)
                .on_press_tx(move |_: &mut Ui, tx: &mut Tx| open_url(tx, member.profile))
                .child(
                    Avatar::new(ui.artwork(), Some(member.avatar))
                        .size(theme.metrics.thumb)
                        .build(theme),
                )
                .child(
                    text(member.login)
                        .font(font(theme, Text::Body, Weight::MEDIUM))
                        .color(theme.foreground.color())
                        .truncate()
                        .grow(1.)
                        .shrink()
                        .min_w(0.)
                        .a11y_hidden(),
                )
                .child(
                    text(member.role.label())
                        .font(font(theme, Text::Label, Weight::NORMAL))
                        .color(theme.muted_foreground.color())
                        .a11y_hidden(),
                )
        }))
}

/// Opens a page in the browser, saying in the log when it cannot.
fn open_url(tx: &mut Tx, url: &str) {
    if let Err(error) = tx.open_url(url) {
        log::warn!("settings: cannot open {url}: {error}");
    }
}

/// Notepad on Windows refuses a file that Sonora still writes to, so the button opens a copy.
/// `File::open` shares write access, and `fs::copy` documents no share mode.
fn snapshot(log: &Path) -> io::Result<PathBuf> {
    let copy = log.with_file_name("sonora.copy.log");
    io::copy(&mut File::open(log)?, &mut File::create(&copy)?)?;
    Ok(copy)
}
