use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, Window, div, img};
use i18n::t;
use state::{Channel, UpdateState};
use ui::{ActiveTheme as _, Avatar, Button, Glowing as _, Switch, Text};

use super::rows::{Row, outward, segmented};
use super::{Group, SettingsView, Slot, beside, group_label};
use crate::shared::cells::relative_stamp;

/// How large the app icon stands, in covers.
const MARK_TILE: f32 = 0.5;
/// The app icon as this platform shows it, registered by the binary's asset source.
const APP_ICON: &str = "brand/sonora.svg";
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
const SOURCE_URL: &str = "https://github.com/sonorahq/sonora";

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

impl SettingsView {
    pub(super) fn about_groups(&self) -> Vec<Group> {
        let itself = state::updates_itself()
            .then_some([Slot::AutoUpdate, Slot::Channel])
            .into_iter()
            .flatten();
        vec![
            Group::new("settings-group-updates")
                .rows([Slot::Version, Slot::Updates].into_iter().chain(itself)),
            Group::new("settings-group-project").rows([Slot::Log, Slot::License, Slot::Source]),
        ]
    }

    /// The version row: the build running, which for a nightly also names its commit, what
    /// the last look for updates found, and the button that looks again now.
    pub(super) fn version_row(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let version: SharedString = match state::commit() {
            Some(commit) => t!(
                "settings-version-commit",
                version = state::version(),
                commit = commit
            ),
            None => state::version().into(),
        };
        let updates = self.updates.read(cx);
        let checking = updates.checking();
        let status = match (updates.state(), checking) {
            (_, true) => t!("settings-update-status-checking"),
            (UpdateState::Offered(release), false) => t!(
                "settings-update-status-found",
                version = release.version.as_str()
            ),
            (UpdateState::Ready(release), false) => t!(
                "settings-update-status-ready",
                version = release.version.as_str()
            ),
            (UpdateState::Fetching(release), false) => {
                t!("update-downloading", name = release.version.as_str())
            }
            (UpdateState::Failed, false) => t!("update-failed"),
            (UpdateState::Quiet, false) if !updates.can_check() => {
                t!("settings-update-status-managed")
            }
            (UpdateState::Quiet, false) if updates.check_failed() => {
                t!("settings-update-status-failed")
            }
            (UpdateState::Quiet, false) if updates.found().is_some() => t!(
                "settings-update-status-found",
                version = updates.found().unwrap_or_default()
            ),
            (UpdateState::Quiet, false) => match updates.checked_at() {
                Some(at) => {
                    let when = relative_stamp(Some(at));
                    t!("settings-update-status-latest", when = when.as_ref())
                }
                None => t!("settings-update-status-never"),
            },
        };
        let can_check = updates.can_check()
            && !matches!(
                updates.state(),
                UpdateState::Fetching(_) | UpdateState::Ready(_)
            );

        let turn = {
            let mut spin = self.update_spin.borrow_mut();
            match checking {
                true => spin.start(),
                false => spin.stop(),
            }
            spin.turn(window, cx)
        };
        let check = Button::new("check-updates-now")
            .outline()
            .small()
            .icon("icons/refresh-cw.svg")
            .turned(turn)
            .label(t!("settings-check-updates"))
            .disabled(checking || !can_check)
            .on_click(cx.listener(|this, _, _, cx| {
                this.updates.update(cx, |updates, cx| updates.check(cx));
            }));

        Row::new(
            t!("settings-version-title", version = version.as_ref()),
            check.into_any_element(),
        )
        .detail(status)
        .into_any_element()
    }

