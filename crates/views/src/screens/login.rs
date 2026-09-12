use crate::shared::popups::{AccountPicker, CookiePrompt};
use gpui::prelude::*;
use gpui::{
    ClipboardItem, Context, Entity, FontWeight, IntoElement, Pixels, Render, SharedString, Window,
    div, px, svg,
};
use i18n::{lookup, t};
use music::{AccountChoice, SignIn, SignInPrompt};
use state::{Session, SessionState, Sonora, Usage};
use ui::ActiveTheme as _;
use ui::{Button, Checkbox, Input, Modal, TabBar, Text};

const COLUMN: Pixels = px(280.);
const LOGO: Pixels = px(48.);

struct Column {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    web_sign_in_label: Option<&'static str>,
    disabled: bool,
    cancel: bool,
}

#[derive(Clone, Copy)]
struct BrowserSignIn {
    enabled: bool,
    label: Option<&'static str>,
}

enum LoginAction {
    SignIn(SignIn),
    Credentials,
    Cookies,
}

struct LoginOption {
    id: SharedString,
    label: SharedString,
    action: LoginAction,
    primary: bool,
}

pub struct LoginView {
    session: Entity<Session>,
    usage: Entity<Usage>,
    secret: Entity<Input>,
    server: Entity<Input>,
    username: Entity<Input>,
    password: Entity<Input>,
    credentials_for: Option<&'static str>,
    manual_secret: bool,
    tab: usize,
}

