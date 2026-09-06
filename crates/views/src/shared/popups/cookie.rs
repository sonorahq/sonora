use std::rc::Rc;

use gpui::{App, Entity, IntoElement, RenderOnce, Window, px};
use gpui::{div, prelude::*};
use i18n::t;
use ui::{ActiveTheme as _, Button, Input, Modal, Text};

use crate::shared::steps::steps;

type Submit = Rc<dyn Fn(&(), &mut Window, &mut App)>;
type Cancel = Rc<dyn Fn(&(), &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct CookiePrompt {
    secret: Entity<Input>,
    slug: &'static str,
    submit: Option<Submit>,
    cancel: Option<Cancel>,
}

impl CookiePrompt {
    pub(crate) fn new(secret: Entity<Input>) -> Self {
        Self {
            secret,
            slug: "youtube",
            submit: None,
            cancel: None,
        }
    }

    pub(crate) fn provider(mut self, slug: &'static str) -> Self {
        self.slug = slug;
        self
    }

    pub(crate) fn on_submit(
        mut self,
        handler: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.submit = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_cancel(
        mut self,
        handler: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.cancel = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for CookiePrompt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            secret,
            slug,
            submit,
            cancel,
        } = self;
        let dismissed = cancel.clone();
        let theme = *cx.theme();
        let deezer = slug == "deezer";
        let open = match deezer {
            true => "https://www.deezer.com/desktop/login/electron/callback",
            false => "https://music.youtube.com",
        };
        let title = match deezer {
            true => t!("login-arl-title"),
            false => t!("login-cookie-title"),
        };
        let open_label = match deezer {
            true => t!("login-arl-open"),
            false => t!("login-cookie-open"),
        };

        Modal::new("cookie-prompt", title)
            .w(px(560.))
            .child(
                Button::new("open-sign-in")
                    .label(open_label)
                    .icon("icons/external-link.svg")
                    .outline()
                    .on_click(move |_, _, cx| cx.open_url(open)),
            )
            .child(steps(match deezer {
                true => [
                    t!("login-arl-step-1"),
                    t!("login-arl-step-2"),
                    t!("login-arl-step-3"),
                    t!("login-arl-step-4"),
                ],
                false => [
                    t!("login-cookie-step-1"),
                    t!("login-cookie-step-2"),
                    t!("login-cookie-step-3"),
                    t!("login-cookie-step-4"),
                ],
            }))
            .child(
                div()
                    .child(match deezer {
                        true => t!("login-arl-step-note"),
                        false => t!("login-cookie-step-note"),
                    })
                    .flex_1()
                    .min_w_0()
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground),
            )
            .child(secret)
            .action(
                Button::new("cancel-cookies")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(move |_, window, cx| {
                        if let Some(cancel) = &cancel {
                            cancel(&(), window, cx);
                        }
                    }),
            )
            .action(
                Button::new("submit-cookies")
                    .label(t!("login-cookie-submit"))
                    .primary()
                    .on_click(move |_, window, cx| {
                        if let Some(submit) = &submit {
                            submit(&(), window, cx);
                        }
                    }),
            )
            .on_dismiss(move |_, window, cx| {
                if let Some(dismissed) = &dismissed {
                    dismissed(&(), window, cx);
                }
            })
    }
}
