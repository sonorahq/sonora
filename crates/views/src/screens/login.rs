//! The sign-in page shown while no account is open: the app's name and where the session
//! stands, a tab per music service with the ways to sign in to it, guest mode, and the dialogs a
//! sign-in raises on the way (the server form, the cookie paste and the account choice).

use i18n::t;
use music::{SignIn, SignInProblem, SignInPrompt};
use rune::{Align, Cx, El, ScrollSpec, TextEdit, Transition, Tx, col, icon};
use state::{Failure, SessionState};
use ui::{Button, Checkbox, Modal, TabBar, Text, Theme, Weight, font, spoken};

use crate::shared::device_code::device_code;
use crate::shared::page::rems;
use crate::shared::popups::{CookiePrompt, account_picker, cookie_hint};
use crate::shared::trouble::trouble;
use crate::{Ui, act, act_tx};

/// How wide a provider's column and the guest button stand.
const COLUMN: f32 = 280.;
/// How large a provider's mark draws.
const LOGO: f32 = 48.;
/// How wide the status line may run before it wraps.
const STATUS: f32 = 560.;
/// How wide the server dialog stands.
const SERVER_WIDTH: f32 = 560.;
/// How far a provider's column rises as its tab is chosen, and how long each of its buttons
/// waits on the one above.
const RISE: f32 = 6.;
const STEP: f32 = 0.04;

/// What the sign-in page keeps between frames: the chosen tab, the dialogs a sign-in put up
/// and the fields they hold.
#[derive(Default)]
pub struct State {
    /// The provider tab chosen, or none to follow the provider whose sign-in failed.
    tab: Option<usize>,
    /// The server sign-in dialog, by provider slug, and its fields.
    credentials_for: Option<&'static str>,
    pub server: TextEdit,
    pub username: TextEdit,
    pub password: TextEdit,
    /// The manual cookie paste, by provider slug and name, and its field.
    manual_secret: Option<(&'static str, &'static str)>,
    pub secret: TextEdit,
}

/// How a column offers one way to sign in.
enum Action {
    SignIn(SignIn),
    Credentials,
    Cookies,
}

struct Choice {
    key: String,
    label: String,
    action: Action,
    primary: bool,
}

