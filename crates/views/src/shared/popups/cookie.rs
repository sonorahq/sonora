//! The manual cookie sign-in: the walkthrough for copying a provider's cookie out of the
//! browser's developer tools and the field it is pasted into.

use std::rc::Rc;

use i18n::{FluentArgs, Value as _, lookup};
use rune::{El, TextStyle, Tx, text};
use ui::{Button, Modal, Text, Theme, Weight};

use crate::Ui;
use crate::shared::steps::steps;

/// How wide the dialog stands.
const WIDTH: f32 = 560.;

/// What a button of the dialog does, inside `act`.
type Action = Rc<dyn Fn(&mut Ui)>;

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

/// The guide for a provider slug. YouTube's is the fallback: pasting a whole request header
/// needs no cookie name, so its wording fits any header paste.
fn guide(slug: &str) -> &'static Guide {
    match slug {
        "apple" => &APPLE,
        "deezer" => &DEEZER,
        _ => &YOUTUBE,
    }
}

/// The site the devtools show and the cookie to copy, for the flows whose strings name them.
/// YouTube's header flow needs neither, so it carries no entry here.
fn named(slug: &str) -> Option<(&'static str, &'static str)> {
    match slug {
        "deezer" => Some(("www.deezer.com", "arl")),
        "apple" => Some(("music.apple.com", "media-user-token")),
        _ => None,
    }
}

/// The Fluent key of the hint the paste field carries for a provider, resolved by whoever
/// builds the field so it follows the language like every other hint.
pub(crate) fn cookie_hint(slug: &str) -> &'static str {
    guide(slug).hint
}

/// The cookie dialog for the provider `slug` called `provider`, around the paste field `secret`
/// its owner builds over its own `TextEdit`.
pub(crate) struct CookiePrompt {
    slug: &'static str,
    provider: &'static str,
    secret: El,
    submit: Option<Action>,
    cancel: Option<Action>,
}

impl CookiePrompt {
    pub(crate) fn new(slug: &'static str, provider: &'static str, secret: El) -> Self {
        Self {
            slug,
            provider,
            secret,
            submit: None,
            cancel: None,
        }
    }

    /// Runs when Sign in is pressed, inside `act`.
    pub(crate) fn on_submit(mut self, f: impl Fn(&mut Ui) + 'static) -> Self {
        self.submit = Some(Rc::new(f));
        self
    }

    /// Runs when Cancel is pressed or the dialog is dismissed, inside `act`.
    pub(crate) fn on_cancel(mut self, f: impl Fn(&mut Ui) + 'static) -> Self {
        self.cancel = Some(Rc::new(f));
        self
    }

    /// Puts the dialog up in `layer`.
    pub(crate) fn build(self, theme: &Theme, layer: &mut Vec<El>) {
        let guide = guide(self.slug);
        let mut args = FluentArgs::new();
        args.set("provider", self.provider.value());
        if let Some((site, cookie)) = named(self.slug) {
            args.set("site", site.value());
            args.set("cookie", cookie.value());
        }
        let note = lookup(guide.note, Some(&args)).into_owned();
        let cancel = self.cancel.clone();
        let dismissed = self.cancel;
        let submit = self.submit;

        Modal::<Ui>::new("cookie-prompt", lookup(guide.title, Some(&args)))
            .width(WIDTH)
            .child(
                Button::new("open-cookie-provider")
                    .label(lookup("login-cookie-open", Some(&args)))
                    .icon("icons/external-link.svg")
                    .outline()
                    .on_press_tx(move |_: &mut Ui, tx: &mut Tx| {
                        if let Err(error) = tx.open_url(guide.url) {
                            log::warn!("login: cannot open {}: {error}", guide.url);
                        }
                    })
                    .build(theme)
                    .self_start(),
            )
            .child(steps(
                guide
                    .steps
                    .iter()
                    .map(|key| lookup(key, Some(&args)).into_owned()),
                theme,
            ))
            .child(
                text(note.clone())
                    .key("cookie-note")
                    .role("paragraph")
                    .label(note)
                    .wrap()
                    .min_w(0.)
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.muted_foreground.color()),
            )
            .child(self.secret)
            .action(
                Button::new("cancel-cookies")
                    .ghost()
                    .label(i18n::t!("common-cancel"))
                    .on_press(crate::act(move |ui: &mut Ui| {
                        if let Some(cancel) = &cancel {
                            cancel(ui);
                        }
                    }))
                    .build(theme),
            )
            .action(
                Button::new("submit-cookies")
                    .label(i18n::t!("login-cookie-submit"))
                    .primary()
                    .on_press(crate::act(move |ui: &mut Ui| {
                        if let Some(submit) = &submit {
                            submit(ui);
                        }
                    }))
                    .build(theme),
            )
            .on_dismiss(crate::act(move |ui: &mut Ui| {
                if let Some(dismissed) = &dismissed {
                    dismissed(ui);
                }
            }))
            .build(theme, layer);
    }
}
