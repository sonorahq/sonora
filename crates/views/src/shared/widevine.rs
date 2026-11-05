//! The two questions before Google's Widevine module is installed: whether to download it,
//! then whether its terms are accepted. Both are `state::Drm`'s to ask; this only draws them.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{Context, Entity, FocusHandle, Render, Window, div};
use i18n::t;
use state::{CdmState, Drm, Sonora};
use ui::{ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Modal, Submit, Text};

pub(crate) struct WidevinePrompt {
    drm: Entity<Drm>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl WidevinePrompt {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let drm = Sonora::global(cx).drm.clone();
        cx.observe(&drm, |_, _, cx| cx.notify()).detach();
        Self {
            drm,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    fn decline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.drm.update(cx, |drm, cx| drm.dismiss(cx));
    }

    fn proceed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.drm.read(cx).state() {
            CdmState::Wanted => self.drm.update(cx, |drm, cx| drm.download(cx)),
            CdmState::Offered(_) => {
                self.release(window, cx);
                self.drm.update(cx, |drm, cx| drm.accept(cx));
            }
            _ => {}
        }
    }

    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.shown = false;
    }
}

impl Render for WidevinePrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.drm.read(cx).state().clone();
        let open = matches!(
            state,
            CdmState::Wanted | CdmState::Offering | CdmState::Offered(_) | CdmState::Installing
        );
        if !open {
            if self.shown {
                self.release(window, cx);
            }
            return div().into_any_element();
        }
        if !self.shown {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.shown = true;
        }

        let theme = *cx.theme();
        let small = theme.text(Text::Small);
        let wanted = match self.drm.read(cx).replacing() {
            true => t!("widevine-prompt-replace"),
            false => t!("widevine-prompt-wanted"),
        };
        let (detail, note, license, action) = match &state {
            CdmState::Wanted => (wanted, None, None, true),
            CdmState::Offering => (wanted, Some(t!("widevine-prompt-downloading")), None, false),
            CdmState::Offered(terms) => (
                t!("widevine-prompt-terms", version = terms.version.as_str()),
                None,
                Some(terms.license.clone()),
                true,
            ),
            _ => (
                t!("widevine-prompt-terms", version = ""),
                Some(t!("widevine-prompt-installing")),
                None,
                false,
            ),
        };
        let accepting = matches!(state, CdmState::Offered(_));
        let proceed = match accepting {
            true => t!("widevine-prompt-accept"),
            false => t!("widevine-prompt-download"),
        };
        let decline = match accepting {
            true => t!("widevine-prompt-decline"),
            false => t!("widevine-prompt-later"),
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.decline(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.proceed(window, cx);
            }))
            .child(
                Modal::new("widevine-prompt", t!("widevine-prompt-title"))
                    .detail(detail)
                    .when_some(license, |modal, license| {
                        modal.child(
                            div()
                                .id("widevine-license")
                                .role(Role::Paragraph)
                                .aria_label(license.clone())
                                .p(theme.metrics.pad)
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.secondary)
                                .text_size(small)
                                .text_color(theme.muted_foreground)
                                .child(license),
                        )
                    })
                    .when_some(note, |modal, note| {
                        modal.child(
                            div()
                                .id("widevine-note")
                                .role(Role::Status)
                                .aria_label(note.clone())
                                .text_size(small)
                                .text_color(theme.muted_foreground)
                                .child(note),
                        )
                    })
                    .action(
                        Button::new("decline-widevine")
                            .ghost()
                            .label(decline)
                            .on_click(cx.listener(|this, _, window, cx| this.decline(window, cx))),
                    )
                    .action(
                        Button::new("proceed-widevine")
                            .primary()
                            .label(proceed)
                            .disabled(!action)
                            .on_click(cx.listener(|this, _, window, cx| this.proceed(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.decline(window, cx))),
            )
            .into_any_element()
    }
}
