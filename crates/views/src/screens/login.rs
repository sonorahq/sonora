use std::time::Instant;

use crate::shared::device_code::device_code;
use crate::shared::popups::{AccountPicker, CookiePrompt, Credentials, CredentialsPrompt};
use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, FontWeight, Hsla, IntoElement, Pixels, Point,
    Render, SharedString, Task, Window, div, hsla, img, point, px, relative, svg,
};
use i18n::t;
use music::{AccountChoices, SignIn, SignInPrompt};
use state::{ProviderInfo, Session, SessionState, Sonora, Usage};
use ui::ActiveTheme as _;
use ui::{
    Button, Checkbox, Input, LEADING, Lattice, Modal, Scrollbar, Scroller, Separator, Text, mix,
};

/// How wide the sign-in card stands.
const CARD: Pixels = px(400.);
/// The app icon over the welcome.
const ICON: Pixels = px(64.);
/// The app icon as this platform shows it, registered by the binary's asset source.
const APP_ICON: &str = "brand/sonora.svg";
/// A provider's mark in its row.
const MARK: Pixels = px(22.);
/// The space between two provider rows.
const ROW_GAP: Pixels = px(4.);
/// How hard the frosted card blurs the lattice behind it.
const FROST: Pixels = px(28.);
/// How tall the bands are where the provider list fades out at its ends when the window is
/// too short to show it whole.
const FADE: Pixels = px(16.);
/// How wide a sign-in choice or code dialog stands, as on the settings page.
const DIALOG: Pixels = px(420.);
/// How many buttons tall the sign-in choice stands, however few it lists.
const CHOICES: f32 = 3.;
/// How many 120 Hz ticks pass between frames while the pools only drift, which makes 30 Hz, and
/// while a glow fades in or out, which makes 60 Hz.
const DRIFT_TICKS: u32 = 4;
const FADE_TICKS: u32 = 2;
/// How far the glow's strength and colour move toward their targets each frame.
const EASE: f32 = 0.3;
/// How close the glow has to come to its target before it counts as settled.
const SETTLED: f32 = 0.01;
/// The pools that light the lattice: indigo, violet and a cool teal, so no provider's brand
/// colour owns the backdrop. The glow behind a hovered provider takes that provider's brand.
const POOLS: [(f32, f32, f32, f32); 3] = [
    (0.66, 0.75, 0.62, 0.95),
    (0.78, 0.70, 0.64, 0.85),
    (0.50, 0.65, 0.52, 0.7),
];

