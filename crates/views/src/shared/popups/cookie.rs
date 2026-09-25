use std::rc::Rc;

use gpui::{App, Entity, IntoElement, RenderOnce, Window, div, prelude::*, px};
use i18n::{FluentArgs, Value as _, lookup, t};
use ui::{ActiveTheme as _, Button, Input, Modal, Text};

use crate::shared::steps::steps;

type Action = Rc<dyn Fn(&(), &mut Window, &mut App)>;

/// The walkthrough for one provider's manual cookie sign-in: the page to send the user to
/// and the keys of every string the dialog shows. A provider whose sign-in is one cookie
/// found by name shares the parametrized flow; a provider needing a whole request header
/// names its own steps, with the title and the opener shared by every provider.
struct Guide {
    url: &'static str,
    title: &'static str,
    hint: &'static str,
    steps: [&'static str; 4],
    note: &'static str,
}

const YOUTUBE: Guide = Guide {
    url: "https://music.youtube.com",
    title: "login-cookie-header-title",
    hint: "login-cookie-hint",
    steps: [
        "login-cookie-step-1",
        "login-cookie-step-2",
        "login-cookie-step-3",
        "login-cookie-step-4",
    ],
    note: "login-cookie-step-note",
};

const APPLE: Guide = Guide {
    url: "https://music.apple.com",
    title: "login-cookie-header-title",
    hint: "login-cookie-hint",
    steps: [
        "login-cookie-named-step-1",
        "login-cookie-step-2",
        "login-cookie-apple-step-3",
        "login-cookie-step-4",
    ],
    note: "login-cookie-apple-note",
};

const DEEZER: Guide = Guide {
    url: "https://www.deezer.com",
    title: "login-cookie-named-title",
    hint: "login-cookie-named-hint",
    steps: [
        "login-cookie-named-step-1",
        "login-cookie-named-step-2",
        "login-cookie-named-step-3",
        "login-cookie-named-step-4",
    ],
    note: "login-cookie-named-note",
};

const NETEASE: Guide = Guide {
    url: "https://music.163.com",
    title: "login-cookie-named-title",
    hint: "login-cookie-named-hint",
    steps: [
        "login-cookie-named-step-1",
        "login-cookie-named-step-2",
        "login-cookie-named-step-3",
        "login-cookie-named-step-4",
    ],
    note: "login-cookie-named-note",
};

/// The guide for a provider slug. YouTube's is the fallback: pasting a whole request header
/// needs no cookie name, so its wording fits any header paste.
fn guide(slug: &str) -> &'static Guide {
    match slug {
        "apple" => &APPLE,
        "deezer" => &DEEZER,
        "netease" => &NETEASE,
        _ => &YOUTUBE,
    }
}

/// The site the devtools show and the cookie to copy, for the flows whose strings name them.
/// YouTube's header flow needs neither, so it carries no entry here.
fn named(slug: &str) -> Option<(&'static str, &'static str)> {
    match slug {
        "deezer" => Some(("www.deezer.com", "arl")),
        "apple" => Some(("music.apple.com", "media-user-token")),
        "netease" => Some(("music.163.com", "MUSIC_U")),
        _ => None,
    }
}

#[derive(IntoElement)]
pub(crate) struct CookiePrompt {
    slug: &'static str,
    provider: &'static str,
    secret: Entity<Input>,
    submit: Option<Action>,
    cancel: Option<Action>,
}

impl CookiePrompt {
    pub(crate) fn new(slug: &'static str, provider: &'static str, secret: Entity<Input>) -> Self {
        Self {
            slug,
            provider,
            secret,
            submit: None,
            cancel: None,
        }
    }

    /// The hint key the paste field should carry for a provider, set on the input when the
    /// manual sign-in starts so it follows the language like every other hint.
    pub(crate) fn hint(slug: &str) -> &'static str {
        guide(slug).hint
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
            slug,
            provider,
            secret,
            submit,
            cancel,
        } = self;
        let dismissed = cancel.clone();
        let theme = *cx.theme();
        let guide = guide(slug);

        let mut args = FluentArgs::new();
        args.set("provider", provider.value());
        if let Some((site, cookie)) = named(slug) {
            args.set("site", site.value());
            args.set("cookie", cookie.value());
        }

        Modal::new("cookie-prompt", lookup(guide.title, Some(&args)))
            .w(px(560.))
            .child(
                Button::new("open-cookie-provider")
                    .label(lookup("login-cookie-open", Some(&args)))
                    .icon("icons/external-link.svg")
                    .outline()
                    .on_click(move |_, _, cx| cx.open_url(guide.url)),
            )
            .child(steps(
                guide.steps.iter().map(|key| lookup(key, Some(&args))),
            ))
            .child(
                div()
                    .child(lookup(guide.note, Some(&args)))
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