/// Keeps `ui.chrome.signing_in` in step with the session, as the window decides whether the
/// sign-in page stays up: set while signed out, kept through an authorization the page started,
/// and dropped once a session is open, so a sign-in started from the settings stays there.
pub(crate) fn follow(ui: &mut Ui) {
    match ui.store.session.state() {
        SessionState::SignedOut | SessionState::Failed(_) => ui.chrome.signing_in = true,
        SessionState::Restoring | SessionState::SignedIn(_) | SessionState::Offline(_) => {
            ui.chrome.signing_in = false
        }
        SessionState::Authorizing(_) => {}
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let session = &ui.store.session;
    let page = &ui.screens.login;
    let state = session.state().clone();
    let pending = session.is_pending();
    let providers: Vec<state::ProviderInfo> = session.providers().collect();
    let guest = providers
        .iter()
        .find(|info| {
            info.options
                .iter()
                .any(|option| matches!(option, SignIn::Anonymous))
        })
        .map(|info| info.slug);
    let manual_secret = page.manual_secret.filter(|_| {
        matches!(
            &state,
            SessionState::Authorizing(Some(SignInPrompt::Secret))
        )
    });
    let waiting = match &state {
        SessionState::Authorizing(prompt) => {
            manual_secret.is_none() && !matches!(prompt, Some(SignInPrompt::Accounts(_)))
        }
        _ => false,
    };
    let count = providers.len();
    let tab = page
        .tab
        .or_else(|| providers.iter().position(|info| info.error.is_some()))
        .unwrap_or(0)
        .min(count.saturating_sub(1));
    let tabs = providers
        .iter()
        .enumerate()
        .map(|(index, info)| {
            Button::new(format!("login-tab-{}", info.slug))
                .label(info.name)
                .small()
                .ghost()
                .selected(index == tab)
                .on_press(act(move |ui: &mut Ui| {
                    acted(ui);
                    ui.screens.login.tab = Some(index);
                }))
        })
        .collect::<Vec<_>>();
    let selected = providers.into_iter().nth(tab);
    let mut failure = selected.as_ref().and_then(|info| info.error.clone());
    // A missing subscription belongs to the service's own column, with the way to check it.
    let subscription = failure
        .as_ref()
        .is_some_and(|failure| failure.problem == Some(SignInProblem::Subscription));
    let inline_failure = match subscription {
        true => failure.take(),
        false => None,
    };

    let status = match &state {
        SessionState::SignedOut => t!("login-signed-out"),
        SessionState::Restoring => t!("login-restoring"),
        SessionState::Authorizing(Some(SignInPrompt::Accounts(_))) => t!("login-signed-out"),
        SessionState::Authorizing(_) => t!("login-authorizing"),
        SessionState::SignedIn(profile) => t!("login-signed-in", name = &profile.display_name),
        SessionState::Offline(_) | SessionState::Failed(_) => t!("login-signed-out"),
    }
    .into_owned();
    let prompt = match &state {
        SessionState::Authorizing(prompt) => prompt.clone(),
        _ => None,
    };
    let asking = ui.store.usage.asking();
    let orphan = asking && guest.is_none();

    let mut content = col()
        .key("login-content")
        .w_full()
        .min_h(0.)
        .items_center()
        .justify_center()
        .gap(rems(theme, 1.5))
        .padding(theme.metrics.pad)
        .child(
            col()
                .key("login-head")
                .items_center()
                .gap(rems(theme, 0.5))
                .child(
                    spoken("login-title", "Sonora", theme)
                        .role("heading")
                        .level(1)
                        .font(font(theme, Text::Display, Weight::BOLD)),
                )
                .child(
                    spoken("login-status", status, theme)
                        .role("status")
                        .max_w(STATUS)
                        .wrap()
                        .align(Align::Center)
                        .color(theme.muted_foreground.color())
                        .transition(Transition::opacity()),
                ),
        );
    if let Some(failure) = failure {
        content = content.child(
            col()
                .key("login-failure")
                .role("alert")
                .label(t!("login-failed-title"))
                .max_w(STATUS)
                .child(trouble(failure, true, theme))
                .transition(rise()),
        );
    }
    match prompt {
        Some(SignInPrompt::Code { code, url }) => {
            content = content.child(code_prompt(&code, &url, theme));
        }
        Some(SignInPrompt::Url(url)) => {
            content = content.child(url_prompt(url, theme));
        }
        _ => {}
    }
    content = content.child(
        TabBar::new("login-providers")
            .name(t!("login-providers"))
            .items(tabs)
            .build(theme),
    );
    if let Some(info) = selected {
        let cancel = waiting && info.pending;
        content = content.child(column(
            ui,
            Column {
                slug: info.slug,
                name: info.name,
                options: info.options,
                web_sign_in: info.web_sign_in,
                disabled: pending,
                cancel,
                failure: inline_failure,
            },
        ));
    }
    if let Some(slug) = guest {
        let mut guest = col()
            .key("login-guest")
            .items_center()
            .gap(rems(theme, 0.5))
            .w(COLUMN)
            .child(
                Button::new("guest-mode")
                    .label(t!("login-guest-title"))
                    .outline()
                    .disabled(pending)
                    .on_press(act(move |ui: &mut Ui| start(ui, slug, SignIn::Anonymous)))
                    .build(theme)
                    .w_full()
                    .justify_center(),
            );
        if asking {
            guest = guest.child(consent(ui));
        }
        content = content.child(guest);
    }
    if orphan {
        content = content.child(consent(ui));
    }

    dialogs(ui, manual_secret, layer);
    let tall = cx
        .layout_with("login-scroll", |rect| rect[3].round())
        .unwrap_or(0.);

    col().key("login").grow(1.).min_h(0.).w_full().child(
        col()
            .key("login-scroll")
            .grow(1.)
            .min_h(0.)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            // the content stands centred in the window, and scrolls once it is taller
            .child(col().w_full().min_h(tall).center().child(content)),
    )
}

/// A provider as its column draws it.
struct Column {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    disabled: bool,
    cancel: bool,
    failure: Option<Failure>,
}

/// The chosen provider's column: its mark and name, a failure that belongs to it, and a button
/// per way to sign in, with Cancel while its sign-in runs. Guest mode has its own button under
/// the column, so its option is left out here.
fn column(ui: &Ui, column: Column) -> El {
    let theme = &ui.theme;
    let Column {
        slug,
        name,
        options,
        web_sign_in,
        disabled,
        cancel,
        failure,
    } = column;
    let buttons: Vec<El> = options
        .iter()
        .filter(|option| !matches!(option, SignIn::Anonymous))
        .flat_map(|method| offered(slug, name, method, web_sign_in))
        .map(|option| option_button(slug, name, option, disabled, theme))
        .chain(cancel.then(|| {
            Button::new("cancel-sign-in")
                .label(t!("common-cancel"))
                .outline()
                .on_press(act(abandon))
                .build(theme)
                .w_full()
                .justify_center()
        }))
        .enumerate()
        .map(|(index, button)| {
            button
                .transition(rise())
                .enter_delay(STEP * (index + 1) as f32)
        })
        .collect();

    let mut column = col()
        .key(format!("login-column-{slug}"))
        .items_center()
        .gap(rems(theme, 0.75))
        .w(COLUMN)
        .child(
            icon(icons::get(provider_logo(slug)), LOGO)
                .color(theme.foreground.color())
                .a11y_hidden(),
        )
        .child(
            spoken("login-provider-name", name, theme)
                .role("heading")
                .level(2)
                .font(font(theme, Text::Large, Weight::SEMIBOLD)),
        );
    if let Some(failure) = failure {
        column = column.child(trouble(failure, true, theme));
    }
    column
        .child(
            col()
                .key("login-options")
                .w_full()
                .gap(rems(theme, 0.5))
                .children(buttons),
        )
        // switching tabs lays the next service's column over the last one as it fades
        .transition(Transition::asymmetric(rise(), Transition::opacity()))
}

/// The buttons one sign-in method yields. A cookie sign-in yields the browser window only where
/// a backend can draw one, and always the manual paste beside it.
fn offered(slug: &str, provider: &str, method: &SignIn, web_sign_in: bool) -> Vec<Choice> {
    let sign_in = || t!("login-sign-in", provider = provider).into_owned();
    match method {
        SignIn::Secret => {
            let mut options = Vec::new();
            if web_sign_in {
                options.push(Choice {
                    key: format!("sign-in-{slug}-cookies"),
                    label: sign_in(),
                    action: Action::SignIn(SignIn::Secret),
                    primary: true,
                });
            }
            options.push(Choice {
                key: format!("sign-in-{slug}-cookies-manual"),
                label: t!("login-connect-cookies").into_owned(),
                action: Action::Cookies,
                primary: false,
            });
            options
        }
        SignIn::Default => vec![Choice {
            key: format!("sign-in-{slug}"),
            label: sign_in(),
            action: Action::SignIn(method.clone()),
            primary: true,
        }],
        SignIn::Anonymous => vec![Choice {
            key: format!("sign-in-{slug}-guest"),
            label: t!("login-use", provider = provider).into_owned(),
            action: Action::SignIn(method.clone()),
            primary: true,
        }],
        SignIn::Path(_) => vec![Choice {
            key: format!("sign-in-{slug}-path"),
            label: sign_in(),
            action: Action::SignIn(method.clone()),
            primary: false,
        }],
        SignIn::Credentials { .. } => vec![Choice {
            key: format!("sign-in-{slug}-server"),
            label: sign_in(),
            action: Action::Credentials,
            primary: true,
        }],
    }
}

fn option_button(
    slug: &'static str,
    provider: &'static str,
    option: Choice,
    disabled: bool,
    theme: &Theme,
) -> El {
    let Choice {
        key,
        label,
        action,
        primary,
    } = option;
    let button = Button::new(key)
        .label(label)
        .disabled(disabled)
        .on_press(act(move |ui: &mut Ui| match &action {
            Action::SignIn(method) => start(ui, slug, method.clone()),
            Action::Credentials => open_credentials(ui, slug),
            Action::Cookies => start_manual(ui, slug, provider),
        }));
    match primary {
        true => button.primary(),
        false => button.outline(),
    }
    .build(theme)
    .w_full()
    .justify_center()
}

/// The box that lets the listener count towards how many people use Sonora.
fn consent(ui: &Ui) -> El {
    let theme = &ui.theme;
    let checked = ui.store.usage.consented();
    Checkbox::new("usage-consent", checked)
        .label(t!("login-usage-consent"))
        .on_press(act(move |ui: &mut Ui| ui.store.usage.consent(!checked)))
        .build(theme)
        .max_w(COLUMN)
}

/// The code a device sign-in asks the listener to type on the provider's site.
fn code_prompt(code: &str, url: &str, theme: &Theme) -> El {
    col()
        .key("login-code")
        .items_center()
        .gap(rems(theme, 0.5))
        .child(
            spoken(
                "login-device-code",
                t!("login-device-code", url = url),
                theme,
            )
            .font(font(theme, Text::Small, Weight::NORMAL))
            .color(theme.muted_foreground.color()),
        )
        .child(device_code(code, theme))
        .transition(rise())
}

/// The button that copies the page a browser sign-in waits on, for a browser that did not open.
fn url_prompt(url: String, theme: &Theme) -> El {
    Button::new("copy-login-url")
        .icon("icons/copy.svg")
        .label(t!("menu-copy-link"))
        .outline()
        .small()
        .on_press_tx(move |_: &mut Ui, tx: &mut Tx| tx.copy(url.clone()))
        .build(theme)
        .transition(rise())
}

/// Every dialog a sign-in started here raises: the server form, the cookie paste and the
/// account choice.
fn dialogs(ui: &Ui, manual_secret: Option<(&'static str, &'static str)>, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    if let Some((slug, provider)) = manual_secret {
        let secret = ui::Input::new(
            "login-cookie-secret",
            &ui.screens.login.secret,
            i18n::lookup(cookie_hint(slug), None),
            |ui: &mut Ui| &mut st(ui).secret,
        )
        .on_submit(act_tx(|ui: &mut Ui, _: &mut Tx| submit_secret(ui)))
        .build(theme);
        CookiePrompt::new(slug, provider, secret)
            .on_submit(submit_secret)
            .on_cancel(abandon)
            .build(theme, layer);
    }
    if ui.screens.login.credentials_for.is_some() {
        credentials_prompt(ui, layer);
    }
    if let SessionState::Authorizing(Some(SignInPrompt::Accounts(accounts))) =
        ui.store.session.state()
    {
        account_picker(
            accounts,
            |ui: &mut Ui, id: String| {
                acted(ui);
                ui.store.session.submit_input(id, &mut ui.store.cx);
            },
            abandon,
            theme,
            layer,
        );
    }
}

/// The dialog a server sign-in asks for its address and credentials in.
fn credentials_prompt(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let page = &ui.screens.login;
    let field = |key: &str, edit: &TextEdit, hint: &str, access: fn(&mut Ui) -> &mut TextEdit| {
        ui::Input::new(key, edit, i18n::lookup(hint, None), access)
            .on_submit(act_tx(|ui: &mut Ui, _: &mut Tx| submit_credentials(ui)))
    };
    Modal::<Ui>::new("server-prompt", t!("login-server-title"))
        .width(SERVER_WIDTH)
        .detail(t!("login-server-detail"))
        .child(
            field("login-server", &page.server, "login-server-hint", |ui| {
                &mut st(ui).server
            })
            .build(theme),
        )
        .child(
            field(
                "login-username",
                &page.username,
                "login-username-hint",
                |ui| &mut st(ui).username,
            )
            .build(theme),
        )
        .child(
            field(
                "login-password",
                &page.password,
                "login-password-hint",
                |ui| &mut st(ui).password,
            )
            .masked(true)
            .build(theme),
        )
        .action(
            Button::new("cancel-server")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(abandon_credentials))
                .build(theme),
        )
        .action(
            Button::new("submit-server")
                .label(t!("login-server-submit"))
                .primary()
                .on_press(act(submit_credentials))
                .build(theme),
        )
        .on_dismiss(act(abandon_credentials))
        .build(theme, layer);
}

