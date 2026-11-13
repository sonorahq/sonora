use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, FontWeight, IntoElement, Pixels, RenderOnce, SharedString, Window, div, px, svg};
use ui::{ActiveTheme as _, Separator, Text};

const BADGE: Pixels = px(28.);
/// The check in the badge of a step that is done.
const CHECK: Pixels = px(14.);

/// A numbered list of instructions. Screen readers meet it as a list whose items carry the
/// step's text and their place in it.
#[derive(IntoElement)]
pub(crate) struct Steps {
    items: Vec<SharedString>,
}

pub(crate) fn steps(items: impl IntoIterator<Item = impl Into<SharedString>>) -> Steps {
    Steps {
        items: items.into_iter().map(Into::into).collect(),
    }
}

impl RenderOnce for Steps {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let total = self.items.len();
        div()
            .id("steps")
            .role(Role::List)
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .children(self.items.into_iter().enumerate().map(|(i, text)| {
                div()
                    .id(("step", i))
                    .role(Role::ListItem)
                    .aria_label(text.clone())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(total)
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .min_w_0()
                    .rounded(theme.radius)
                    .p_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_none()
                            .size(BADGE)
                            .rounded_full()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .text_color(theme.foreground)
                            .text_size(theme.text(Text::Small))
                            .font_weight(FontWeight::SEMIBOLD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child((i + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(text),
                    )
            }))
    }
}

/// Where a dialog of several steps stands: the steps in a row with a rule between them, the
/// ones behind checked off and the current one marked. Screen readers meet it as a list in which
/// the current step is the selected item.
#[derive(IntoElement)]
pub(crate) struct Progress {
    labels: Vec<SharedString>,
    current: usize,
}

/// A progress row over `labels`, with the step at `current`, counted from 0, as the one on show.
pub(crate) fn progress(
    labels: impl IntoIterator<Item = impl Into<SharedString>>,
    current: usize,
) -> Progress {
    Progress {
        labels: labels.into_iter().map(Into::into).collect(),
        current,
    }
}

impl RenderOnce for Progress {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let total = self.labels.len();
        let current = self.current;
        let mut row = div()
            .id("progress")
            .role(Role::List)
            .flex()
            .items_center()
            .w_full()
            .gap_2();
        for (i, label) in self.labels.into_iter().enumerate() {
            let done = i < current;
            let here = i == current;
            if i > 0 {
                row = row.child(
                    Separator::horizontal()
                        .flex_1()
                        .when(done || here, |rule| rule.bg(theme.primary)),
                );
            }
            let badge = div()
                .flex_none()
                .size(BADGE)
                .rounded_full()
                .border_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(theme.text(Text::Small))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|badge| match (done, here) {
                    (true, _) => badge.border_color(theme.primary).bg(theme.primary).child(
                        svg()
                            .path(icons::path("icons/check.svg"))
                            .size(CHECK)
                            .text_color(theme.primary_foreground),
                    ),
                    (false, true) => badge
                        .border_color(theme.primary)
                        .bg(theme.secondary)
                        .text_color(theme.foreground)
                        .child((i + 1).to_string()),
                    (false, false) => badge
                        .border_color(theme.border)
                        .text_color(theme.muted_foreground)
                        .child((i + 1).to_string()),
                });
            row = row.child(
                div()
                    .id(("progress-step", i))
                    .role(Role::ListItem)
                    .aria_label(label.clone())
                    .aria_position_in_set(i + 1)
                    .aria_size_of_set(total)
                    .aria_selected(here)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(badge)
                    .child(
                        div()
                            .text_size(theme.text(Text::Small))
                            .text_color(match here {
                                true => theme.foreground,
                                false => theme.muted_foreground,
                            })
                            .when(here, |label| label.font_weight(FontWeight::MEDIUM))
                            .child(label),
                    ),
            );
        }
        row
    }
}
