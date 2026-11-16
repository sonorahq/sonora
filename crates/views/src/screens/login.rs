use std::time::{Duration, Instant};

use crate::shared::device_code::device_code;
use crate::shared::popups::{AccountPicker, CookiePrompt, Credentials, CredentialsPrompt};
use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, BoxShadow, ClipboardItem, Context, ElementId, Entity, FontWeight, Hsla,
    IntoElement, MouseMoveEvent, Pixels, Point, Render, SharedString, Task, Window, div, hsla,
    img, point, px, relative, svg,
};
use i18n::t;
use music::{AccountChoices, SignIn, SignInProblem, SignInPrompt};
use state::{Failure, ProviderInfo, Session, SessionState, Sonora, Usage};
use ui::ActiveTheme as _;
use ui::{Button, Checkbox, HoverFade, Input, LEADING, Lattice, Separator, Text, mix};

use super::settings::{Offer, ProviderRow};

/// How wide the sign-in card and the split layout's form stand.
const CARD: Pixels = px(400.);
const FORM: Pixels = px(380.);
const LOGO: Pixels = px(48.);
/// The Sonora app icon over the welcome, the larger one the stage opens with, and the split panel's.
const MARK: Pixels = px(64.);
const STAGE_MARK: Pixels = px(88.);
const SPLIT_ICON: Pixels = px(32.);
/// The app icon as this platform shows it, registered by the binary's asset source.
const APP_ICON: &str = "brand/sonora.svg";
/// How hard the frosted card blurs the lattice behind it.
const FROST: Pixels = px(28.);
/// A stage tile's size, its mark and how far its brand glow spreads under the pointer.
const TILE_W: Pixels = px(150.);
const TILE_H: Pixels = px(124.);
const TILE_MARK: Pixels = px(30.);
const SPLIT_MARK: Pixels = px(22.);
const GLOW: Pixels = px(36.);
const TILE_FADE: Duration = Duration::from_millis(180);
/// How many 120 Hz ticks pass between the lattice's frames, which makes 60 Hz.
const TICKS: u32 = 2;
/// How far the spotlight's colour moves toward its target each frame.
const LAMP_EASE: f32 = 0.14;
/// The pools that light the lattice: indigo, violet and a cool teal, so no provider's brand
/// colour owns the backdrop. The spotlight takes the brand of whatever lies under the pointer.
const POOLS: [(f32, f32, f32, f32); 3] = [
    (0.66, 0.75, 0.62, 0.95),
    (0.78, 0.70, 0.64, 0.85),
    (0.50, 0.65, 0.52, 0.7),
];
/// The split layout's art panel stays dark in either theme.
const PANEL: (f32, f32, f32) = (0.67, 0.12, 0.06);

/// Which of the prototype layouts the sign-in screen draws, picked by `SONORA_LOGIN`.
#[derive(Clone, Copy, PartialEq)]
enum Variant {
    /// A frosted card of provider rows over the lattice.
    Card,
    /// Art on the left, a form with a grid of provider tiles on the right.
    Split,
    /// Large frosted tiles straight on the lattice, glowing in their brand under the pointer.
    Stage,
}

impl Variant {
    fn from_env() -> Self {
        match std::env::var("SONORA_LOGIN").as_deref() {
            Ok("split") => Self::Split,
            Ok("stage") => Self::Stage,
            _ => Self::Card,
        }
    }
}

struct Column {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    disabled: bool,
    cancel: bool,
    failure: Option<Failure>,
}