pub struct LoginView {
    session: Entity<Session>,
    usage: Entity<Usage>,
    secret: Entity<Input>,
    credentials: Credentials,
    credentials_for: Option<&'static str>,
    manual_secret: Option<(&'static str, &'static str)>,
    /// The provider whose sign-in methods are up in a dialog, with its name.
    choosing: Option<(&'static str, &'static str)>,
    /// Whether a method picked in that dialog is running, so the dialog closes once it settles.
    choice_running: bool,
    /// The provider under the pointer, whose brand the glow behind it takes.
    hovered: Option<&'static str>,
    /// Where the glow sits: behind the middle of the card at the height the pointer entered
    /// the hovered row. It stays put while the glow fades out.
    glow_at: Option<Point<Pixels>>,
    glow: f32,
    lamp: Hsla,
    started: Instant,
    pacing: Option<Task<()>>,
    scrollbar: Entity<Scrollbar>,
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
            choosing: None,
            choice_running: false,
            hovered: None,
            glow_at: None,
            glow: 0.,
            lamp: cx.theme().foreground,
            started: Instant::now(),
            pacing: None,
            scrollbar: cx.new(|_| Scrollbar::inset()),
        }
    }

    fn acted(&self, cx: &mut Context<Self>) {
        self.usage.update(cx, |usage, cx| usage.report(cx));
    }

    fn consent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checked = self.usage.read(cx).consented();
        Checkbox::new("usage-consent", checked)
            .label(t!("login-usage-consent"))
            .max_w(CARD)
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

    /// Starts a sign-in the way the provider asks for it, as the settings page does. One method
    /// goes straight through, a server asks for its address and credentials, and anything with
    /// a choice to make puts the choice up in a dialog.
    fn pick(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        let Some(info) = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
        else {
            return;
        };
        let methods: Vec<SignIn> = info.options.into_iter().filter(offered).collect();
        let buttons: usize = methods
            .iter()
            .map(|method| method_count(method, info.web_sign_in))
            .sum();
        match methods.as_slice() {
            [SignIn::Credentials { .. }] => self.open_credentials(slug, cx),
            [SignIn::Secret] if buttons == 1 => self.start_manual(slug, info.name, cx),
            [method] if buttons == 1 => self.start(slug, method.clone(), cx),
            [] => {}
            _ => {
                self.acted(cx);
                self.choosing = Some((slug, info.name));
                cx.notify();
            }
        }
    }

    fn close_choice(&mut self, cx: &mut Context<Self>) {
        self.choosing = None;
        self.choice_running = false;
        cx.notify();
    }

    /// Lights the glow behind the card at the height the pointer came into the row, in the
    /// provider's brand, and lets it fade once the pointer leaves.
    fn hover(&mut self, slug: &'static str, inside: bool, window: &Window, cx: &mut Context<Self>) {
        match inside {
            true => {
                let entered = window.mouse_position();
                self.hovered = Some(slug);
                self.glow_at = Some(point(window.viewport_size().width / 2., entered.y));
            }
            false if self.hovered == Some(slug) => self.hovered = None,
            false => {}
        }
        cx.notify();
    }

    /// Eases the glow one step toward its targets and asks for the lattice's next frame on the
    /// shared tick, faster while the glow is still moving.
    fn pace(&mut self, lamp: Hsla, window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.hovered {
            Some(_) => 1.,
            None => 0.,
        };
        self.glow += (target - self.glow) * EASE;
        if self.hovered.is_some() {
            self.lamp = mix(self.lamp, lamp, EASE);
        }
        let fading = (target - self.glow).abs() > SETTLED;
        if self.pacing.is_some() {
            return;
        }
        let ticks = match fading {
            true => FADE_TICKS,
            false => DRIFT_TICKS,
        };
        let timer = crate::shared::animation::next(ticks, cx);
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

    /// The Sonora icon, the welcome and the line under it saying what the screen is waiting on.
    fn welcome(&self, status: SharedString, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let title = t!("login-welcome-title");
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(img(APP_ICON).size(ICON).mb_2())
            .child(
                div()
                    .id("login-title")
                    .role(Role::Heading)
                    .aria_level(1)
                    .aria_label(title.clone())
                    .text_size(theme.text(Text::Title))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id("login-status")
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .text_center()
                    .line_height(relative(LEADING))
                    .text_size(theme.text(Text::Body))
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
    }

    /// The providers as plain ghost buttons the width of the card: the mark, muted until the
    /// pointer brings out its brand, the name, and what a click does. The list scrolls inside
    /// the card when the window is too short for it and fades at whichever end has more.
    fn rows(
        &self,
        providers: &[ProviderInfo],
        pending: bool,
        waiting: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let count = providers.len();
        let rows = providers.iter().enumerate().map(|(index, info)| {
            let slug = info.slug;
            let busy = waiting && info.pending;
            let group = SharedString::from(format!("login-row-{slug}"));
            let brand = crate::shared::provider_brand(slug, cx);
            let mark = svg()
                .path(icons::path(crate::shared::provider_logo(slug)))
                .size(MARK)
                .flex_none()
                .text_color(theme.muted_foreground)
                .group_hover(group.clone(), move |style| style.text_color(brand));
            let detail = match busy {
                true => t!("common-cancel"),
                false => t!("settings-provider-sign-in"),
            };
            Button::new(SharedString::from(format!("login-{slug}")))
                .ghost()
                .group(group)
                .leading(mark)
                .label(info.name)
                .detail(detail)
                .trailing("icons/chevron-right.svg")
                .h(theme.metrics.list_row)
                .mx_neg_3()
                .px_3()
                .gap_3()
                .disabled(pending && !busy)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(count)
                .on_hover(cx.listener(move |this, inside: &bool, window, cx| {
                    this.hover(slug, *inside, window, cx)
                }))
                .on_click(cx.listener(move |this, _, _, cx| match busy {
                    true => this.abandon(cx),
                    false => this.pick(slug, cx),
                }))
                .into_any_element()
        });
        // The list stands as tall as its rows, a definite height the scroller can fill, and gives
        // way from there when the card runs out of room.
        let tall = theme.metrics.list_row * count as f32 + ROW_GAP * count.saturating_sub(1) as f32;
        div()
            .id("login-providers")
            .role(Role::List)
            .aria_label(t!("login-providers"))
            .flex()
            .flex_col()
            .h(tall)
            .flex_shrink(1.)
            .min_h_0()
            .child(
                Scroller::new("login-providers-scroll", &self.scrollbar)
                    .fade_ends(FADE)
                    .flex()
                    .flex_col()
                    .gap(ROW_GAP)
                    .px_3()
                    .children(rows),
            )
            .into_any_element()
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

    /// The choice of sign-in methods, for a provider that has more than one, the same dialog
    /// the settings page puts up.
    fn choice_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let pending = self.session.read(cx).is_pending();
        let info = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug);
        let web_sign_in = info.as_ref().is_some_and(|info| info.web_sign_in);
        let buttons = info
            .map(|info| info.options)
            .unwrap_or_default()
            .into_iter()
            .filter(offered)
            .flat_map(|method| {
                self.method_buttons(slug, provider, method, web_sign_in, pending, cx)
            })
            .collect::<Vec<_>>();

        Modal::new(
            "login-sign-in-prompt",
            t!("login-choose-title", provider = provider),
        )
        .w(DIALOG)
        .close_button()
        .detail(t!("login-choose-detail", provider = provider))
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .gap_3()
                .w_full()
                .min_h(theme.metrics.control * CHOICES)
                .children(
                    buttons
                        .into_iter()
                        .map(|button| div().w_full().child(button)),
                ),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| {
            if this.choice_running {
                this.abandon(cx);
            }
            this.close_choice(cx);
        }))
    }

    /// The buttons that start one sign-in method. A cookie sign-in yields the browser window
    /// only where a backend can draw one, and always the manual paste beside it.
    fn method_buttons(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        web_sign_in: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut buttons = Vec::new();
        let manual = matches!(method, SignIn::Secret);
        if !manual || web_sign_in {
            let id = match &method {
                SignIn::Secret => format!("sign-in-{slug}-cookies"),
                SignIn::Credentials { .. } => format!("sign-in-{slug}-server"),
                _ => format!("sign-in-{slug}"),
            };
            buttons.push(
                Button::new(SharedString::from(id))
                    .label(t!("login-sign-in", provider = provider))
                    .primary()
                    .disabled(pending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choice_running = true;
                        match &method {
                            SignIn::Credentials { .. } => this.open_credentials(slug, cx),
                            method => this.start(slug, method.clone(), cx),
                        }
                    }))
                    .into_any_element(),
            );
        }
        if manual {
            buttons.push(
                Button::new(SharedString::from(format!("sign-in-{slug}-cookies-manual")))
                    .label(t!("login-connect-cookies"))
                    .secondary()
                    .disabled(pending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choice_running = true;
                        this.start_manual(slug, provider, cx);
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }

    /// The code a device sign-in is waiting on. Closing the dialog cancels the sign-in.
    fn code_prompt(&self, code: String, url: String, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("login-code-prompt", t!("login-device-code", url = &url))
            .w(DIALOG)
            .close_button()
            .detail(t!("login-authorizing"))
            .child(device_code(code, cx))
            .action(
                Button::new("login-cancel-code")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon(cx)))
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
                manual_secret.is_none() && !matches!(prompt, Some(SignInPrompt::Accounts(..)))
            }
            _ => false,
        };
        if self.choice_running && !pending {
            self.choosing = None;
            self.choice_running = false;
        }

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
        let code = match &prompt {
            Some(SignInPrompt::Code { code, url }) => Some((code.clone(), url.clone())),
            _ => None,
        };
        let url = match prompt {
            Some(SignInPrompt::Url(url)) => Some(url),
            _ => None,
        };

        // only one dialog is ever up: a prompt the sign-in raised hides the choice behind it
        let taken = accounts.is_some()
            || code.is_some()
            || manual_secret.is_some()
            || self.credentials_for.is_some();
        let choosing = self.choosing.filter(|_| !taken);
        let failure = providers.iter().find_map(|info| info.error.clone());

        let theme = *cx.theme();
        let asking = self.usage.read(cx).asking();
        let lamp = match self.hovered {
            Some(slug) => crate::shared::provider_brand(slug, cx),
            None => self.lamp,
        };
        let animates = ui::motion::animates(cx);
        match animates {
            true => self.pace(lamp, window, cx),
            false => {
                self.glow = f32::from(u8::from(self.hovered.is_some()));
                self.lamp = lamp;
            }
        }
        let time = match animates {
            true => self.started.elapsed().as_secs_f32(),
            false => 0.,
        };

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        let card = div()
            .flex()
            .flex_col()
            .gap_6()
            .w(CARD)
            .min_h_0()
            .max_h_full()
            .p_8()
            .rounded(ui::corners(theme.radius))
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .bg(theme.background.opacity(0.62))
            .backdrop_blur(FROST)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .child(self.welcome(status, cx))
                    .when_some(failure, |this, failure| {
                        this.child(crate::shared::trouble::trouble(failure, true, cx))
                    })
                    .when_some(url, |this, url| this.child(self.url_prompt(url))),
            )
            .child(self.rows(&providers, pending, waiting, cx))
            .when_some(guest, |this, slug| {
                this.child(div().flex_none().child(self.divider(cx)))
                    .child(div().flex_none().child(self.guest_mode(slug, pending, cx)))
            });

        let backdrop = Lattice::new(POOLS.map(|(h, s, l, a)| hsla(h, s, l, a)))
            .stage(Hsla {
                a: 1.,
                ..theme.background
            })
            .lamp(self.lamp)
            .glow(self.glow_at, self.glow)
            .time(time)
            .when_some(radius, |this, radius| this.corner_radius(radius));

        div()
            .id("login")
            .relative()
            .size_full()
            .overflow_hidden()
            .when_some(radius, |this, radius| this.rounded(radius))
            .child(div().absolute().inset_0().child(backdrop))
            // The card never runs past the window: it shrinks to fit, and its provider list scrolls.
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .size_full()
                    .p(theme.metrics.pad)
                    .pt(theme.metrics.title_bar)
                    .child(card)
                    .when(asking, |this| {
                        this.child(div().flex_none().child(self.consent(cx)))
                    }),
            )
            .when_some(manual_secret, |this, (slug, provider)| {
                this.child(self.secret_prompt(slug, provider, cx).into_any_element())
            })
            .when(self.credentials_for.is_some(), |this| {
                this.child(self.credentials_prompt(cx).into_any_element())
            })
            .when_some(accounts, |this, accounts| {
                this.child(self.account_modal(accounts, cx).into_any_element())
            })
            .when_some(code, |this, (code, url)| {
                this.child(self.code_prompt(code, url, cx).into_any_element())
            })
            .when_some(choosing, |this, (slug, provider)| {
                this.child(self.choice_prompt(slug, provider, cx).into_any_element())
            })
    }
}

/// Which sign-in methods a provider offers on this screen. Guest mode has a button of its own,
/// and a local path is not a way to sign in.
fn offered(method: &SignIn) -> bool {
    matches!(
        method,
        SignIn::Default | SignIn::Secret | SignIn::Credentials { .. }
    )
}

/// How many buttons one sign-in method draws: the method itself, and the manual paste beside
/// a cookie one. A provider that draws exactly one never opens the choice dialog.
fn method_count(method: &SignIn, web_sign_in: bool) -> usize {
    let manual = matches!(method, SignIn::Secret);
    usize::from(!manual || web_sign_in) + usize::from(manual)
}