// Handlers.

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.login
}

/// Counts the listener as having used the page, for the usage estimate they agreed to.
fn acted(ui: &mut Ui) {
    ui.store.usage.report(&mut ui.store.cx);
}

/// Starts signing in to `slug` by `method`, keeping this page up while it runs.
fn start(ui: &mut Ui, slug: &'static str, method: SignIn) {
    acted(ui);
    st(ui).manual_secret = None;
    ui.chrome.signing_in = true;
    ui.store.session.sign_in(slug, method, &mut ui.store.cx);
}

/// Starts the manual cookie sign-in, whose dialog asks for the cookie once the session waits on
/// it.
fn start_manual(ui: &mut Ui, slug: &'static str, provider: &'static str) {
    acted(ui);
    let state = st(ui);
    state.manual_secret = Some((slug, provider));
    state.secret.focused = true;
    ui.chrome.signing_in = true;
    ui.store
        .session
        .sign_in_with_cookies(slug, &mut ui.store.cx);
}

fn submit_secret(ui: &mut Ui) {
    let text = st(ui).secret.text.clone();
    if text.trim().is_empty() {
        return;
    }
    acted(ui);
    clear_secret(ui);
    ui.store.session.submit_input(text, &mut ui.store.cx);
}

fn clear_secret(ui: &mut Ui) {
    let state = st(ui);
    state.manual_secret = None;
    state.secret.clear();
}

