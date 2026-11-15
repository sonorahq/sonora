//! The dialog that sets Soloist up: the download, the API key and the pairing, one step at a
//! time. `state::Soloist` says which step is on and what went wrong in the one before. This
//! draws it and holds the key field.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FocusHandle, Render, SharedString, Window, div};
use i18n::t;
use music::soloist::{DASHBOARD, DEVICE, TERMS};
use state::{Soloist, SoloistProblem, SoloistState, SoloistStep, Sonora};
use ui::{
    ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Field, Form, Input, Modal, Notice, Spinner,
    Submit, Text,
};

use crate::shared::steps::{progress, steps};

const ID: &str = "soloist-setup";
/// How wide the dialog stands, in covers, so the three steps fit on one line with room between
/// them.
const WIDTH: f32 = 3.4;

pub(crate) struct SoloistSetup {
    soloist: Entity<Soloist>,
    key: Entity<Input>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    /// The step drawn last, so focus moves and the body scrolls back up when it changes.
    step: Option<SoloistStep>,
}

impl SoloistSetup {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let soloist = Sonora::global(cx).soloist.clone();
        cx.observe(&soloist, |_, _, cx| cx.notify()).detach();
        let key = cx.new(|cx| {
            Input::new("soloist-key-hint", cx)
                .masked()
                .name("soloist-key-hint")
        });
        cx.observe(&key, |_, _, cx| cx.notify()).detach();
        Self {
            soloist,
            key,
            focus: cx.focus_handle(),
            restore: None,
            step: None,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.soloist.update(cx, |soloist, cx| soloist.close(cx));
    }

    /// The step the dialog is open on, or `None` while it is closed.
    fn open_on(&self, cx: &App) -> Option<SoloistStep> {
        match self.soloist.read(cx).state() {
            SoloistState::Setup(step) => Some(*step),
            _ => None,
        }
    }

    /// Enter and the emphasised button: the next step, where the current one has one.
    fn proceed(&mut self, cx: &mut Context<Self>) {
        match self.open_on(cx) {
            Some(SoloistStep::Consent) => {
                self.soloist.update(cx, |soloist, cx| soloist.download(cx));
            }
            Some(SoloistStep::Key) => {
                let key = self.key.read(cx).text().to_owned();
                self.soloist
                    .update(cx, |soloist, cx| soloist.submit(key, cx));
            }
            _ => {}
        }
    }

    /// Hands focus back and empties the key field, which must not outlive the dialog.
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.key.update(cx, |key, cx| key.set_text("", cx));
        self.step = None;
    }

    /// Follows the dialog to `step`: the key field takes focus on its step and the dialog
    /// itself on the others, and the body starts from the top again.
    fn enter(&mut self, step: SoloistStep, window: &mut Window, cx: &mut Context<Self>) {
        if self.step.is_none() {
            self.restore = window.focused(cx);
        }
        self.step = Some(step);
        Modal::rewind(ID, cx);
        match step {
            SoloistStep::Key => self.key.update(cx, |key, cx| key.focus(window, cx)),
            _ => window.focus(&self.focus, cx),
        }
    }

    fn body(&self, step: SoloistStep, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let problem = self.soloist.read(cx).problem().cloned();
        let mut body = vec![
            progress(
                [
                    t!("soloist-step-download"),
                    t!("soloist-step-key"),
                    t!("soloist-step-pair"),
                ],
                match step {
                    SoloistStep::Consent | SoloistStep::Fetching => 0,
                    SoloistStep::Key => 1,
                    SoloistStep::Pairing => 2,
                },
            )
            .into_any_element(),
        ];
        // a dialog that came up by itself says why, on the step it opens with
        if self.soloist.read(cx).offered() && step != SoloistStep::Pairing {
            body.push(paragraph("soloist-offered", t!("soloist-offered"), cx));
        }
        if let Some(problem) = problem {
            let (title, message) = said(&problem);
            body.push(
                Notice::new(title, message)
                    .failed()
                    .spoken("soloist-problem")
                    .into_any_element(),
            );
        }
        match step {
            SoloistStep::Consent => {
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(paragraph("soloist-terms", t!("soloist-terms"), cx))
                        .child(
                            Button::new("soloist-terms-open")
                                .label(t!("soloist-terms-open"))
                                .icon("icons/external-link.svg")
                                .small()
                                .ghost()
                                .on_click(|_, _, cx| cx.open_url(TERMS)),
                        )
                        .into_any_element(),
                );
            }
            SoloistStep::Fetching => {
                body.push(waiting("soloist-fetching", t!("soloist-downloading"), cx));
            }
            SoloistStep::Key => {
                body.push(
                    steps([
                        t!("soloist-key-step-open"),
                        t!("soloist-key-step-generate"),
                        t!("soloist-key-step-paste"),
                    ])
                    .into_any_element(),
                );
                body.push(
                    div()
                        .flex()
                        .child(
                            Button::new("soloist-dashboard")
                                .label(t!("soloist-key-open"))
                                .icon("icons/external-link.svg")
                                .outline()
                                .on_click(|_, _, cx| cx.open_url(DASHBOARD)),
                        )
                        .into_any_element(),
                );
                body.push(
                    Form::new("soloist-key-form")
                        .field(Field::new(&self.key, cx))
                        .into_any_element(),
                );
            }
            SoloistStep::Pairing => {
                body.push(
                    steps([
                        t!("soloist-pair-step-open"),
                        t!("soloist-pair-step-play"),
                        t!("soloist-pair-step-pick", device = DEVICE),
                    ])
                    .into_any_element(),
                );
                // said only once pairing has waited a while, when a firewall is the likely cause
                if let Some(port) = self.soloist.read(cx).blocked_port() {
                    // as text, so no locale groups the digits of a port number
                    let port = port.to_string();
                    body.push(paragraph(
                        "soloist-pair-firewall",
                        t!("soloist-pair-firewall", port = port.as_str()),
                        cx,
                    ));
                }
            }
        }
        body
    }
}

