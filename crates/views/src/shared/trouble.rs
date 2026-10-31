use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, ElementId, SharedString, Window};
use i18n::t;
use music::SignInProblem;
use state::{Failure, Network};
use ui::{Button, Notice, Vacancy};

/// The sign-in failure a screen puts up. One that only says the provider could not be reached
/// wears the No connection title and the crossed-out wifi mark, since the account itself is
/// fine and nothing about it has to be done again.
pub(crate) fn trouble(failure: Failure, centered: bool) -> AnyElement {
    let offline = failure.offline();
    let Failure {
        problem,
        summary,
        detail,
    } = failure;

    let (title, message) = match (offline, problem) {
        (true, _) => (t!("trouble-offline"), t!("trouble-offline-detail")),
        (false, Some(problem)) => (
            t!("login-failed-title"),
            i18n::lookup(reason(problem), None),
        ),
        (false, None) => (
            t!("login-failed-title"),
            SharedString::from(sentence(summary, detail)),
        ),
    };

    Notice::new(title, message)
        .spoken("trouble-notice")
        .failed()
        .when(offline, |notice| notice.icon("icons/wifi-off.svg"))
        .when(centered, Notice::centered)
        .into_any_element()
}

/// Whether a page built from `id` cannot be read at all because the network is gone. A local
/// id never is, since nothing about it leaves the machine.
pub(crate) fn unreachable(id: &str, cx: &App) -> bool {
    !music::is_local_id(id) && Network::lost(cx)
}

/// The page-filling state for a screen that cannot load, with a button that tries again.
/// `reason` is what a load came back with, or `None` when the app already knows the network is
/// gone and the screen never tried. A reason that reads as a lost connection becomes the same
/// No connection state; anything else keeps its own text under `label`, which is why every
/// caller passes the caption for its own page rather than one shared line. In offline mode
/// every screen says so instead, with a button that goes online.
pub(crate) fn lost(
    id: impl Into<ElementId>,
    label: SharedString,
    reason: Option<&str>,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Vacancy {
    if Network::offline(cx) {
        return offline();
    }
    let id: ElementId = id.into();
    let vacancy = match reason.filter(|reason| !music::trouble::offline(reason)) {
        None => Vacancy::new(t!("trouble-offline"))
            .detail(t!("trouble-offline-detail"))
            .icon("icons/wifi-off.svg"),
        Some(reason) => Vacancy::new(label)
            .detail(SharedString::from(reason.to_owned()))
            .icon("icons/circle-alert.svg"),
    };

    vacancy.spoken((id.clone(), "caption")).action(
        Button::new(id)
            .label(t!("trouble-retry"))
            .icon("icons/refresh-cw.svg")
            .outline()
            .on_click(retry),
    )
}

/// The page-filling state for offline mode, with a button that turns it off.
pub(crate) fn offline() -> Vacancy {
    Vacancy::new(t!("offline-title"))
        .spoken("offline-caption")
        .icon("icons/wifi-off.svg")
        .action(
            Button::new("go-online")
                .label(t!("offline-go-online"))
                .outline()
                .on_click(|_, _, cx| {
                    let network = Network::global(cx);
                    network.update(cx, |network, cx| network.set_offline(false, cx));
                }),
        )
}

/// The failure's message alone, for the places that only have room for one line.
pub(crate) fn short(failure: &Failure) -> SharedString {
    match failure.problem {
        Some(problem) => i18n::lookup(reason(problem), None),
        None => SharedString::from(sentence(failure.summary.clone(), failure.detail.clone())),
    }
}

fn sentence(summary: String, detail: Option<String>) -> String {
    let summary = summary.trim_end_matches('.').to_owned();
    match detail.map(|detail| unwrapped_reason(&detail)) {
        Some(reason) => format!("{summary}: {reason}"),
        None => format!("{summary}."),
    }
}

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
        SignInProblem::Region => "login-problem-region",
        SignInProblem::Credentials => "login-problem-credentials",
        SignInProblem::Network => "login-problem-network",
        SignInProblem::Cancelled => "login-problem-cancelled",
        SignInProblem::Refused => "login-problem-refused",
    }
}