fn open_credentials(ui: &mut Ui, slug: &'static str) {
    acted(ui);
    let state = st(ui);
    state.credentials_for = Some(slug);
    state.server.focused = true;
}

fn clear_credentials(ui: &mut Ui) {
    let state = st(ui);
    state.credentials_for = None;
    state.server.clear();
    state.username.clear();
    state.password.clear();
}

fn abandon_credentials(ui: &mut Ui) {
    acted(ui);
    clear_credentials(ui);
}

fn submit_credentials(ui: &mut Ui) {
    let state = st(ui);
    let Some(slug) = state.credentials_for else {
        return;
    };
    let server = state.server.text.clone();
    let username = state.username.text.clone();
    let password = state.password.text.clone();
    if server.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
        return;
    }
    clear_credentials(ui);
    start(
        ui,
        slug,
        SignIn::Credentials {
            server,
            username,
            password,
        },
    );
}

/// Stops the sign-in under way, and empties whatever its dialogs held.
fn abandon(ui: &mut Ui) {
    acted(ui);
    clear_secret(ui);
    clear_credentials(ui);
    ui.store.session.cancel_sign_in(&mut ui.store.cx);
}

// Free helpers.

/// How a part of the page comes in: up from a little below, fading in.
fn rise() -> Transition {
    Transition::offset(0., RISE).combined(Transition::opacity())
}

/// The icon a provider's mark is drawn from. The Providers settings keep the same table
/// privately; this copy stays until `shared` carries the one every screen uses.
fn provider_logo(slug: &str) -> &'static str {
    match slug {
        "spotify" => "icons/spotify.svg",
        "youtube" => "icons/youtubemusic.svg",
        "subsonic" => "icons/subsonic.svg",
        "deezer" => "icons/deezer.svg",
        "apple" => "icons/applemusic.svg",
        "tidal" => "icons/tidal.svg",
        _ => "icons/music.svg",
    }
}