enum LoginAction {
    SignIn(SignIn),
    Credentials,
    Cookies(&'static str),
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
    credentials: Credentials,
    credentials_for: Option<&'static str>,
    manual_secret: Option<(&'static str, &'static str)>,
    variant: Variant,
    /// The provider whose sign-in methods are open, in place of the list of providers.
    chosen: Option<&'static str>,
    /// The provider under the pointer, whose brand the spotlight warms to.
    hovered: Option<&'static str>,
    pointer: Option<Point<Pixels>>,
    lamp: Hsla,
    started: Instant,
    pacing: Option<Task<()>>,
}

impl LoginView {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let usage = Sonora::global(cx).usage.clone();
        cx.observe(&usage, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            usage,
            secret: cx.new(|cx| Input::new("login-cookie-hint", cx)),
            credentials: Credentials::new(cx),
            credentials_for: None,
            manual_secret: None,
            variant: Variant::from_env(),
            chosen: None,
            hovered: None,
            pointer: None,
            lamp: cx.theme().foreground,
            started: Instant::now(),
            pacing: None,
        }
    }

    fn acted(&self, cx: &mut Context<Self>) {
        self.usage.update(cx, |usage, cx| usage.report(cx));
    }

    fn consent(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.usage.read(cx).consented();
        Checkbox::new("usage-consent", checked)
            .label(t!("login-usage-consent"))
            .max_w(width)
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
        self.credentials.clear(cx);
    }

    fn clear_secret(&mut self, cx: &mut Context<Self>) {
        self.manual_secret = None;
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
        let Some((server, username, password)) = self.credentials.filled(slug, cx) else {
            return;
        };
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
        self.manual_secret = None;
        self.session
            .update(cx, |session, cx| session.sign_in(slug, method, cx));
    }

