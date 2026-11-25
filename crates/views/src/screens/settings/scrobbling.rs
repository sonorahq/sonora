use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, Window, div, px};
use i18n::t;
use scrobble::{Link, Secret};
use state::ScrobbleState;
use ui::{ActiveTheme as _, Button, Form, Modal, Room, Switch};

use super::accounts::{Offer, Press, ProviderCard};
use super::{Group, SettingsView, Slot};

/// One field of the scrobbling link dialog: which hint it carries, what it starts with, and
/// whether it holds a secret and so is drawn as dots.
struct Field {
    hint: &'static str,
    value: String,
    masked: bool,
}

impl SettingsView {
    /// One card per scrobbling service, in the order the `scrobble` crate lists them, then the
    /// switch for the active provider's plays when a provider is signed in.
    pub(super) fn scrobbling_groups(&self, cx: &App) -> Vec<Group> {
        let provider = self.session.read(cx).provider_slug().is_some();
        vec![
            Group::untitled()
                .cards(Slot::Scrobbles)
                .rows(provider.then_some(Slot::ScrobbleProvider)),
        ]
    }

    /// What a search finds the cards by: the page's name over every service's name and where
    /// its link stands.
    pub(super) fn scrobbles_meta(&self, cx: &App) -> (SharedString, SharedString) {
        let words = (0..self.scrobbling.read(cx).rows().len())
            .filter_map(|index| self.scrobble_meta(index, cx))
            .map(|(title, detail)| format!("{title} {detail}"))
            .collect::<Vec<_>>()
            .join(" ");
        (t!("settings-page-scrobbling"), words.into())
    }

    /// The switch's label and explanation, named after the active provider.
    pub(super) fn scrobble_provider_meta(&self, cx: &App) -> Option<(SharedString, SharedString)> {
        let provider = self.session.read(cx).provider_name()?;
        Some((
            t!("settings-scrobble-provider", provider = provider),
            t!("settings-scrobble-provider-detail", provider = provider),
        ))
    }

    /// The switch that keeps the active provider's plays out of the linked services, for
    /// accounts that already scrobble to Last.fm on their own. Empty when no provider is active.
    pub(super) fn scrobble_provider_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let (Some(slug), Some(name)) = (session.provider_slug(), session.provider_name()) else {
            return div().into_any_element();
        };
        let on = self.settings.read(cx).scrobbles(slug);
        let label = t!("settings-scrobble-provider", provider = name);

