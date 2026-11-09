//! What a screen says when it cannot sign in or cannot load: the sign-in failure with guidance
//! for known account problems, the page-filling No connection and offline states, and the
//! one-line reason for places with room for little.

use i18n::t;
use music::SignInProblem;
use rune::{Cursor, El, col, text};
use state::Failure;
use ui::{Button, Notice, Text, Theme, Vacancy, Weight};

use crate::shared::dialogs::rems;
use crate::{Ui, act};

/// Where Apple's subscription page is.
const APPLE_MUSIC: &str = "https://music.apple.com/";

/// A sign-in failure, with recovery guidance for known account problems. Connection failures
/// keep their own title and icon because signing in again will not help.
pub(crate) fn trouble(failure: Failure, centered: bool, theme: &Theme) -> El {
    if failure.problem == Some(SignInProblem::Subscription) {
        let message = t!("login-problem-subscription").into_owned();
        let mut column = col()
            .key("subscription-failure")
            .role("alert")
            .label(message.clone())
            .gap(rems(theme, 0.25))
            .child(
                text(message)
                    .font(ui::font(theme, Text::Small, Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .wrap()
                    .a11y_hidden(),
            )
            .child(
                Button::new("check-apple-subscription")
                    .label(t!("login-subscription-check"))
                    .icon("icons/external-link.svg")
                    .ghost()
                    .small()
                    .tint(theme.muted_foreground)
                    .on_press(|_: &mut Ui| {
                        if let Err(error) = open::that(APPLE_MUSIC) {
                            log::warn!("login: cannot open the subscription page: {error}");
                        }
                    })
                    .build(theme)
                    .cursor(Cursor::Pointer),
            );
        if centered {
            column = column.items_center();
        }
        return column;
    }
    let offline = failure.offline();
    let Failure {
        problem,
        summary,
        detail,
    } = failure;

    let (title, message) = match (offline, problem) {
        (true, _) => (
            t!("trouble-offline").into_owned(),
            t!("trouble-offline-detail").into_owned(),
        ),
        (false, Some(problem)) => (
            t!("login-failed-title").into_owned(),
            i18n::lookup(reason(problem), None).into_owned(),
        ),
        (false, None) => (
            t!("login-failed-title").into_owned(),
            sentence(summary, detail),
        ),
    };

    let mut notice = Notice::new(title, message)
        .spoken("trouble-notice")
        .failed();
    if offline {
        notice = notice.icon("icons/wifi-off.svg");
    }
    if centered {
        notice = notice.centered();
    }
    notice.build(theme)
}

/// Whether a page built from `id` cannot be read at all because the network is gone. A local
/// id never is, since nothing about it leaves the machine.
pub(crate) fn unreachable(ui: &Ui, id: &str) -> bool {
    !music::is_local_id(id) && ui.store.network.lost()
}

/// The page-filling state for a screen that cannot load, with a button that tries again.
/// `reason` is what a load came back with, or `None` when the app already knows the network is
/// gone and the screen never tried. A reason that reads as a lost connection becomes the same
/// No connection state; anything else keeps its own text under `label`, which is why every
/// caller passes the caption for its own page rather than one shared line. In offline mode
/// every screen says so instead, with a button that goes online. `retry` runs through `act`.
pub(crate) fn lost(
    ui: &Ui,
    key: &str,
    label: impl Into<String>,
    reason: Option<&str>,
    retry: impl Fn(&mut Ui) + 'static,
) -> El {
    let theme = &ui.theme;
    if ui.store.network.offline() {
        return offline(theme);
    }
    let vacancy = match reason.filter(|reason| !music::trouble::offline(reason)) {
        None => Vacancy::new(t!("trouble-offline"))
            .detail(t!("trouble-offline-detail"))
            .icon("icons/wifi-off.svg"),
        Some(reason) => Vacancy::new(label)
            .detail(reason.to_owned())
            .icon("icons/circle-alert.svg"),
    };

    vacancy
        .spoken(format!("{key}-caption"))
        .action(
            Button::new(key)
                .label(t!("trouble-retry"))
                .icon("icons/refresh-cw.svg")
                .outline()
                .on_press(act(retry))
                .build(theme),
        )
        .build(theme)
}

/// The page-filling state for offline mode, with a button that turns it off.
pub(crate) fn offline(theme: &Theme) -> El {
    Vacancy::new(t!("offline-title"))
        .spoken("offline-caption")
        .icon("icons/wifi-off.svg")
        .action(
            Button::new("go-online")
                .label(t!("offline-go-online"))
                .outline()
                .on_press(act(|ui: &mut Ui| ui.store.set_offline(false)))
                .build(theme),
        )
        .build(theme)
}

/// The failure's message alone, for the places that only have room for one line.
pub(crate) fn short(failure: &Failure) -> String {
    match failure.problem {
        Some(problem) => i18n::lookup(reason(problem), None).into_owned(),
        None => sentence(failure.summary.clone(), failure.detail.clone()),
    }
}

fn sentence(summary: String, detail: Option<String>) -> String {
    let summary = summary.trim_end_matches('.').to_owned();
    match detail.map(|detail| unwrapped_reason(&detail)) {
        Some(reason) => format!("{summary}: {reason}"),
        None => format!("{summary}."),
    }
}

/// The reason inside a provider's wrapped error text, as a sentence that starts lowercase.
fn unwrapped_reason(text: &str) -> String {
    let inner = text
        .split_once('{')
        .and_then(|(_, rest)| rest.rsplit_once('}'))
        .map(|(inner, _)| inner)
        .unwrap_or(text)
        .trim();
    let reason = inner
        .split_once("with reason:")
        .map(|(_, reason)| reason)
        .unwrap_or(inner)
        .trim()
        .trim_end_matches('.');
    let mut letters = reason.chars();
    match letters.next() {
        Some(first) => format!("{}{}.", first.to_lowercase(), letters.as_str()),
        None => text.to_owned(),
    }
}

fn reason(problem: SignInProblem) -> &'static str {
    match problem {
        SignInProblem::Premium => "login-problem-premium",
        SignInProblem::Subscription => "login-problem-subscription",
        SignInProblem::Region => "login-problem-region",
        SignInProblem::Credentials => "login-problem-credentials",
        SignInProblem::Network => "login-problem-network",
        SignInProblem::Cancelled => "login-problem-cancelled",
        SignInProblem::Refused => "login-problem-refused",
    }
}