impl Render for SoloistSetup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(step) = self.open_on(cx) else {
            if self.step.is_some() {
                self.release(window, cx);
            }
            return div().into_any_element();
        };
        if self.step != Some(step) {
            self.enter(step, window, cx);
        }

        let typed = !self.key.read(cx).text().trim().is_empty();
        let body = self.body(step, cx);

        let modal = Modal::new(ID, t!("soloist-setup-title"))
            .w(cx.theme().metrics.cover * WIDTH)
            .close_button()
            .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx)));
        let modal = body
            .into_iter()
            .fold(modal, |modal, child| modal.child(child));
        let modal = match step {
            SoloistStep::Pairing => modal.action(
                Button::new("soloist-back")
                    .ghost()
                    .label(t!("soloist-back"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.soloist.update(cx, |soloist, cx| soloist.back(cx));
                    })),
            ),
            _ => modal
                .action(
                    Button::new("soloist-cancel")
                        .ghost()
                        .label(match step {
                            SoloistStep::Consent => t!("soloist-later"),
                            _ => t!("common-cancel"),
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                )
                .action(
                    Button::new("soloist-proceed")
                        .primary()
                        .label(match step {
                            SoloistStep::Key => t!("soloist-continue"),
                            _ => t!("soloist-download"),
                        })
                        .disabled(match step {
                            SoloistStep::Consent => false,
                            SoloistStep::Key => !typed,
                            _ => true,
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.proceed(cx))),
                ),
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, _, cx| {
                cx.stop_propagation();
                this.proceed(cx);
            }))
            .child(modal)
            .into_any_element()
    }
}

/// What a problem says: a short title and the sentence under it.
fn said(problem: &SoloistProblem) -> (SharedString, SharedString) {
    match problem {
        SoloistProblem::Download(error) => (
            t!("soloist-problem-download"),
            SharedString::from(error.clone()),
        ),
        SoloistProblem::Key => (t!("soloist-problem-key"), t!("soloist-problem-key-detail")),
        SoloistProblem::Account(name) => (
            t!("soloist-problem-account"),
            t!("soloist-problem-account-detail", name = name.as_str()),
        ),
        SoloistProblem::Waited => (
            t!("soloist-problem-waited"),
            t!("soloist-problem-waited-detail"),
        ),
        SoloistProblem::Pairing(error) => (
            t!("soloist-problem-pairing"),
            SharedString::from(error.clone()),
        ),
    }
}

/// A paragraph of quiet text that screen readers reach under `id`.
fn paragraph(id: &'static str, text: SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .role(Role::Paragraph)
        .aria_label(text.clone())
        .flex_1()
        .min_w_0()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .child(text)
        .into_any_element()
}

/// A spinner with what is being waited for beside it, announced as a status.
fn waiting(id: &'static str, text: SharedString, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .child(Spinner::new(SharedString::from(format!("{id}-spinner"))))
        .child(text)
        .into_any_element()
}
