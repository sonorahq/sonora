use gpui::prelude::*;
use gpui::{AnyElement, SharedString};
use i18n::t;
use music::SignInProblem;
use state::Failure;
use ui::Notice;

pub(crate) fn trouble(failure: Failure, centered: bool) -> AnyElement {
    let Failure {
        problem,
        summary,
        detail,
    } = failure;

    let message = match problem {
        Some(problem) => i18n::lookup(reason(problem), None),
        None => SharedString::from(sentence(summary, detail)),
    };

    Notice::new(t!("login-failed-title"), message)
        .failed()
        .when(centered, Notice::centered)
        .into_any_element()
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
        SignInProblem::Region => "login-problem-region",
        SignInProblem::Credentials => "login-problem-credentials",
        SignInProblem::Network => "login-problem-network",
        SignInProblem::Cancelled => "login-problem-cancelled",
        SignInProblem::Refused => "login-problem-refused",
    }
}