    fn start_manual(&mut self, slug: &'static str, provider: &'static str, cx: &mut Context<Self>) {
        self.acted(cx);
        self.manual_secret = Some((slug, provider));
        let hint = CookiePrompt::hint(slug);
        self.secret.update(cx, |input, cx| input.set_hint(hint, cx));
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

    /// Opens a provider. One way in goes straight through and shows the provider waiting on
    /// it, a server asks for its address in a dialog, and a choice of ways puts the choice up
    /// in place of the list.
    fn pick(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        let Some(info) = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
        else {
            return;
        };
        self.acted(cx);
        let methods: Vec<SignIn> = info
            .options
            .iter()
            .filter(|option| !matches!(option, SignIn::Anonymous))
            .cloned()
            .collect();
        let buttons: usize = methods
            .iter()
            .map(|method| method_count(method, info.web_sign_in))
            .sum();
        match methods.as_slice() {
            [SignIn::Credentials { .. }] => self.open_credentials(slug, cx),
            [SignIn::Secret] if buttons == 1 => self.start_manual(slug, info.name, cx),
            [method] if buttons == 1 => {
                self.chosen = Some(slug);
                self.start(slug, method.clone(), cx);
            }
            _ => {
                self.chosen = Some(slug);
                cx.notify();
            }
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).is_pending() {
            self.abandon(cx);
        }
        self.chosen = None;
        cx.notify();
    }

    fn hover(&mut self, slug: &'static str, inside: bool, cx: &mut Context<Self>) {
        match inside {
            true => self.hovered = Some(slug),
            false if self.hovered == Some(slug) => self.hovered = None,
            false => {}
        }
        cx.notify();
    }

    /// Asks for the lattice's next frame on the shared tick, and eases the spotlight's colour
    /// one step toward the brand under the pointer.
    fn pace(&mut self, target: Hsla, window: &mut Window, cx: &mut Context<Self>) {
        self.lamp = mix(self.lamp, target, LAMP_EASE);
        if self.pacing.is_some() {
            return;
        }
        let timer = crate::shared::animation::next(TICKS, cx);
        let handle = window.window_handle();
        self.pacing = Some(cx.spawn(async move |this, cx| {
            timer.await;
            if this
                .update(cx, |this, cx| {
                    this.pacing = None;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            handle
                .update(cx, |_, window, _| window.on_next_frame(|_, _| {}))
                .ok();
        }));
    }

    fn option_buttons(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: &SignIn,
        web_sign_in: bool,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Vec<Button> {
        let options = match method {
            SignIn::Secret => {
                let mut options = Vec::new();
                if web_sign_in {
                    options.push(LoginOption {
                        id: format!("sign-in-{slug}-cookies").into(),
                        label: t!("login-sign-in", provider = provider),
                        action: LoginAction::SignIn(SignIn::Secret),
                        primary: true,
                    });
                }
                options.push(LoginOption {
                    id: format!("sign-in-{slug}-cookies-manual").into(),
                    label: t!("login-connect-cookies"),
                    action: LoginAction::Cookies(provider),
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
                LoginAction::Cookies(provider) => this.start_manual(slug, provider, cx),
            }));
        match option.primary {
            true => button.primary(),
            false => button.outline(),
        }
    }

    /// One provider opened: its mark and name, its error, and a button for every way in, or
    /// the cancel button while it waits on one.
    fn column(&self, column: Column, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let Column {
            slug,
            name,
            options,
            web_sign_in,
            disabled,
            cancel,
            failure,
        } = column;
        let options = options
            .iter()
            .filter(|option| !matches!(option, SignIn::Anonymous));

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w_full()
            .child(
                svg()
                    .path(icons::path(crate::shared::provider_logo(slug)))
                    .size(LOGO)
                    .flex_none()
                    .text_color(crate::shared::provider_brand(slug, cx)),
            )
            .child({
                let name = SharedString::from(name.to_string());
                div()
                    .id("login-provider-name")
                    .role(Role::Heading)
                    .aria_level(2)
                    .aria_label(name.clone())
                    .text_size(theme.text(Text::Large))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name)
            })
            .when_some(failure, |this, failure| {
                this.child(crate::shared::trouble::trouble(failure, true, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .pt_2()
                    .when(!cancel, |this| {
                        this.children(options.into_iter().flat_map(|method| {
                            self.option_buttons(slug, name, method, web_sign_in, disabled, cx)
                        }))
                    })
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

    /// The opened provider with the way back to the list above it, and whatever its sign-in
    /// is waiting on under it.
    fn step(
        &self,
        info: ProviderInfo,
        pending: bool,
        waiting: bool,
        prompt: Prompt,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = Column {
            slug: info.slug,
            name: info.name,
            options: info.options,
            web_sign_in: info.web_sign_in,
            disabled: pending,
            cancel: waiting && info.pending,
            failure: info.error,
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .w_full()
            .child(
                div().absolute().top_0().left_0().child(
                    Button::new("login-back")
                        .ghost()
                        .small()
                        .icon("icons/chevron-left.svg")
                        .tooltip("common-previous")
                        .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                ),
            )
            .child(self.column(column, cx))
            .when_some(prompt.code, |this, (code, url)| {
                this.child(self.code_prompt(code, url, cx).into_any_element())
            })
            .when_some(prompt.url, |this, url| this.child(self.url_prompt(url)))
            .into_any_element()
    }

    fn guest_mode(&self, slug: &'static str, pending: bool, cx: &mut Context<Self>) -> Button {
        Button::new("guest-mode")
            .label(t!("login-guest-use"))
            .icon("icons/hat-glasses.svg")
            .outline()
            .w_full()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.start(slug, SignIn::Anonymous, cx);
            }))
    }

    /// A rule with "or" in its middle, between the providers and guest mode.
    fn divider(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .child(div().flex_1().child(Separator::horizontal()))
            .child(
                div()
                    .text_size(theme.text(Text::Tiny))
                    .text_color(theme.muted_foreground)
                    .child(t!("login-or")),
            )
            .child(div().flex_1().child(Separator::horizontal()))
    }

    fn code_prompt(&self, code: String, url: String, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                ui::spoken("login-device-code", t!("login-device-code", url = &url))
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground),
            )
            .child(device_code(code, cx))
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

    fn account_modal(&self, accounts: AccountChoices, cx: &mut Context<Self>) -> impl IntoElement {
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
        CredentialsPrompt::new(&self.credentials)
            .provider(self.credentials_for.unwrap_or("subsonic"))
            .on_submit(cx.listener(|this, _, _, cx| this.submit_credentials(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon_credentials(cx)))
    }

    fn secret_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        CookiePrompt::new(slug, provider, self.secret.clone())
            .on_submit(cx.listener(|this, _, _, cx| this.submit_secret(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    /// The Sonora mark, a title and the line under it, centred.
    fn welcome(
        &self,
        mark: Pixels,
        title: SharedString,
        size: Text,
        detail: SharedString,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                img(APP_ICON).size(mark).mb_2(),
            )
            .child(
                div()
                    .id("login-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .aria_label(title.clone())
                    .text_size(theme.text(size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id("login-status")
                    .role(Role::Status)
                    .aria_label(detail.clone())
                    .max_w(px(420.))
                    .text_center()
                    .line_height(relative(LEADING))
                    .text_size(theme.text(Text::Body))
                    .text_color(theme.muted_foreground)
                    .child(detail),
            )
    }

    /// The providers as the settings page lists them: soft rows that warm to their brand under
    /// the pointer.
    fn rows(&self, providers: &[ProviderInfo], pending: bool, cx: &mut Context<Self>) -> AnyElement {
        let count = providers.len();
        let rows = providers.iter().enumerate().map(|(index, info)| {
            let slug = info.slug;
            ProviderRow {
                id: SharedString::from(format!("login-{slug}")),
                logo: crate::shared::provider_logo(slug),
                brand: crate::shared::provider_brand(slug, cx),
                active: false,
                name: info.name.into(),
                status: None,
                offer: Offer::SignIn,
                press: (!pending).then(|| -> super::settings::Press {
                    Box::new(cx.listener(move |this, _, _, cx| this.pick(slug, cx)))
                }),
                error: info.error.clone(),
                place: (index, count),
                hovered: Some(Box::new(cx.listener(move |this, inside: &bool, _, cx| {
                    this.hover(slug, *inside, cx)
                }))),
            }
            .into_any_element()
        });
        div()
            .id("login-providers")
            .role(Role::List)
            .aria_label(t!("login-providers"))
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .px_3()
            .children(rows)
            .into_any_element()
    }

    /// One provider as a tile: its mark in its brand colour over its name and what kind of
    /// service it is. Under the pointer the edge and a soft glow take on the brand.
    fn tile(
        &self,
        info: &ProviderInfo,
        (index, count): (usize, usize),
        pending: bool,
        stage: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let slug = info.slug;
        let id = SharedString::from(format!("login-tile-{slug}"));
        let brand = crate::shared::provider_brand(slug, cx);
        let fade = window.use_keyed_state((ElementId::from(id.clone()), "glow"), cx, |_, _| {
            HoverFade::new(TILE_FADE)
        });
        let lit = fade.update(cx, |fade, cx| fade.level(window, cx));
        let kind = match info
            .options
            .iter()
            .any(|option| matches!(option, SignIn::Credentials { .. }))
        {
            true => t!("login-kind-server"),
            false => t!("login-kind-service"),
        };
        let rest = match stage {
            true => theme.border.opacity(0.5),
            false => theme.border,
        };
        let edge = mix(rest, brand.opacity(0.7), lit);
        let fill = match stage {
            true => mix(theme.background.opacity(0.35), brand.opacity(0.12), lit),
            false => mix(theme.background.opacity(0.), brand.opacity(0.07), lit),
        };
        let name = SharedString::from(info.name);

        div()
            .id(id)
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_description(kind.clone())
            .aria_position_in_set(index + 1)
            .aria_size_of_set(count)
            .flex()
            .rounded(theme.radius)
            .border_1()
            .border_color(edge)
            .bg(fill)
            .when(stage, |this| {
                this.w(TILE_W)
                    .h(TILE_H)
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .backdrop_blur(FROST)
            })
            .when(!stage, |this| this.items_center().gap_3().px_3().py_3())
            .shadow(vec![BoxShadow {
                color: brand.opacity(0.35 * lit),
                offset: point(px(0.), px(0.)),
                blur_radius: GLOW,
                spread_radius: px(0.),
                inset: false,
            }])
            .when(!pending, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(slug, cx)))
            })
            .on_hover(cx.listener(move |this, inside: &bool, window, cx| {
                fade.update(cx, |fade, _| fade.set(*inside));
                this.hover(slug, *inside, cx);
                window.refresh();
            }))
            .child(
                svg()
                    .path(icons::path(crate::shared::provider_logo(slug)))
                    .size(match stage {
                        true => TILE_MARK,
                        false => SPLIT_MARK,
                    })
                    .flex_none()
                    .text_color(match stage {
                        true => mix(theme.foreground, brand, lit),
                        false => brand,
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .when(stage, |this| this.items_center())
                    .child(
                        div()
                            .truncate()
                            .text_size(theme.text(Text::Body))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(kind),
                    ),
            )
            .into_any_element()
    }

    fn lattice(&self, theme: &ui::Theme, animates: bool) -> Lattice {
        Lattice::new(POOLS.map(|(h, s, l, a)| hsla(h, s, l, a)))
            .stage(Hsla {
                a: 1.,
                ..theme.background
            })
            .lamp(self.lamp)
            .pointer(self.pointer)
            .time(match animates {
                true => self.started.elapsed().as_secs_f32(),
                false => 0.,
            })
    }
}

/// What a waiting sign-in asks the person to do outside the app.
#[derive(Default)]
struct Prompt {
    code: Option<(String, String)>,
    url: Option<String>,
}

impl Render for LoginView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.session.read(cx).state().clone();
        let pending = self.session.read(cx).is_pending();
        let providers: Vec<ProviderInfo> = self.session.read(cx).providers().collect();
        let guest = providers
            .iter()
            .filter(|info| {
                info.options
                    .iter()
                    .any(|option| matches!(option, SignIn::Anonymous))
            })
            .map(|info| info.slug)
            .next();
        let manual_secret = self.manual_secret.filter(|_| {
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
        let failure = providers
            .iter()
            .find_map(|info| info.error.clone())
            .filter(|failure| failure.problem != Some(SignInProblem::Subscription));

        let status = match &state {
            SessionState::SignedOut
            | SessionState::Offline(_)
            | SessionState::Failed(_)
            | SessionState::Authorizing(Some(SignInPrompt::Accounts(_))) => {
                t!("login-welcome-detail")
            }
            SessionState::Restoring => t!("login-restoring"),
            SessionState::Authorizing(_) => t!("login-authorizing"),
            SessionState::SignedIn(profile) => t!("login-signed-in", name = &profile.display_name),
        };

        let prompt = match &state {
            SessionState::Authorizing(prompt) => prompt.clone(),
            _ => None,
        };
        let accounts = match &prompt {
            Some(SignInPrompt::Accounts(accounts)) => Some(accounts.clone()),
            _ => None,
        };
        let asks = Prompt {
            code: match prompt {
                Some(SignInPrompt::Code { code, url }) => Some((code, url)),
                _ => None,
            },
            url: match &state {
                SessionState::Authorizing(Some(SignInPrompt::Url(url))) => Some(url.clone()),
                _ => None,
            },
        };

        let theme = *cx.theme();
        let asking = self.usage.read(cx).asking();
        let animates = ui::motion::animates(cx);
        let target = match self.hovered {
            Some(slug) => crate::shared::provider_brand(slug, cx),
            None => theme.foreground,
        };
        match animates {
            true => self.pace(target, window, cx),
            false => self.lamp = target,
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        let opened = self
            .chosen
            .and_then(|slug| providers.iter().find(|info| info.slug == slug).cloned());
        let count = providers.len();
        let body: AnyElement = match self.variant {
            Variant::Card => {
                let inner = match opened {
                    Some(info) => self.step(info, pending, waiting, asks, cx),
                    None => div()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .w_full()
                        .child(self.rows(&providers, pending, cx))
                        .when_some(guest, |this, slug| {
                            this.child(self.divider(cx))
                                .child(self.guest_mode(slug, pending, cx))
                        })
                        .into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .w_full()
                    .min_h_full()
                    .p(theme.metrics.pad)
                    .pt(theme.metrics.title_bar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .w(CARD)
                            .p_8()
                            .rounded(ui::corners(theme.radius))
                            .border_1()
                            .border_color(theme.border.opacity(0.7))
                            .bg(theme.background.opacity(0.62))
                            .backdrop_blur(FROST)
                            .shadow(vec![BoxShadow {
                                color: hsla(0., 0., 0., 0.35),
                                offset: point(px(0.), px(24.)),
                                blur_radius: px(64.),
                                spread_radius: px(-12.),
                                inset: false,
                            }])
                            .child(self.welcome(
                                MARK,
                                t!("login-welcome-title"),
                                Text::Title,
                                status,
                                cx,
                            ))
                            .child(inner),
                    )
                    .when(asking, |this| this.child(self.consent(CARD, cx)))
                    .into_any_element()
            }
            Variant::Split => {
                let tiles = providers
                    .iter()
                    .enumerate()
                    .map(|(index, info)| self.tile(info, (index, count), pending, false, window, cx))
                    .collect::<Vec<_>>();
                let inner = match opened {
                    Some(info) => self.step(info, pending, waiting, asks, cx),
                    None => div()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .w_full()
                        .child(
                            div()
                                .id("login-providers")
                                .role(Role::List)
                                .aria_label(t!("login-providers"))
                                .grid()
                                .grid_cols(2)
                                .gap_2()
                                .w_full()
                                .children(tiles),
                        )
                        .when_some(guest, |this, slug| {
                            this.child(self.divider(cx))
                                .child(self.guest_mode(slug, pending, cx))
                        })
                        .into_any_element(),
                };
                let names = providers
                    .iter()
                    .map(|info| info.name)
                    .collect::<Vec<_>>()
                    .join(" · ");
                let (h, s, l) = PANEL;
                let panel = hsla(h, s, l, 1.);
                div()
                    .flex()
                    .size_full()
                    .p_2()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .w(relative(0.48))
                            .h_full()
                            .p_10()
                            .rounded(ui::corners(theme.radius))
                            .overflow_hidden()
                            .bg(panel)
                            .text_color(gpui::white())
                            .child(
                                div().absolute().inset_0().child(
                                    Lattice::new(POOLS.map(|(h, s, l, a)| hsla(h, s, l, a)))
                                        .stage(panel)
                                        .lamp(self.lamp)
                                        .pointer(self.pointer)
                                        .corner_radius(ui::corners(theme.radius))
                                        .time(match animates {
                                            true => self.started.elapsed().as_secs_f32(),
                                            false => 0.,
                                        }),
                                ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        img(APP_ICON).size(SPLIT_ICON),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text(Text::Heading))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Sonora"),
                                    ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .max_w(px(460.))
                                    .child(
                                        div()
                                            .text_size(theme.text(Text::Display))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .line_height(relative(1.1))
                                            .child(t!("login-tagline")),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text(Text::Small))
                                            .text_color(hsla(0., 0., 1., 0.6))
                                            .child(SharedString::from(names)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("login-scroll")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_center()
                                    .min_h_full()
                                    .p(theme.metrics.pad)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_6()
                                            .w(FORM)
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .id("login-title")
                                                            .role(Role::Heading)
                                                            .aria_level(1)
                                                            .aria_label(t!("login-sign-in-title"))
                                                            .text_size(theme.text(Text::Title))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child(t!("login-sign-in-title")),
                                                    )
                                                    .child(
                                                        ui::spoken("login-status", status)
                                                            .line_height(relative(LEADING))
                                                            .text_color(theme.muted_foreground),
                                                    ),
                                            )
                                            .when_some(failure.clone().filter(|_| opened_none(self.chosen)), |this, failure| {
                                                this.child(crate::shared::trouble::trouble(failure, false, cx))
                                            })
                                            .child(inner)
                                            .when(asking, |this| this.child(self.consent(FORM, cx))),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            Variant::Stage => {
                let tiles = providers
                    .iter()
                    .enumerate()
                    .map(|(index, info)| self.tile(info, (index, count), pending, true, window, cx))
                    .collect::<Vec<_>>();
                let inner = match opened {
                    Some(info) => div()
                        .w(CARD)
                        .p_8()
                        .rounded(ui::corners(theme.radius))
                        .border_1()
                        .border_color(theme.border.opacity(0.6))
                        .bg(theme.background.opacity(0.55))
                        .backdrop_blur(FROST)
                        .child(self.step(info, pending, waiting, asks, cx))
                        .into_any_element(),
                    None => div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_6()
                        .child(
                            div()
                                .id("login-providers")
                                .role(Role::List)
                                .aria_label(t!("login-providers"))
                                .flex()
                                .flex_wrap()
                                .justify_center()
                                .gap_3()
                                .max_w(TILE_W * 4. + px(12.) * 3.)
                                .children(tiles),
                        )
                        .when_some(guest, |this, slug| {
                            this.child(
                                Button::new("guest-mode")
                                    .label(t!("login-guest-use"))
                                    .icon("icons/hat-glasses.svg")
                                    .ghost()
                                    .tint(theme.muted_foreground)
                                    .disabled(pending)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.start(slug, SignIn::Anonymous, cx);
                                    })),
                            )
                        })
                        .into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_10()
                    .w_full()
                    .min_h_full()
                    .p(theme.metrics.pad)
                    .pt(theme.metrics.title_bar)
                    .child(self.welcome(
                        STAGE_MARK,
                        t!("login-welcome-title"),
                        Text::Display,
                        status,
                        cx,
                    ))
                    .when_some(failure.filter(|_| opened_none(self.chosen)), |this, failure| {
                        this.child(crate::shared::trouble::trouble(failure, true, cx))
                    })
                    .child(inner)
                    .when(asking, |this| this.child(self.consent(CARD, cx)))
                    .into_any_element()
            }
        };

        let backdrop = (self.variant != Variant::Split).then(|| {
            div()
                .absolute()
                .inset_0()
                .child(self.lattice(&theme, animates).when_some(radius, |this, radius| {
                    this.corner_radius(radius)
                }))
        });

        div()
            .id("login")
            .relative()
            .size_full()
            .overflow_hidden()
            .when_some(radius, |this, radius| this.rounded(radius))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.pointer = Some(event.position);
                cx.notify();
            }))
            .children(backdrop)
            .map(|this| match self.variant {
                Variant::Split => this.child(body),
                _ => this.child(
                    div()
                        .id("login-scroll")
                        .relative()
                        .size_full()
                        .overflow_y_scroll()
                        .child(body),
                ),
            })
            .when_some(manual_secret, |this, (slug, provider)| {
                this.child(self.secret_prompt(slug, provider, cx).into_any_element())
            })
            .when(self.credentials_for.is_some(), |this| {
                this.child(self.credentials_prompt(cx).into_any_element())
            })
            .when_some(accounts, |this, accounts| {
                this.child(self.account_modal(accounts, cx).into_any_element())
            })
    }
}

/// How many buttons one sign-in method draws: the method itself, and the manual paste beside
/// a cookie one.
fn method_count(method: &SignIn, web_sign_in: bool) -> usize {
    let manual = matches!(method, SignIn::Secret);
    usize::from(!manual || web_sign_in) + usize::from(manual)
}

fn opened_none(chosen: Option<&'static str>) -> bool {
    chosen.is_none()
}
