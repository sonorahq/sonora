use std::path::PathBuf;
use std::time::Duration;

use gpui::accesskit::{Role, Toggled};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, FontWeight, Hsla, Pixels, SharedString,
    Window, div, px, relative, svg,
};
use i18n::t;
use music::{AccountChoice, SignIn, SignInPrompt};
use router::{Destination, SettingsPage, navigate};
use state::{Failure, Scan, SessionState, Sonora};
use ui::{
    ActiveTheme as _, Avatar, Button, HoverFade, Initials, LEADING, Modal, Radio, Skeleton, Text,
    Theme, mix,
};

use super::rows::drill;
use super::{Group, SettingsView, Slot};
use crate::shared::device_code::device_code;
use crate::shared::local;
use crate::shared::popups::{AccountPicker, CookiePrompt};
use crate::shared::text;

/// The icon beside a provider's sign-in error.
const ERROR_ICON: Pixels = px(14.);
/// How large a provider's mark draws, and how long it takes to turn to its brand colour under
/// the pointer.
const MARK: Pixels = px(22.);
const MARK_FADE: Duration = Duration::from_millis(160);
/// How many buttons tall the sign-in choice stands, however few it lists.
const CHOICES: f32 = 3.;
/// What the guest row answers to, where a provider row answers to its slug.
const GUEST: &str = "guest";

/// What a row does when it is clicked.
type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) struct Account {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    stored: bool,
    active: bool,
    cancel: bool,
    error: Option<Failure>,
}

/// The guest entry the accounts block draws under the providers. It is a card of the app's
/// own, not a provider: it stands for the anonymous session `slug` offers, so signing out of
/// it signs that provider out.
#[derive(Clone, Copy)]
pub(super) struct Guest {
    slug: &'static str,
    stored: bool,
    active: bool,
}

/// Which sign-in methods a provider card lists. Anonymous never appears there, because guest
/// mode has a card of its own, and a stored account is asked for nothing.
fn offered(method: &SignIn, stored: bool) -> bool {
    match method {
        SignIn::Default | SignIn::Secret | SignIn::Credentials { .. } => !stored,
        SignIn::Anonymous | SignIn::Path(_) => false,
    }
}