    pub(super) fn updates_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).check_updates();

        self.row(
            t!("settings-auto-check"),
            Switch::new("check-updates", on)
                .name(t!("settings-auto-check"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_check_updates(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn auto_update_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).auto_update();

        self.row(
            t!("settings-auto-update"),
            Switch::new("auto-update", on)
                .name(t!("settings-auto-update"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_auto_update(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn channel_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.read(cx).update_channel();

        let choices = segmented(
            "update-channel",
            Channel::ALL.map(|channel| (channel, channel.label())),
            current,
            cx.listener(|this, channel: &Channel, _, cx| {
                let channel = *channel;
                this.settings
                    .update(cx, |settings, cx| settings.set_update_channel(channel, cx));
            }),
        );

        self.row(
            t!("settings-update-channel"),
            choices
                .name(t!("settings-update-channel"))
                .into_any_element(),
        )
    }

    /// The row that opens a copy of the current log file in whatever the system reads text
    /// with. It is disabled when the platform names no state or cache folder to log into.
    pub(super) fn log_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let path = state::log_file();

        outward("open-log", t!("settings-log-open"), None, cx)
            .disabled(path.is_none())
            .on_click(move |_, _, _| {
                let Some(path) = &path else { return };
                if let Err(error) = snapshot(path).and_then(open::that_detached) {
                    log::warn!(
                        "settings: cannot open a copy of {}: {error}",
                        path.display()
                    );
                }
            })
            .into_any_element()
    }

    pub(super) fn license_row(&self, cx: &mut Context<Self>) -> AnyElement {
        outward(
            "license",
            t!("settings-license"),
            Some(t!("settings-license-short")),
            cx,
        )
        .on_click(|_, _, cx| cx.open_url(LICENSE_URL))
        .into_any_element()
    }

    pub(super) fn source_row(&self, cx: &mut Context<Self>) -> AnyElement {
        outward("source", t!("settings-source"), None, cx)
            .on_click(|_, _, cx| cx.open_url(SOURCE_URL))
            .into_any_element()
    }

    /// Opens About with the app icon beside its name, aligned with the settings below. The icon
    /// is the one this platform shows, in its own colours whatever the theme.
    pub(super) fn mark(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let tile = theme.metrics.cover * MARK_TILE;

        div()
            .flex()
            .items_center()
            .gap_5()
            .child(img(APP_ICON).size(tile).flex_none())
            .child(
                div()
                    .id("settings-about-name")
                    .role(gpui::accesskit::Role::Heading)
                    .aria_level(2)
                    .aria_label(t!("common-sonora"))
                    .text_size(theme.text(Text::Title))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("common-sonora")),
            )
    }

    pub(super) fn notice(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .id("settings-notice")
            .role(gpui::accesskit::Role::Paragraph)
            .aria_label(t!("settings-notice"))
            .text_color(theme.muted_foreground)
            .text_size(theme.text(Text::Small))
            .child(t!("settings-notice"))
    }

    /// The people behind Sonora, each opening their GitHub profile, beside the name of the list
    /// as a group of the page stands beside its name.
    pub(super) fn team(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let heading = group_label("settings-team".into(), t!("settings-team"), cx);

        let members = div()
            .flex()
            .flex_col()
            .children(MEMBERS.into_iter().enumerate().map(|(index, member)| {
                div()
                    .id(("team-member", index))
                    .role(gpui::accesskit::Role::Link)
                    .aria_label(member.login)
                    .aria_description(member.role.label())
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(MEMBERS.len())
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(theme.metrics.list_row)
                    .mx_neg_3()
                    .px_3()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_url(member.profile))
                    .child(Avatar::new(Some(member.avatar)).size(theme.metrics.thumb))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(member.login),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme.text(Text::Label))
                            .text_color(theme.muted_foreground)
                            .child(member.role.label()),
                    )
                    .glowed(theme.secondary_hover)
            }));
        beside(
            Some(heading.into_any_element()),
            true,
            true,
            members,
            window,
            cx,
        )
    }
}

/// Notepad on Windows refuses a file that Sonora still writes to, so the button opens a copy.
/// `File::open` shares write access, and `fs::copy` documents no share mode.
fn snapshot(log: &Path) -> io::Result<PathBuf> {
    let copy = log.with_file_name("sonora.copy.log");
    io::copy(&mut File::open(log)?, &mut File::create(&copy)?)?;
    Ok(copy)
}