impl LoginView {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let usage = Sonora::global(cx).usage.clone();
        cx.observe(&usage, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            usage,
            secret: cx.new(|cx| Input::new("login-manual-hint", cx)),
            server: cx.new(|cx| Input::new("login-server-hint", cx)),
            username: cx.new(|cx| Input::new("login-username-hint", cx)),
            password: cx.new(|cx| Input::new("login-password-hint", cx).masked()),
            credentials_for: None,
            manual_secret: false,
            tab: 0,
        }
    }

    fn acted(&self, cx: &mut Context<Self>) {
        self.usage.update(cx, |usage, cx| usage.report(cx));
    }

    fn consent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.usage.read(cx).consented();
        Checkbox::new("usage-consent", checked)
            .label(t!("login-usage-consent"))
            .max_w(COLUMN)
            .text_color(cx.theme().muted_foreground)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.usage
                    .update(cx, |usage, cx| usage.consent(!checked, cx));
            }))
    }

    fn abandon(&mut self, cx: &mut Context<Self>) {
        self.acted(cx);
        self.clear_secret(cx);
        self.clear_credentials(cx);
        self.session
            .update(cx, |session, cx| session.cancel_sign_in(cx));
    }

    fn open_credentials(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        self.acted(cx);
        self.credentials_for = Some(slug);
        cx.notify();
    }

    fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credentials_for = None;
        self.server.update(cx, |input, cx| input.set_text("", cx));
        self.username.update(cx, |input, cx| input.set_text("", cx));
        self.password.update(cx, |input, cx| input.set_text("", cx));
    }

    fn clear_secret(&mut self, cx: &mut Context<Self>) {
        self.manual_secret = false;
        self.secret.update(cx, |input, cx| input.set_text("", cx));
    }

    fn abandon_credentials(&mut self, cx: &mut Context<Self>) {
        self.acted(cx);
        self.clear_credentials(cx);
        cx.notify();
    }

    fn submit_credentials(&mut self, cx: &mut Context<Self>) {
        let Some(slug) = self.credentials_for else {
            return;
        };
        let server = self.server.read(cx).text().to_string();
        let username = self.username.read(cx).text().to_string();
        let password = self.password.read(cx).text().to_string();
        if server.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
            return;
        }
        self.clear_credentials(cx);
        self.start(
            slug,
            SignIn::Credentials {
                server,
                username,
                password,
            },
            cx,
        );
    }

    fn start(&mut self, slug: &'static str, method: SignIn, cx: &mut Context<Self>) {
        self.acted(cx);
        self.manual_secret = false;
        self.session
            .update(cx, |session, cx| session.sign_in(slug, method, cx));
    }

    fn start_manual(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        self.acted(cx);
        self.manual_secret = true;
        self.session
            .update(cx, |session, cx| session.sign_in_with_cookies(slug, cx));
    }

    fn submit_secret(&mut self, cx: &mut Context<Self>) {
        let text = self.secret.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.acted(cx);
        self.clear_secret(cx);
        self.session
            .update(cx, |session, cx| session.submit_input(text, cx));
    }

    fn option_buttons(
        &self,
        slug: &'static str,
        provider: &str,
        method: &SignIn,
        browser: BrowserSignIn,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Vec<Button> {
        let options = match method {
            SignIn::Secret => {
                let mut options = Vec::new();
                if browser.enabled {
                    options.push(LoginOption {
                        id: format!("sign-in-{slug}-cookies").into(),
                        label: browser.label.map_or_else(
                            || t!("login-sign-in", provider = provider),
                            |label| lookup(label, None),
                        ),
                        action: LoginAction::SignIn(SignIn::Secret),
                        primary: true,
                    });
                }
                options.push(LoginOption {
                    id: format!("sign-in-{slug}-cookies-manual").into(),
                    label: t!("login-manual-sign-in"),
                    action: LoginAction::Cookies,
                    primary: false,
                });
                options
            }
            SignIn::Default | SignIn::Anonymous | SignIn::Path(_) | SignIn::Credentials { .. } => {
                let (suffix, label, action, primary) = match method {
                    SignIn::Default => (
                        "",
                        t!("login-sign-in", provider = provider),
                        LoginAction::SignIn(method.clone()),
                        true,
                    ),
                    SignIn::Anonymous => (
                        "-guest",
                        t!("login-use", provider = provider),
                        LoginAction::SignIn(method.clone()),
                        true,
                    ),
                    SignIn::Path(_) => (
                        "-path",
                        t!("login-sign-in", provider = provider),
                        LoginAction::SignIn(method.clone()),
                        false,
                    ),
                    SignIn::Credentials { .. } => (
                        "-server",
                        t!("login-sign-in", provider = provider),
                        LoginAction::Credentials,
                        true,
                    ),
                    SignIn::Secret => unreachable!(),
                };
                vec![LoginOption {
                    id: format!("sign-in-{slug}{suffix}").into(),
                    label,
                    action,
                    primary,
                }]
            }
        };

        options
            .into_iter()
            .map(|option| self.login_button(slug, option, disabled, cx))
            .collect()
    }

    fn login_button(
        &self,
        slug: &'static str,
        option: LoginOption,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let button = Button::new(option.id)
            .label(option.label)
            .w_full()
            .disabled(disabled)
            .on_click(cx.listener(move |this, _, _, cx| match &option.action {
                LoginAction::SignIn(method) => this.start(slug, method.clone(), cx),
                LoginAction::Credentials => this.open_credentials(slug, cx),
                LoginAction::Cookies => this.start_manual(slug, cx),
            }));
        match option.primary {
            true => button.primary(),
            false => button.outline(),
        }
    }

    fn column(&self, column: Column, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let Column {
            slug,
            name,
            options,
            web_sign_in,
            web_sign_in_label,
            disabled,
            cancel,
        } = column;
        let options: Vec<&SignIn> = options
            .iter()
            .filter(|option| !matches!(option, SignIn::Anonymous))
            .collect();

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w(COLUMN)
            .child(
                svg()
                    .path(icons::path(crate::shared::provider_logo(slug)))
                    .size(LOGO)
                    .flex_none()
                    .text_color(theme.foreground),
            )
            .child(
                div()
                    .text_size(theme.text(Text::Large))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(name.to_string())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .children(options.into_iter().flat_map(|method| {
                        self.option_buttons(
                            slug,
                            name,
                            method,
                            BrowserSignIn {
                                enabled: web_sign_in,
                                label: web_sign_in_label,
                            },
                            disabled,
                            cx,
                        )
                    }))
                    .when(cancel, |this| {
                        this.child(
                            Button::new("cancel-sign-in")
                                .label(t!("common-cancel"))
                                .outline()
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| this.abandon(cx))),
                        )
                    }),
            )
    }

    fn guest_mode(&self, slug: &'static str, pending: bool, cx: &mut Context<Self>) -> Button {
        Button::new("guest-mode")
            .label(t!("login-guest-continue"))
            .outline()
            .w_full()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.start(slug, SignIn::Anonymous, cx);
            }))
    }

    fn code_prompt(&self, code: String, url: String, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground)
                    .child(t!("login-device-code", url = &url)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(theme.text(Text::Title))
                            .font_weight(FontWeight::BOLD)
                            .child(SharedString::from(code.clone())),
                    )
                    .child(
                        Button::new("copy-code")
                            .icon("icons/copy.svg")
                            .ghost()
                            .small()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            }),
                    ),
            )
    }

    fn url_prompt(&self, url: String) -> impl IntoElement {
        Button::new("copy-login-url")
            .icon("icons/copy.svg")
            .label(t!("menu-copy-link"))
            .outline()
            .small()
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
            })
    }

    fn account_modal(
        &self,
        accounts: Vec<AccountChoice>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        AccountPicker::new(accounts)
            .on_pick(cx.listener(|this, id: &SharedString, _, cx| {
                this.acted(cx);
                let id = id.to_string();
                this.session
                    .update(cx, |session, cx| session.submit_input(id, cx));
            }))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    fn credentials_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("server-prompt", t!("login-server-title"))
            .w(px(560.))
            .detail(t!("login-server-detail"))
            .child(self.server.clone())
            .child(self.username.clone())
            .child(self.password.clone())
            .action(
                Button::new("cancel-server")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon_credentials(cx))),
            )
            .action(
                Button::new("submit-server")
                    .label(t!("login-server-submit"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.submit_credentials(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon_credentials(cx)))
    }

    fn secret_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        CookiePrompt::new(self.secret.clone())
            .on_submit(cx.listener(|this, _, _, cx| this.submit_secret(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.session.read(cx).state().clone();
        let pending = self.session.read(cx).is_pending();
        let providers: Vec<state::ProviderInfo> = self.session.read(cx).providers().collect();
        let guest = providers
            .iter()
            .filter(|info| {
                info.options
                    .iter()
                    .any(|option| matches!(option, SignIn::Anonymous))
            })
            .map(|info| info.slug)
            .next();
        let manual_secret = self.manual_secret
            && matches!(
                &state,
                SessionState::Authorizing(Some(SignInPrompt::Secret))
            );
        let waiting = match &state {
            SessionState::Authorizing(prompt) => {
                !manual_secret && !matches!(prompt, Some(SignInPrompt::Accounts(_)))
            }
            _ => false,
        };
        let tabs = providers
            .iter()
            .enumerate()
            .map(|(index, info)| {
                Button::new(SharedString::from(format!("login-tab-{}", info.slug)))
                    .label(SharedString::from(info.name))
                    .small()
                    .ghost()
                    .selected(index == self.tab)
                    .flex_1()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.acted(cx);
                        this.tab = index;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let column = providers.into_iter().nth(self.tab).map(|info| Column {
            slug: info.slug,
            name: info.name,
            options: info.options,
            web_sign_in: info.web_sign_in,
            web_sign_in_label: info.web_sign_in_label,
            disabled: pending,
            cancel: waiting && info.pending,
        });

        let failure = match &state {
            SessionState::Failed(failure) => Some(failure.clone()),
            _ => None,
        };

        let status = match &state {
            SessionState::SignedOut => t!("login-signed-out"),
            SessionState::Restoring => t!("login-restoring"),
            SessionState::Authorizing(Some(SignInPrompt::Accounts(_))) => t!("login-signed-out"),
            SessionState::Authorizing(_) => t!("login-authorizing"),
            SessionState::SignedIn(profile) => t!("login-signed-in", name = &profile.display_name),
            SessionState::Failed(_) => t!("login-signed-out"),
        };

        let prompt = match &state {
            SessionState::Authorizing(prompt) => prompt.clone(),
            _ => None,
        };
        let accounts = match &prompt {
            Some(SignInPrompt::Accounts(accounts)) => Some(accounts.clone()),
            _ => None,
        };
        let code = match prompt {
            Some(SignInPrompt::Code { code, url }) => Some((code, url)),
            _ => None,
        };
        let url = match &state {
            SessionState::Authorizing(Some(SignInPrompt::Url(url))) => Some(url.clone()),
            _ => None,
        };

        let theme = *cx.theme();
        let asking = self.usage.read(cx).asking();
        let orphan = asking && guest.is_none();

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx));
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .size_full()
            .when_some(radius, |this, radius| this.rounded_b(radius))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .child("Sonora")
                            .text_size(theme.text(Text::Display))
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        div()
                            .max_w(px(560.))
                            .text_center()
                            .text_size(theme.text(Text::Body))
                            .text_color(theme.muted_foreground)
                            .child(status),
                    ),
            )
            .when_some(failure, |this, failure| {
                this.child(crate::shared::trouble::trouble(failure, true))
            })
            .when_some(code, |this, (code, url)| {
                this.child(self.code_prompt(code, url, cx).into_any_element())
            })
            .when_some(url, |this, url| this.child(self.url_prompt(url)))
            .child(TabBar::new().w(COLUMN).items(tabs))
            .when_some(column, |this, column| this.child(self.column(column, cx)))
            .when_some(guest, |this, slug| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .w(COLUMN)
                        .child(self.guest_mode(slug, pending, cx))
                        .child(
                            div()
                                .text_center()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(t!("login-guest-detail")),
                        )
                        .when(asking, |this| this.child(self.consent(cx))),
                )
            })
            .when(orphan, |this| this.child(self.consent(cx)))
            .when(manual_secret, |this| {
                this.child(self.secret_prompt(cx).into_any_element())
            })
            .when(self.credentials_for.is_some(), |this| {
                this.child(self.credentials_prompt(cx).into_any_element())
            })
            .when_some(accounts, |this, accounts| {
                this.child(self.account_modal(accounts, cx).into_any_element())
            })
    }
}