impl SettingsView {
    pub(super) fn accounts_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-group-services", "icons/music.svg").cards(Slot::Accounts),
            Group::new("settings-group-local-files", "icons/file-music.svg")
                .rows([Slot::LocalFolder]),
        ]
    }

    pub(super) fn music_folders_groups(&self) -> Vec<Group> {
        vec![Group::untitled().rows([Slot::Folders])]
    }

    pub(super) fn profile(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        // The service the account belongs to, named next to the region so the card says which
        // provider it is rather than a bare country code.
        let provider = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.active)
            .map(|info| info.name.to_string());

        div()
            .flex()
            .items_center()
            .gap_4()
            .child(match self.session.read(cx).state() {
                SessionState::SignedIn(profile) => match &profile.avatar {
                    Some(avatar) => Avatar::new(Some(avatar.clone()))
                        .size(px(64.))
                        .into_any_element(),
                    None => Initials::new(profile.display_name.clone(), px(64.)).into_any_element(),
                },
                _ => Skeleton::new().size(px(64.)).circle().into_any_element(),
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => {
                            ui::spoken("settings-profile-name", profile.display_name.clone())
                                .text_size(theme.text(Text::Large))
                                .font_weight(FontWeight::SEMIBOLD)
                                .into_any_element()
                        }
                        _ => Skeleton::new().w(px(140.)).h(px(14.)).into_any_element(),
                    })
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => ui::spoken(
                            "settings-profile-account",
                            match &provider {
                                Some(provider) => t!(
                                    "settings-profile-account",
                                    provider = provider,
                                    account = &profile.id
                                ),
                                None => SharedString::from(profile.id.clone()),
                            },
                        )
                        .text_color(muted)
                        .text_size(theme.text(Text::Small))
                        .into_any_element(),
                        _ => Skeleton::new().w(px(90.)).h(px(10.)).into_any_element(),
                    }),
            )
    }

    /// The row that opens the music folders page, saying how many folders it watches.
    pub(super) fn local_folder_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let paths = self.session.read(cx).local_paths().len();
        let value = match paths {
            0 => t!("settings-local-folder-empty"),
            count => t!("settings-local-folder-count", count = count),
        };

        drill(
            "music-folders",
            t!("settings-local-folder"),
            Some(value),
            cx,
        )
        .on_click(|_, _, cx| navigate(Destination::Settings(SettingsPage::MusicFolders), cx))
        .into_any_element()
    }

    /// The music folders page's list: one row per watched folder with its remove button, then
    /// the row that adds a folder or scans them all again, with how the last scan went.
    pub(super) fn folders(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let paths = self.session.read(cx).local_paths();

        let add = local::choose_button("add-local-folder")
            .label(t!("settings-add-folder"))
            .icon("icons/folder-plus.svg")
            .small()
            .primary();

        let scan = Scan::global(cx).read(cx);
        let scanning = scan.progress();
        // Until the walk is over there is no total to be a fraction of, and on a network share
        // that is the longest part, so it says so rather than showing nothing.
        let note = match (scanning, scan.done()) {
            (Some(progress), _) => Some(match progress.percent() {
                Some(percent) => t!("settings-scan-progress", percent = percent),
                None => t!("settings-scan-walking"),
            }),
            (None, Some(took)) => Some(t!("settings-scan-done", seconds = text::lapsed(took))),
            (None, None) => None,
        };

        let rescan = (!paths.is_empty()).then(|| {
            Button::new("rescan-local-folder")
                .label(t!("settings-rescan"))
                .icon("icons/refresh-cw.svg")
                .small()
                .outline()
                .disabled(scanning.is_some())
                .on_click(cx.listener(|this, _, _, cx| this.rescan_local_folder(cx)))
        });

        let count = paths.len();
        let items = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| Self::local_folder_item((index, count), path, cx))
            .collect::<Vec<_>>();
        let list = (!items.is_empty()).then(|| {
            div()
                .id("local-folders")
                .role(Role::List)
                .aria_label(t!("settings-local-folder"))
                .flex()
                .flex_col()
                .children(items)
        });
        let empty = list.is_none().then(|| {
            ui::spoken("local-folders-empty", t!("settings-local-folder-none"))
                .flex()
                .items_center()
                .h(theme.metrics.list_row)
                .text_color(theme.muted_foreground)
        });

        div()
            .flex()
            .flex_col()
            .children(list)
            .children(empty)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(theme.metrics.list_row)
                    .child(add)
                    .children(rescan)
                    .child(div().flex_1())
                    .children(note.map(|note| {
                        ui::spoken("local-folders-scan", note)
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .text_size(theme.text(Text::Label))
                    })),
            )
            .into_any_element()
    }

    /// One watched folder, placed by its index and the list's length: its path and the
    /// button that stops watching it.
    fn local_folder_item(
        (index, count): (usize, usize),
        path: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();

        div()
            .id(SharedString::from(format!("local-folder-item-{path}")))
            .role(Role::ListItem)
            .aria_label(path.clone())
            .aria_position_in_set(index + 1)
            .aria_size_of_set(count)
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .h(theme.metrics.list_row)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(path.clone())),
            )
            .child(
                Button::new(SharedString::from(format!("remove-local-folder-{path}")))
                    .ghost()
                    .icon("icons/trash-2.svg")
                    .size_8()
                    .px_0()
                    .tooltip("settings-remove-folder")
                    .tint(theme.muted_foreground)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_local_folder(path.clone(), cx)
                    })),
            )
            .into_any_element()
    }

    pub(super) fn rescan_local_folder(&mut self, cx: &mut Context<Self>) {
        Scan::global(cx).update(cx, |scan, _| scan.asked());
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| library.rescan_local(true, cx));
    }

    pub(super) fn remove_local_folder(&mut self, path: String, cx: &mut Context<Self>) {
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| {
                library.remove_local_folder(PathBuf::from(path), cx)
            });
    }

    /// The provider accounts as plain data, shared by the row, its height and its search
    /// entry, so the three never disagree about what is shown.
    pub(super) fn providers(&self, cx: &App) -> Vec<Account> {
        let session = self.session.read(cx);
        let signed_out = matches!(session.state(), SessionState::SignedOut);
        let guest = !session.authenticated();
        let waiting = match session.state() {
            SessionState::Authorizing(prompt) => !matches!(prompt, Some(SignInPrompt::Accounts(_))),
            _ => false,
        };
        let loading = session.is_pending();
        let chosen = self.chosen.filter(|_| loading);
        session
            .providers()
            .map(|info| Account {
                slug: info.slug,
                name: info.name,
                options: info.options,
                web_sign_in: info.web_sign_in,
                stored: info.stored && !info.guest,
                // a session on its way up reports no account yet, so while it loads the card
                // it belongs to keeps the radio rather than leaving the list blank
                active: match chosen {
                    Some(picked) => picked == info.slug,
                    None if loading => info.active && !info.guest,
                    None => info.active && !signed_out && !guest,
                },
                cancel: waiting && info.pending,
                error: info.error,
            })
            .collect()
    }

    /// The guest card, when a provider offers an anonymous session at all. A guest run leaves
    /// that provider's own card connected to nothing, so only this card reports it.
    pub(super) fn guest(&self, cx: &App) -> Option<Guest> {
        let session = self.session.read(cx);
        let signed_out = matches!(session.state(), SessionState::SignedOut);
        let info = session.providers().find(|info| {
            info.options
                .iter()
                .any(|option| matches!(option, SignIn::Anonymous))
        })?;
        let loading = session.is_pending();
        let active = match self.chosen.filter(|_| loading) {
            Some(picked) => picked == GUEST,
            None if loading => info.active && info.guest,
            None => info.active && !signed_out && !session.authenticated(),
        };
        Some(Guest {
            slug: info.slug,
            stored: info.guest,
            active,
        })
    }

    /// The words the accounts row answers a search with: every provider name, and the guest
    /// entry when one is shown.
    pub(super) fn account_words(&self, cx: &App) -> String {
        let mut names: Vec<String> = self
            .providers(cx)
            .iter()
            .map(|account| account.name.to_string())
            .collect();
        if self.guest(cx).is_some() {
            names.push(t!("login-guest-title").to_string());
        }
        names.join(" ")
    }

    /// One row per provider, then the guest row when a provider offers guest mode, with a
    /// small gap between them so each row's fill stands on its own.
    pub(super) fn accounts_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.session.read(cx).is_pending();
        let providers = self.providers(cx);
        let guest = self.guest(cx);
        let count = providers.len() + usize::from(guest.is_some());
        let mut rows = Vec::new();
        for (place, account) in providers.into_iter().enumerate() {
            rows.push(self.account_row(account, pending, (place, count), cx));
        }
        if let Some(guest) = guest {
            rows.push(self.guest_row(guest, pending, (count - 1, count), cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(rows)
            .into_any_element()
    }

    /// A provider's row at `place`, its index and the length of the list. Clicking a provider
    /// that is signed in but not in use switches to it, clicking one signed out starts its
    /// sign-in, and one being signed in offers to stop.
    pub(super) fn account_row(
        &self,
        account: Account,
        pending: bool,
        place: (usize, usize),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Account {
            slug,
            name,
            options,
            web_sign_in,
            stored,
            active,
            cancel,
            error,
        } = account;
        let status = active.then(|| t!("settings-provider-current"));
        let offer = match (cancel, stored) {
            (true, _) => Offer::Cancel(self.cancel_button(slug, cx)),
            (false, true) => Offer::Choice {
                leave: Some(self.sign_out_button(slug, slug, pending, cx)),
            },
            (false, false) => Offer::SignIn,
        };
        let press: Option<Press> = match (cancel, stored, active) {
            (true, ..) | (false, true, true) => None,
            (false, true, false) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = Some(slug);
                this.session
                    .update(cx, |session, cx| session.switch(slug, cx));
            }))),
            (false, false, _) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = None;
                this.start_sign_in(slug, name, &options, web_sign_in, cx);
            }))),
        };

        ProviderRow {
            id: SharedString::from(format!("account-{slug}")),
            logo: crate::shared::provider_logo(slug),
            brand: crate::shared::provider_brand(slug, cx),
            active,
            name: name.into(),
            status,
            offer,
            press: press.filter(|_| !pending),
            error,
            place,
        }
        .into_any_element()
    }

    /// The guest row. It is always a choice, since guest mode needs no account, and its sign
    /// out button drives the provider the anonymous session belongs to, so it carries an id of
    /// its own rather than that provider's, which has a row too.
    pub(super) fn guest_row(
        &self,
        guest: Guest,
        pending: bool,
        place: (usize, usize),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Guest {
            slug,
            stored,
            active,
        } = guest;
        let status = active.then(|| t!("settings-provider-current"));
        let leave = active.then(|| self.sign_out_button(GUEST, slug, pending, cx));
        let press: Option<Press> = (!active).then(|| -> Press {
            Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = Some(GUEST);
                match stored {
                    true => this
                        .session
                        .update(cx, |session, cx| session.switch(slug, cx)),
                    false => this.session.update(cx, |session, cx| {
                        session.sign_in(slug, SignIn::Anonymous, cx)
                    }),
                }
            }))
        });

        ProviderRow {
            id: SharedString::from("account-guest"),
            logo: "icons/hat-glasses.svg",
            brand: cx.theme().foreground,
            active,
            name: t!("login-guest-title"),
            status,
            offer: Offer::Choice { leave },
            press: press.filter(|_| !pending),
            error: None,
            place,
        }
        .into_any_element()
    }

    /// The button that stops a sign-in in flight.
    fn cancel_button(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        Button::new(SharedString::from(format!("cancel-{id}")))
            .label(t!("common-cancel"))
            .small()
            .outline()
            .on_click(cx.listener(|this, _, _, cx| this.abandon(cx)))
            .into_any_element()
    }

    /// The button that signs `slug` out, carrying `id` so the guest row's never clashes with
    /// the provider's own.
    fn sign_out_button(
        &self,
        id: &'static str,
        slug: &'static str,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();

        Button::new(SharedString::from(format!("sign-out-{id}")))
            .ghost()
            .icon("icons/log-out.svg")
            .tooltip("settings-sign-out")
            .tint(theme.muted_foreground)
            .size(theme.metrics.control_small)
            .px_0()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.chosen = None;
                this.session
                    .update(cx, |session, cx| session.forget(slug, cx));
            }))
            .into_any_element()
    }

    /// Starts a sign-in the way the provider asks for it. One method goes straight through,
    /// a server asks for its address and credentials, and anything with a choice to make puts
    /// the choice up in a dialog.
    pub(super) fn start_sign_in(
        &mut self,
        slug: &'static str,
        provider: &'static str,
        options: &[SignIn],
        web_sign_in: bool,
        cx: &mut Context<Self>,
    ) {
        let methods: Vec<&SignIn> = options
            .iter()
            .filter(|option| offered(option, false))
            .collect();
        let buttons: usize = methods
            .iter()
            .map(|method| method_count(method, web_sign_in))
            .sum();
        match methods.as_slice() {
            [SignIn::Credentials { .. }] => self.open_credentials(slug, cx),
            [SignIn::Secret] if buttons == 1 => self.start_manual(slug, provider, cx),
            [method] if buttons == 1 => {
                let method = (*method).clone();
                self.session
                    .update(cx, |session, cx| session.sign_in(slug, method, cx));
            }
            [] => {}
            _ => {
                self.sign_in_for = Some((slug, provider));
                cx.notify();
            }
        }
    }

    /// Closes the dialog that is up, topmost first, the way clicking outside it does. A
    /// prompt a sign-in raised takes the sign-in down with it. Returns false when no dialog
    /// was up.
    pub(super) fn close_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.deleting_theme.is_some() {
            self.close_theme_delete(cx);
            return true;
        }
        if self.credentials_for.is_some() {
            self.abandon_credentials(cx);
            return true;
        }
        if self.scrobble_prompt.is_some() {
            self.close_scrobble(cx);
            return true;
        }
        let prompted = matches!(
            self.session.read(cx).state(),
            SessionState::Authorizing(Some(_))
        );
        if prompted {
            self.abandon(cx);
            return true;
        }
        if self.sign_in_for.is_some() {
            self.close_sign_in(cx);
            return true;
        }
        false
    }

    pub(super) fn close_sign_in(&mut self, cx: &mut Context<Self>) {
        self.sign_in_for = None;
        self.sign_in_running = false;
        cx.notify();
    }

    /// The choice of sign-in methods, for a provider that has more than one. The buttons are
    /// the same ones the card used to carry.
    pub(super) fn sign_in_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let pending = self.session.read(cx).is_pending();
        let options = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
            .map(|info| info.options)
            .unwrap_or_default();
        let web_sign_in = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
            .is_some_and(|info| info.web_sign_in);
        let buttons = options
            .into_iter()
            .filter(|option| offered(option, false))
            .flat_map(|method| {
                self.method_buttons(slug, provider, method, web_sign_in, pending, cx)
            });

        Modal::new(
            "settings-sign-in-prompt",
            t!("login-choose-title", provider = provider),
        )
        .w(px(420.))
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
                .children(buttons.map(|button| div().w_full().child(button))),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| this.close_sign_in(cx)))
    }

    pub(super) fn abandon(&mut self, cx: &mut Context<Self>) {
        self.clear_secret(cx);
        self.clear_credentials(cx);
        self.session
            .update(cx, |session, cx| session.cancel_sign_in(cx));
    }

    pub(super) fn open_credentials(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        self.credentials_for = Some(slug);
        cx.notify();
    }

    pub(super) fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credentials_for = None;
        self.server.update(cx, |input, cx| input.set_text("", cx));
        self.username.update(cx, |input, cx| input.set_text("", cx));
        self.password.update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn abandon_credentials(&mut self, cx: &mut Context<Self>) {
        self.clear_credentials(cx);
        cx.notify();
    }

    pub(super) fn submit_credentials(&mut self, cx: &mut Context<Self>) {
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
        self.session.update(cx, |session, cx| {
            session.sign_in(
                slug,
                SignIn::Credentials {
                    server,
                    username,
                    password,
                },
                cx,
            )
        });
    }

    pub(super) fn credentials_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("settings-server-prompt", t!("login-server-title"))
            .w(px(560.))
            .detail(t!("login-server-detail"))
            .child(self.server.clone())
            .child(self.username.clone())
            .child(self.password.clone())
            .action(
                Button::new("settings-cancel-server")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon_credentials(cx))),
            )
            .action(
                Button::new("settings-submit-server")
                    .label(t!("login-server-submit"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.submit_credentials(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon_credentials(cx)))
    }

    pub(super) fn start_manual(
        &mut self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.manual_secret = Some((slug, provider));
        let hint = CookiePrompt::hint(slug);
        self.secret.update(cx, |input, cx| input.set_hint(hint, cx));
        self.session
            .update(cx, |session, cx| session.sign_in_with_cookies(slug, cx));
    }

    pub(super) fn clear_secret(&mut self, cx: &mut Context<Self>) {
        self.manual_secret = None;
        self.secret.update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn submit_secret(&mut self, cx: &mut Context<Self>) {
        let text = self.secret.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.clear_secret(cx);
        self.session
            .update(cx, |session, cx| session.submit_input(text, cx));
    }

    /// The code a device sign-in is waiting on. Closing the dialog cancels the sign-in.
    pub(super) fn code_prompt(
        &self,
        code: String,
        url: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Modal::new("settings-code-prompt", t!("login-device-code", url = &url))
            .w(px(420.))
            .close_button()
            .detail(t!("login-authorizing"))
            .child(device_code(code, cx))
            .action(
                Button::new("settings-cancel-code")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    pub(super) fn secret_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        CookiePrompt::new(slug, provider, self.secret.clone())
            .on_submit(cx.listener(|this, _, _, cx| this.submit_secret(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    /// The buttons that start one sign-in method. A cookie sign-in yields the browser window
    /// only where a backend can draw one, and always the manual paste beside it.
    pub(super) fn method_buttons(
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
            buttons.push(
                self.method_button(slug, provider, method, pending, cx)
                    .into_any_element(),
            );
        }
        if manual {
            buttons.push(
                Button::new(SharedString::from(format!("connect-{slug}-cookies-manual")))
                    .label(t!("login-connect-cookies"))
                    .secondary()
                    .disabled(pending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sign_in_running = true;
                        this.start_manual(slug, provider, cx);
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }

    pub(super) fn method_button(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (id, label) = match &method {
            SignIn::Default => (
                format!("connect-{slug}"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Anonymous => (format!("connect-{slug}-guest"), t!("login-guest-use")),
            SignIn::Secret => (
                format!("connect-{slug}-cookies"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Path(_) => (
                format!("connect-{slug}-path"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Credentials { .. } => (
                format!("connect-{slug}-server"),
                t!("login-sign-in", provider = provider),
            ),
        };

        Button::new(SharedString::from(id))
            .label(label)
            .primary()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sign_in_running = true;
                match &method {
                    SignIn::Credentials { .. } => this.open_credentials(slug, cx),
                    method => {
                        let method = method.clone();
                        this.session
                            .update(cx, |session, cx| session.sign_in(slug, method, cx));
                    }
                }
            }))
    }

    pub(super) fn account_modal(
        &self,
        accounts: Vec<AccountChoice>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        AccountPicker::new(accounts)
            .on_pick(cx.listener(|this, id: &SharedString, _, cx| {
                let id = id.to_string();
                this.session
                    .update(cx, |session, cx| session.submit_input(id, cx));
            }))
            .on_cancel(cx.listener(|this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.cancel_sign_in(cx));
            }))
    }
}

/// How tall one line of `step` text stands at the leading every label is set in.
fn line(theme: &Theme, step: Text) -> Pixels {
    px((theme.text(step) / px(1.) * LEADING).round())
}

/// What a provider row ends with.
pub(super) enum Offer {
    /// The radio saying whether the row is the one in use, with the sign out button before it
    /// when there is an account to leave.
    Choice { leave: Option<AnyElement> },
    /// A quiet "Sign in" for a provider with no account yet. The row's own click answers it.
    SignIn,
    /// The button that stops a sign-in in flight.
    Cancel(AnyElement),
}

/// One provider as a soft borderless row: its mark, the name over a muted status, and the
/// `offer` at the end. Only the provider in use (`active`) sits on a fill and shows its `brand`
/// colour. The others are muted and warm to their
/// colour and a lighter fill under the pointer. The sign out button keeps a slot of its own on
/// every row, shown on hover and always on the active row, so nothing moves as it comes and
/// goes. A sign-in error wraps under the name on a line of its own. Screen readers hear the
/// row as one node named after the provider, a radio button for an account to switch to and a
/// button for one to sign in to, with the status, hint and error as its description.
#[derive(IntoElement)]
pub(super) struct ProviderRow {
    id: SharedString,
    logo: &'static str,
    brand: Hsla,
    active: bool,
    name: SharedString,
    status: Option<SharedString>,
    offer: Offer,
    press: Option<Press>,
    error: Option<Failure>,
    /// The row's index in the list and the list's length.
    place: (usize, usize),
}

impl RenderOnce for ProviderRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ProviderRow {
            id,
            logo,
            brand,
            active,
            name,
            status,
            offer,
            press,
            error,
            place: (position, count),
        } = self;
        let theme = *cx.theme();
        let role = match offer {
            Offer::Choice { .. } => Role::RadioButton,
            Offer::SignIn => Role::Button,
            Offer::Cancel(_) => Role::Group,
        };
        let hint = matches!(offer, Offer::SignIn).then(|| t!("settings-provider-sign-in"));
        let description = status
            .iter()
            .chain(hint.iter())
            .cloned()
            .chain(error.as_ref().map(crate::shared::trouble::short))
            .map(|part| part.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let fade = window.use_keyed_state((ElementId::from(id.clone()), "mark"), cx, |_, _| {
            HoverFade::new(MARK_FADE)
        });
        let hover = fade.update(cx, |fade, cx| fade.level(window, cx));
        let mark = match active {
            true => brand,
            false => mix(theme.muted_foreground, brand, hover),
        };
        let fill = match (active, press.is_some()) {
            (true, _) => theme.secondary_active,
            (false, true) => theme.secondary_hover.opacity(hover),
            (false, false) => theme.secondary_hover.opacity(0.),
        };
        let trailing = match offer {
            Offer::Choice { leave } => {
                let shown = active || hover > 0.;
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .w(theme.metrics.control_small)
                            .opacity(match active {
                                true => 1.,
                                false => hover,
                            })
                            .children(leave.filter(|_| shown)),
                    )
                    .child(Radio::new(SharedString::from(format!("{id}-radio")), active).silent())
                    .into_any_element()
            }
            Offer::SignIn => div()
                .text_size(theme.text(Text::Label))
                .text_color(mix(theme.muted_foreground, theme.foreground, hover))
                .children(hint)
                .into_any_element(),
            Offer::Cancel(cancel) => cancel,
        };

        div()
            .id(id)
            .role(role)
            .aria_label(name.clone())
            .when(role == Role::RadioButton, |this| {
                this.aria_toggled(Toggled::from(active))
            })
            .when(!description.is_empty(), |this| {
                this.aria_description(description)
            })
            .aria_position_in_set(position + 1)
            .aria_size_of_set(count)
            .flex()
            .flex_col()
            .mx_neg_3()
            .px_3()
            .rounded(theme.radius)
            .bg(fill)
            .on_hover(move |hovered, window, cx| {
                fade.update(cx, |fade, _| fade.set(*hovered));
                window.refresh();
            })
            .when_some(press, |this, press| {
                this.cursor_pointer()
                    .on_click(move |event, window, cx| press(event, window, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(theme.metrics.list_row)
                    .child(
                        div().relative().flex_none().size(MARK).child(
                            svg()
                                .path(icons::path(logo))
                                .absolute()
                                .inset_0()
                                .size(MARK)
                                .text_color(mark),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .line_height(relative(LEADING))
                                    .text_size(theme.text(Text::Body))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name),
                            )
                            .children(status.map(|status| {
                                div()
                                    .truncate()
                                    .line_height(relative(LEADING))
                                    .text_size(theme.text(Text::Small))
                                    .text_color(theme.muted_foreground)
                                    .child(status)
                            })),
                    )
                    .child(div().flex().flex_none().items_center().child(trailing)),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .flex()
                        .gap_3()
                        .pb_3()
                        .child(div().flex_none().w(MARK))
                        .child(div().flex_1().min_w_0().child(account_error(&error, cx))),
                )
            })
    }
}

/// How many buttons one sign-in method draws: the method itself, and the manual paste beside
/// a cookie one. A provider that draws exactly one never opens the choice dialog.
pub(super) fn method_count(method: &SignIn, web_sign_in: bool) -> usize {
    let manual = matches!(method, SignIn::Secret);
    usize::from(!manual || web_sign_in) + usize::from(manual)
}

/// A provider's sign-in error in one line, for the fixed-height card. The full notice does
/// not fit a measured row.
pub(super) fn account_error(error: &Failure, cx: &App) -> impl IntoElement {
    let theme = *cx.theme();
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(line(&theme, Text::Small).max(ERROR_ICON))
        .child(
            svg()
                .path(icons::path("icons/circle-alert.svg"))
                .size(ERROR_ICON)
                .flex_none()
                .text_color(theme.danger),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .line_height(relative(LEADING))
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(crate::shared::trouble::short(error)),
        )
}
