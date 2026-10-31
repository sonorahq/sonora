use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, FontWeight, SharedString, Window, div, px};
use i18n::t;
use music::AccountChoice;
use ui::{ActiveTheme as _, Button, Initials, Modal, Text};

const AVATAR: gpui::Pixels = px(32.);

type Pick = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;
type Cancel = Rc<dyn Fn(&(), &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct AccountPicker {
    accounts: Vec<AccountChoice>,
    pick: Option<Pick>,
    cancel: Option<Cancel>,
}

impl AccountPicker {
    pub(crate) fn new(accounts: Vec<AccountChoice>) -> Self {
        Self {
            accounts,
            pick: None,
            cancel: None,
        }
    }

    pub(crate) fn on_pick(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.pick = Some(Rc::new(handler));
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

impl RenderOnce for AccountPicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            accounts,
            pick,
            cancel,
        } = self;
        let theme = *cx.theme();
        let dismissed = cancel.clone();

        Modal::new("account-picker", t!("login-account-title"))
            .w(px(440.))
            .detail(t!("login-account-detail"))
            .child(div().flex().flex_col().gap_2().children({
                let total = accounts.len();
                accounts
                    .into_iter()
                    .enumerate()
                    .map(move |(place, account)| {
                        let pick = pick.clone();
                        let id = SharedString::from(account.id);
                        div()
                            .id(SharedString::from(format!("account-{id}")))
                            .role(Role::Button)
                            .aria_label(SharedString::from(account.name.clone()))
                            .when_some(account.detail.clone(), |this, detail| {
                                this.aria_description(SharedString::from(detail))
                            })
                            .aria_position_in_set(place + 1)
                            .aria_size_of_set(total)
                            .flex()
                            .items_center()
                            .gap_3()
                            .w_full()
                            .p_2()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .cursor_pointer()
                            .hover(|this| this.bg(theme.secondary_hover))
                            .child(Initials::new(account.name.clone(), AVATAR))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(SharedString::from(account.name)),
                                    )
                                    .when_some(account.detail, |this, detail| {
                                        this.child(
                                            div()
                                                .truncate()
                                                .text_size(theme.text(Text::Small))
                                                .text_color(theme.muted_foreground)
                                                .child(SharedString::from(detail)),
                                        )
                                    }),
                            )
                            .on_click(move |_, window, cx| {
                                if let Some(pick) = &pick {
                                    pick(&id, window, cx);
                                }
                            })
                    })
            }))
            .action(
                Button::new("cancel-account")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(move |_, window, cx| {
                        if let Some(cancel) = &cancel {
                            cancel(&(), window, cx);
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