        self.row(
            label.clone(),
            Switch::new("scrobble-provider", on)
                .name(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_scrobbles(slug, !on, cx));
                }))
                .into_any_element(),
        )
    }

    /// A service's name and where its link stands, which the row shows and a search reads.
    pub(super) fn scrobble_meta(
        &self,
        index: usize,
        cx: &App,
    ) -> Option<(SharedString, SharedString)> {
        let rows = self.scrobbling.read(cx).rows();
        let row = rows.get(index)?;
        let service = row.id();
        let detail = match row.state() {
            ScrobbleState::Off => t!("common-not-connected"),
            ScrobbleState::Linking => t!("settings-scrobble-waiting"),
            ScrobbleState::On(name) => match name.is_empty() {
                true => t!("settings-scrobble-on"),
                false => t!("settings-scrobble-as", name = name.as_ref()),
            },
            ScrobbleState::Refused => t!("settings-scrobble-key-refused"),
            ScrobbleState::Failed(key) => i18n::lookup(key, None),
        };
        Some((i18n::lookup(&format!("settings-{service}"), None), detail))
    }

    /// One card per service, two to a line, or one where the page is tight.
    pub(super) fn scrobble_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.scrobbling.read(cx).rows().len();
        let cards = (0..count)
            .map(|index| self.scrobble_card((index, count), cx))
            .collect::<Vec<_>>();
        let columns = match super::Layout::of(window, cx).room() {
            Room::Tight => 1,
            _ => 2,
        };

        div()
            .grid()
            .grid_cols(columns)
            .gap_2()
            .children(cards)
            .into_any_element()
    }

    /// A service's card at `place`, its index and the length of the list, drawn as a streaming
    /// provider's is. Not linked, the card is a dashed outline and its click starts the link.
    /// Linking, it offers to stop. Linked, it carries the switch that turns scrobbling to it on
    /// and the button that disconnects it, and it is edged in the accent while it scrobbles. A
    /// link that failed or was turned down says so in red.
    fn scrobble_card(&self, place: (usize, usize), cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (title, detail) = self.scrobble_meta(place.0, cx).unwrap_or_default();
        let row = &self.scrobbling.read(cx).rows()[place.0];
        let service = row.id();
        let link = row.link();
        let linked = row.linked();
        let linking = row.linking();
        let enabled = row.enabled();
        let state = row.state();
        let alert = matches!(state, ScrobbleState::Refused | ScrobbleState::Failed(_));
        let status = (!matches!(state, ScrobbleState::Off)).then_some(detail);

        let (offer, press): (Offer, Option<Press>) = match (linked, linking) {
            (true, _) => (
                Offer::Control(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("scrobble-unlink-{service}")))
                                .ghost()
                                .icon("icons/log-out.svg")
                                .tooltip("settings-scrobble-disconnect")
                                .tint(theme.muted_foreground)
                                .size(theme.metrics.control_small)
                                .px_0()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.scrobbling.update(cx, |scrobbling, cx| {
                                        scrobbling.disconnect(service, cx)
                                    });
                                })),
                        )
                        .child(
                            Switch::new(
                                SharedString::from(format!("scrobble-on-{service}")),
                                enabled,
                            )
                            .name(title.clone())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.scrobbling.update(cx, |scrobbling, cx| {
                                        scrobbling.set_enabled(service, !enabled, cx)
                                    });
                                },
                            )),
                        )
                        .into_any_element(),
                ),
                None,
            ),
            (false, true) => (
                Offer::Cancel(
                    Button::new(SharedString::from(format!("scrobble-cancel-{service}")))
                        .label(t!("common-cancel"))
                        .small()
                        .outline()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.scrobbling
                                .update(cx, |scrobbling, cx| scrobbling.cancel(cx));
                        }))
                        .into_any_element(),
                ),
                None,
            ),
            (false, false) => (
                Offer::SignIn(t!("settings-scrobble-connect")),
                Some(Box::new(cx.listener(move |this, _, _, cx| {
                    this.open_scrobble(service, link, cx)
                }))),
            ),
        };

        ProviderCard {
            id: SharedString::from(format!("scrobble-{service}")),
            logo: Some(crate::shared::scrobbler_logo(service)),
            brand: crate::shared::scrobbler_brand(service, cx),
            active: linked && enabled,
            stored: linked || linking,
            name: title,
            status,
            alert,
            offer,
            press,
            error: None,
            place,
        }
        .into_any_element()
    }

    /// Starts a link. A service with nothing to type goes straight to the browser; the rest put
    /// up the dialog with whatever was stored last time already in the fields.
    pub(super) fn open_scrobble(
        &mut self,
        service: &'static str,
        link: Link,
        cx: &mut Context<Self>,
    ) {
        let account = self.scrobbling.read(cx).account(service);
        let (first, second) = match link {
            Link::Browser => {
                return self.scrobbling.update(cx, |scrobbling, cx| {
                    scrobbling.connect(service, Secret::None, cx)
                });
            }
            Link::Keys => (
                Field {
                    hint: "common-api-key",
                    value: account.key,
                    masked: false,
                },
                Some(Field {
                    hint: "settings-scrobble-secret",
                    value: account.secret,
                    masked: true,
                }),
            ),
            Link::Token => (
                Field {
                    hint: "settings-scrobble-token",
                    value: account.session,
                    masked: true,
                },
                Some(Field {
                    hint: "settings-scrobble-server-optional",
                    value: account.server,
                    masked: false,
                }),
            ),
            Link::Server => (
                Field {
                    hint: "common-server-address",
                    value: account.server,
                    masked: false,
                },
                Some(Field {
                    hint: "common-api-key",
                    value: account.session,
                    masked: true,
                }),
            ),
            Link::Password => (
                Field {
                    hint: "settings-scrobble-handle",
                    value: account.name,
                    masked: false,
                },
                Some(Field {
                    hint: "settings-scrobble-app-password",
                    value: account.session,
                    masked: true,
                }),
            ),
        };

        self.scrobble_first.update(cx, |input, cx| {
            input.set_hint(first.hint, cx);
            input.set_masked(first.masked, cx);
            input.set_text(first.value, cx);
        });
        if let Some(second) = second {
            self.scrobble_second.update(cx, |input, cx| {
                input.set_hint(second.hint, cx);
                input.set_masked(second.masked, cx);
                input.set_text(second.value, cx);
            });
        }
        self.scrobble_prompt = Some(service);
        cx.notify();
    }

    pub(super) fn link_scrobble(
        &mut self,
        service: &'static str,
        link: Link,
        cx: &mut Context<Self>,
    ) {
        let first = self.scrobble_first.read(cx).text().to_string();
        let second = self.scrobble_second.read(cx).text().to_string();
        let secret = match link {
            Link::Browser => Secret::None,
            Link::Keys => Secret::Keys {
                key: first,
                secret: second,
            },
            Link::Token => Secret::Token {
                token: first,
                server: second,
            },
            Link::Server => Secret::Server {
                url: first,
                key: second,
            },
            Link::Password => Secret::Password {
                identifier: first,
                password: second,
            },
        };

        self.scrobble_prompt = None;
        self.scrobbling
            .update(cx, |scrobbling, cx| scrobbling.connect(service, secret, cx));
    }

    pub(super) fn scrobble_modal(
        &self,
        service: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let row = self
            .scrobbling
            .read(cx)
            .rows()
            .iter()
            .find(|row| row.id() == service);
        let link = row.map(|row| row.link()).unwrap_or(Link::Token);
        let signup = row.and_then(|row| row.signup());
        let title = i18n::lookup(&format!("settings-{service}"), None);
        let request = match link {
            Link::Token => "settings-scrobble-token-request",
            Link::Password => "settings-scrobble-password-request",
            _ => "settings-scrobble-request",
        };

        Modal::new(
            "settings-scrobble-prompt",
            t!("settings-scrobble-title", service = title.as_ref()),
        )
        .w(px(560.))
        .detail(i18n::lookup(&format!("settings-{service}-detail"), None))
        .child(
            Form::new("settings-scrobble-form")
                .field(ui::Field::new(&self.scrobble_first, cx))
                .field(ui::Field::new(&self.scrobble_second, cx))
                .on_submit(cx.listener(move |this, _, _, cx| this.link_scrobble(service, link, cx)))
                .on_dismiss(cx.listener(|this, _, _, cx| this.close_scrobble(cx))),
        )
        .when_some(signup, |this, url| {
            this.action(
                Button::new("settings-scrobble-request")
                    .ghost()
                    .label(i18n::lookup(request, None))
                    .on_click(move |_, _, cx| cx.open_url(url)),
            )
        })
        .action(
            Button::new("settings-scrobble-cancel")
                .ghost()
                .label(t!("common-cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.close_scrobble(cx))),
        )
        .action(
            Button::new("settings-scrobble-submit")
                .primary()
                .label(t!("settings-scrobble-connect"))
                .on_click(cx.listener(move |this, _, _, cx| this.link_scrobble(service, link, cx))),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| this.close_scrobble(cx)))
    }

    pub(super) fn close_scrobble(&mut self, cx: &mut Context<Self>) {
        self.scrobble_prompt = None;
        cx.notify();
    }
}
