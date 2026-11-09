//! The Providers page: one row per streaming service to sign in to, switch to or sign out of,
//! the guest row, and the way into the music folders page; that page's own list of watched
//! folders; and every dialog a sign-in raises on the way.

use std::path::PathBuf;

use i18n::t;
use music::{AccountChoice, SignIn, SignInPrompt};
use router::{Destination, SettingsPage};
use rune::{Cursor, Cx, El, GrowthSpec, TextEdit, Transition, Tx, col, el, icon, row, text};
use state::{Failure, SessionState};
use ui::{Button, Initials, Modal, Radio, Skeleton, Text, Theme, Weight, font};

use super::{Layout, drill, group, whole};
use crate::shared::device_code::device_code;
use crate::shared::popups::{CookiePrompt, account_picker, cookie_hint};
use crate::shared::trouble::short;
use crate::{Ui, act, act_tx};

/// The icon beside a provider's sign-in error.
const ERROR_ICON: f32 = 14.;
/// How large a provider's mark draws.
const MARK: f32 = 22.;
/// How large the signed-in profile's picture stands, and the skeleton lines while it loads.
const PROFILE: f32 = 64.;
const NAME_SKELETON: [f32; 2] = [140., 14.];
const ACCOUNT_SKELETON: [f32; 2] = [90., 10.];
/// How many buttons tall the sign-in choice stands, however few it lists.
const CHOICES: f32 = 3.;
/// How wide the sign-in dialogs stand.
const CHOICE_WIDTH: f32 = 420.;
const SERVER_WIDTH: f32 = 560.;
/// How far a provider row reaches past the column on either side, so its mark lines up with
/// the headings while its fill stands clear of them.
const REACH: f32 = 12.;
/// What the guest row answers to, where a provider row answers to its slug.
const GUEST: &str = "guest";

/// What the Providers page keeps between frames: the dialogs a sign-in puts up and the fields
/// they hold.
#[derive(Default)]
pub struct State {
    /// The card the user just switched to, by slug or `guest`. Its radio fills while the
    /// session tears the old provider down and brings the new one up, which reports nothing
    /// active in between.
    chosen: Option<&'static str>,
    /// The provider whose sign-in choice is up, by slug and name.
    sign_in_for: Option<(&'static str, &'static str)>,
    /// Whether the choice the dialog offered has been taken. The dialog stays up until the
    /// sign-in it started is over, so the veil never blinks away between it and the prompt
    /// that follows.
    sign_in_running: bool,
    /// The server sign-in dialog, by provider slug, and its fields.
    credentials_for: Option<&'static str>,
    pub server: TextEdit,
    pub username: TextEdit,
    pub password: TextEdit,
    /// The manual cookie paste, by provider slug and name, and its field.
    manual_secret: Option<(&'static str, &'static str)>,
    pub secret: TextEdit,
}

/// A provider as the row draws it.
struct Account {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    stored: bool,
    active: bool,
    cancel: bool,
    error: Option<Failure>,
}

/// The guest entry the accounts block draws under the providers. It stands for the anonymous
/// session `slug` offers, so signing out of it signs that provider out.
#[derive(Clone, Copy)]
struct Guest {
    slug: &'static str,
    stored: bool,
    active: bool,
}

/// What a provider row ends with.
enum Offer {
    /// The radio saying whether the row is the one in use, with the sign out button before it
    /// when there is an account to leave.
    Choice { leave: Option<El> },
    /// A quiet "Sign in" for a provider with no account yet. The row's own press answers it.
    SignIn,
    /// The button that stops a sign-in in flight.
    Cancel(El),
}

/// What a row does when it is pressed.
type Press = Box<dyn Fn(&mut Ui)>;

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.settings.accounts
}

/// The Providers page: the profile signed in, the services, and the row that opens the music
/// folders. The sign-in dialogs go up in `layer` while a sign-in asks for them.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, _layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    dialogs(ui, layer);
    vec![
        profile(ui),
        group(
            theme,
            Some(("settings-group-services", "icons/music.svg")),
            Some(accounts_rows(ui)),
            [],
        ),
        group(
            theme,
            Some(("settings-group-local-files", "icons/file-music.svg")),
            None,
            [local_folder_row(ui)],
        ),
    ]
}

/// The music folders page: one row per watched folder with its remove button, then the row
/// that adds a folder or scans them all again, with how the last scan went.
pub(super) fn folders(
    ui: &Ui,
    _cx: &mut Cx<Ui>,
    _layer: &mut Vec<El>,
    _layout: &Layout,
) -> Vec<El> {
    let theme = &ui.theme;
    let paths = ui.store.session.local_paths();
    let scan = &ui.store.scan;
    let scanning = scan.progress();
    // Until the walk is over there is no total to be a fraction of, and on a network share
    // that is the longest part, so it says so rather than showing nothing.
    let note = match (&scanning, scan.done()) {
        (Some(progress), _) => Some(match progress.percent() {
            Some(percent) => t!("settings-scan-progress", percent = percent),
            None => t!("settings-scan-walking"),
        }),
        (None, Some(took)) => Some(t!(
            "settings-scan-done",
            seconds = crate::shared::text::lapsed(took)
        )),
        (None, None) => None,
    };

    let add = Button::new("add-local-folder")
        .label(t!("settings-add-folder"))
        .icon("icons/folder-plus.svg")
        .small()
        .primary()
        .on_press_tx(|_: &mut Ui, tx: &mut Tx| crate::shared::local::choose_folder(tx))
        .build(theme);
    let rescan = (!paths.is_empty()).then(|| {
        Button::new("rescan-local-folder")
            .label(t!("settings-rescan"))
            .icon("icons/refresh-cw.svg")
            .small()
            .outline()
            .spinning(scanning.is_some())
            .disabled(scanning.is_some())
            .on_press(act(|ui: &mut Ui| {
                ui.store.scan.asked();
                ui.store.rescan_local(true);
            }))
            .build(theme)
    });

    let count = paths.len();
    let list = match count {
        0 => ui::spoken(
            "local-folders-empty",
            t!("settings-local-folder-none"),
            theme,
        )
        .color(theme.muted_foreground.color())
        .h(theme.metrics.list_row)
        .transition(Transition::opacity()),
        _ => col()
            .key("local-folders")
            .role("list")
            .label(t!("settings-local-folder"))
            .w_full()
            .growth(GrowthSpec {
                spring: rune::GROWTH,
                edge: rune::edge::NO_MASK,
            })
            .children(
                paths
                    .into_iter()
                    .enumerate()
                    .map(|(index, path)| folder_item(theme, index, count, path)),
            ),
    };
    let mut actions = row()
        .key("local-folder-actions")
        .w_full()
        .center_y()
        .gap(theme.font_size * 0.5)
        .h(theme.metrics.list_row)
        .child(add)
        .children(rescan)
        .child(el().grow(1.));
    if let Some(note) = note {
        actions = actions.child(
            ui::spoken("local-folders-scan", note, theme)
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .transition(Transition::opacity()),
        );
    }

    vec![group(
        theme,
        None,
        None,
        [col()
            .key("settings-row-folders")
            .w_full()
            .child(list)
            .child(actions)],
    )]
}

/// One watched folder, placed by its index and the list's length: its path and the button
/// that stops watching it. It slides in as it is added and fades out as it goes.
fn folder_item(theme: &Theme, index: usize, count: usize, path: String) -> El {
    let removed = PathBuf::from(&path);
    row()
        .key(format!("local-folder-item-{path}"))
        .role("listitem")
        .label(path.clone())
        .position_in_set(index + 1, count)
        .w_full()
        .center_y()
        .justify_between()
        .gap(theme.font_size * 0.75)
        .h(theme.metrics.list_row)
        .transition(Transition::offset(0., 8.).combined(Transition::opacity()))
        .child(
            text(path.clone())
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.color())
                .truncate_middle()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        )
        .child(
            Button::new(format!("remove-local-folder-{path}"))
                .ghost()
                .icon("icons/trash-2.svg")
                .small()
                .tooltip(t!("settings-remove-folder"))
                .tint(theme.muted_foreground)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.remove_local_folder(&removed)
                }))
                .build(theme)
                .w(theme.metrics.control)
                .px(0.)
                .justify_center(),
        )
}

/// The signed-in profile above the services: the picture, the name, and the provider and
/// account it belongs to, as skeletons while the session is on its way up.
fn profile(ui: &Ui) -> El {
    let theme = &ui.theme;
    let session = &ui.store.session;
    // The service the account belongs to, named next to the account so the card says which
    // provider it is rather than a bare id.
    let provider = session
        .providers()
        .find(|info| info.active)
        .map(|info| info.name.to_owned());
    let (picture, name, account) = match session.state() {
        SessionState::SignedIn(profile) => {
            let picture = match &profile.avatar {
                Some(url) => ui::Avatar::new(ui.artwork(), Some(url.as_str()))
                    .size(PROFILE)
                    .build(theme),
                None => Initials::new(profile.display_name.clone(), PROFILE).build(theme),
            };
            let name = ui::spoken("settings-profile-name", profile.display_name.clone(), theme)
                .font(font(theme, Text::Large, Weight::SEMIBOLD))
                .truncate();
            let line = match &provider {
                Some(provider) => t!(
                    "settings-profile-account",
                    provider = provider,
                    account = &profile.id
                )
                .into_owned(),
                None => profile.id.clone(),
            };
            let account = ui::spoken("settings-profile-account", line, theme)
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate();
            (picture, name, account)
        }
        _ => (
            Skeleton::new().circle().build(theme).size(PROFILE, PROFILE),
            Skeleton::new()
                .build(theme)
                .size(NAME_SKELETON[0], NAME_SKELETON[1]),
            Skeleton::new()
                .build(theme)
                .size(ACCOUNT_SKELETON[0], ACCOUNT_SKELETON[1]),
        ),
    };
    row()
        .key("settings-profile")
        .center_y()
        .gap(theme.font_size)
        .child(picture.transition(Transition::opacity()))
        .child(
            col()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .gap(theme.font_size * 0.25)
                .child(name.transition(Transition::opacity()))
                .child(account.transition(Transition::opacity())),
        )
}

/// The row that opens the music folders page, saying how many folders it watches.
fn local_folder_row(ui: &Ui) -> El {
    let paths = ui.store.session.local_paths().len();
    let value = match paths {
        0 => t!("settings-local-folder-empty"),
        count => t!("settings-local-folder-count", count = count),
    };
    whole(
        drill(
            "local-folder",
            t!("settings-local-folder"),
            Some(value.into_owned()),
        )
        .on_press(act(|ui: &mut Ui| {
            ui.go(Destination::Settings(SettingsPage::MusicFolders))
        })),
        &ui.theme,
    )
}

/// Which sign-in methods a provider card lists. Anonymous never appears there, because guest
/// mode has a card of its own, and a stored account is asked for nothing.
fn offered(method: &SignIn, stored: bool) -> bool {
    match method {
        SignIn::Default | SignIn::Secret | SignIn::Credentials { .. } => !stored,
        SignIn::Anonymous | SignIn::Path(_) => false,
    }
}

/// How many buttons one sign-in method draws: the method itself, and the manual paste beside
/// a cookie one. A provider that draws exactly one never opens the choice dialog.
fn method_count(method: &SignIn, web_sign_in: bool) -> usize {
    let manual = matches!(method, SignIn::Secret);
    usize::from(!manual || web_sign_in) + usize::from(manual)
}

/// The provider accounts as plain data, shared by the rows and the search words.
fn providers(ui: &Ui) -> Vec<Account> {
    let session = &ui.store.session;
    let signed_out = matches!(session.state(), SessionState::SignedOut);
    let guest = !session.authenticated();
    let waiting = match session.state() {
        SessionState::Authorizing(prompt) => !matches!(prompt, Some(SignInPrompt::Accounts(_))),
        _ => false,
    };
    let loading = session.is_pending();
    let chosen = ui.screens.settings.accounts.chosen.filter(|_| loading);
    session
        .providers()
        .map(|info| Account {
            slug: info.slug,
            name: info.name,
            options: info.options,
            web_sign_in: info.web_sign_in,
            stored: info.stored && !info.guest,
            // a session on its way up reports no account yet, so while it loads the card it
            // belongs to keeps the radio rather than leaving the list blank
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

/// The guest card, when a provider offers an anonymous session at all. A guest run leaves that
/// provider's own card connected to nothing, so only this card reports it.
fn guest(ui: &Ui) -> Option<Guest> {
    let session = &ui.store.session;
    let signed_out = matches!(session.state(), SessionState::SignedOut);
    let info = session.providers().find(|info| {
        info.options
            .iter()
            .any(|option| matches!(option, SignIn::Anonymous))
    })?;
    let loading = session.is_pending();
    let active = match ui.screens.settings.accounts.chosen.filter(|_| loading) {
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

/// The words the accounts row answers a search with: every provider name, and the guest entry
/// when one is shown.
pub(super) fn account_words(ui: &Ui) -> String {
    let mut names: Vec<String> = providers(ui)
        .iter()
        .map(|account| account.name.to_owned())
        .collect();
    if guest(ui).is_some() {
        names.push(t!("login-guest-title").into_owned());
    }
    names.join(" ")
}

/// One row per provider, then the guest row when a provider offers guest mode, with a small
/// gap between them so each row's fill stands on its own.
fn accounts_rows(ui: &Ui) -> El {
    let theme = &ui.theme;
    let pending = ui.store.session.is_pending();
    let providers = providers(ui);
    let guest = guest(ui);
    let count = providers.len() + usize::from(guest.is_some());
    let mut rows: Vec<El> = providers
        .into_iter()
        .enumerate()
        .map(|(place, account)| account_row(ui, account, pending, (place, count)))
        .collect();
    if let Some(guest) = guest {
        rows.push(guest_row(ui, guest, pending, (count - 1, count)));
    }
    col()
        .key("settings-row-accounts")
        .role("radiogroup")
        .label(t!("settings-accounts"))
        .w_full()
        .gap(theme.font_size * 0.25)
        .children(rows)
}

/// A provider's row at `place`. Pressing a provider that is signed in but not in use switches
/// to it, pressing one signed out starts its sign-in, and one being signed in offers to stop.
fn account_row(ui: &Ui, account: Account, pending: bool, place: (usize, usize)) -> El {
    let theme = &ui.theme;
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
    let status = active.then(|| t!("settings-provider-current").into_owned());
    let offer = match (cancel, stored) {
        (true, _) => Offer::Cancel(cancel_button(theme, slug)),
        (false, true) => Offer::Choice {
            leave: Some(sign_out_button(theme, slug, slug, pending)),
        },
        (false, false) => Offer::SignIn,
    };
    let press: Option<Press> = match (cancel, stored, active) {
        (true, ..) | (false, true, true) => None,
        (false, true, false) => Some(Box::new(move |ui: &mut Ui| {
            st(ui).chosen = Some(slug);
            ui.store.switch_provider(slug);
        })),
        (false, false, _) => Some(Box::new(move |ui: &mut Ui| {
            st(ui).chosen = None;
            start_sign_in(ui, slug, name, &options, web_sign_in);
        })),
    };
    provider_row(
        theme,
        ProviderRow {
            key: format!("account-{slug}"),
            logo: provider_logo(slug),
            brand: provider_brand(slug, theme),
            active,
            name: name.to_owned(),
            status,
            offer,
            press: press.filter(|_| !pending),
            error,
            place,
        },
    )
}

/// The guest row. It is always a choice, since guest mode needs no account, and its sign out
/// button drives the provider the anonymous session belongs to, so it carries a key of its own.
fn guest_row(ui: &Ui, guest: Guest, pending: bool, place: (usize, usize)) -> El {
    let theme = &ui.theme;
    let Guest {
        slug,
        stored,
        active,
    } = guest;
    let status = active.then(|| t!("settings-provider-current").into_owned());
    let leave = active.then(|| sign_out_button(theme, GUEST, slug, pending));
    let press: Option<Press> = (!active).then(|| -> Press {
        Box::new(move |ui: &mut Ui| {
            st(ui).chosen = Some(GUEST);
            match stored {
                true => ui.store.switch_provider(slug),
                false => ui
                    .store
                    .session
                    .sign_in(slug, SignIn::Anonymous, &mut ui.store.cx),
            }
        })
    });
    provider_row(
        theme,
        ProviderRow {
            key: "account-guest".to_owned(),
            logo: "icons/hat-glasses.svg",
            brand: theme.foreground,
            active,
            name: t!("login-guest-title").into_owned(),
            status,
            offer: Offer::Choice { leave },
            press: press.filter(|_| !pending),
            error: None,
            place,
        },
    )
}

/// The button that stops a sign-in in flight.
fn cancel_button(theme: &Theme, slug: &str) -> El {
    Button::new(format!("cancel-{slug}"))
        .label(t!("common-cancel"))
        .small()
        .outline()
        .on_press(act(abandon))
        .build(theme)
}

/// The button that signs `slug` out, keyed by `id` so the guest row's never clashes with the
/// provider's own.
fn sign_out_button(theme: &Theme, id: &str, slug: &'static str, pending: bool) -> El {
    Button::new(format!("sign-out-{id}"))
        .ghost()
        .small()
        .icon("icons/log-out.svg")
        .tooltip(t!("settings-sign-out"))
        .tint(theme.muted_foreground)
        .disabled(pending)
        .on_press(act(move |ui: &mut Ui| {
            st(ui).chosen = None;
            ui.store.forget_provider(slug);
        }))
        .build(theme)
        .w(theme.metrics.control_small)
        .px(0.)
        .justify_center()
}

/// One provider as a soft borderless row: its mark, the name over a muted status, and the
/// `offer` at the end.
struct ProviderRow {
    key: String,
    logo: &'static str,
    brand: ui::Hsla,
    active: bool,
    name: String,
    status: Option<String>,
    offer: Offer,
    press: Option<Press>,
    error: Option<Failure>,
    /// The row's index in the list and the list's length.
    place: (usize, usize),
}

/// Draws a provider row. Only the provider in use sits on a fill and shows its brand colour;
/// the others are muted and warm to their colour and a lighter fill under the pointer, through
/// the row's hover group, so hovering never runs the view. The sign out button keeps a slot of
/// its own on every row, shown on hover and always on the active row, so nothing moves as it
/// comes and goes. A sign-in error wraps under the name on a line of its own. Screen readers
/// hear the row as one node named after the provider, a radio button for an account to switch
/// to and a button for one to sign in to, with the status, hint and error as its description.
fn provider_row(theme: &Theme, row_: ProviderRow) -> El {
    let ProviderRow {
        key,
        logo,
        brand,
        active,
        name,
        status,
        offer,
        press,
        error,
        place: (position, count),
    } = row_;
    let role = match offer {
        Offer::Choice { .. } => "radio",
        Offer::SignIn => "button",
        Offer::Cancel(_) => "group",
    };
    let hint = matches!(offer, Offer::SignIn).then(|| t!("settings-provider-sign-in").into_owned());
    let description = status
        .iter()
        .chain(hint.iter())
        .cloned()
        .chain(error.as_ref().map(short))
        .collect::<Vec<_>>()
        .join(", ");
    let mark = match active {
        true => icon(icons::get(logo), MARK)
            .icon_one_color()
            .color(brand.color()),
        false => icon(icons::get(logo), MARK)
            .icon_one_color()
            .color(theme.muted_foreground.color())
            .group_color(brand.color()),
    }
    .a11y_hidden();
    let trailing = match offer {
        Offer::Choice { leave } => {
            let mut slot = row().key("leave").w(theme.metrics.control_small).center();
            if !active {
                slot = slot.group_opacity(0., 1.);
            }
            row()
                .center_y()
                .gap(theme.font_size * 0.5)
                .child(slot.children(leave))
                .child(
                    Radio::new(format!("{key}-radio"), active)
                        .silent()
                        .build(theme),
                )
        }
        Offer::SignIn => row().child(
            text(hint.clone().unwrap_or_default())
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .group_color(theme.foreground.color())
                .a11y_hidden(),
        ),
        Offer::Cancel(cancel) => cancel,
    };
    let mut words = col().grow(1.).shrink().min_w(0.).child(
        text(name.clone())
            .font(font(theme, Text::Body, Weight::MEDIUM))
            .color(theme.foreground.color())
            .truncate()
            .a11y_hidden(),
    );
    if let Some(status) = status {
        words = words.child(
            text(status)
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate()
                .transition(Transition::opacity())
                .a11y_hidden(),
        );
    }

    let mut entry = col()
        .key(key)
        .role(role)
        .label(name)
        .position_in_set(position + 1, count)
        .w_full()
        .mx(-REACH)
        .px(REACH)
        .radius(theme.radius)
        .hover_group()
        .motion(ui::Springs::CONTROL)
        .fill(match active {
            true => theme.secondary_active.color(),
            false => theme.secondary_hover.opacity(0.).color(),
        })
        .child(
            row()
                .center_y()
                .gap(theme.font_size * 0.75)
                .h(theme.metrics.list_row)
                .child(row().size(MARK, MARK).center().child(mark))
                .child(words)
                .child(row().center_y().child(trailing)),
        );
    if role == "radio" {
        entry = entry.checked(active);
    }
    if !description.is_empty() {
        entry = entry.description(description);
    }
    if let Some(press) = press {
        entry = entry
            .focusable(true)
            .cursor(Cursor::Pointer)
            .hover(theme.secondary_hover.color())
            .press_scale(ui::PRESS_SCALE_CARD)
            .on_press(act(move |ui: &mut Ui| press(ui)));
    }
    if let Some(error) = error {
        entry = entry.child(
            row()
                .key("error")
                .gap(theme.font_size * 0.75)
                .pb(theme.font_size * 0.75)
                .transition(Transition::offset(0., -4.).combined(Transition::opacity()))
                .child(el().w(MARK))
                .child(
                    col()
                        .grow(1.)
                        .shrink()
                        .min_w(0.)
                        .child(match error.problem {
                            Some(music::SignInProblem::Subscription) => {
                                crate::shared::trouble::trouble(error, false, theme)
                            }
                            _ => account_error(theme, &error),
                        }),
                ),
        );
    }
    entry
}

/// A provider's sign-in error in one line: the alert mark and the reason.
fn account_error(theme: &Theme, error: &Failure) -> El {
    let reason = short(error);
    row()
        .key("account-error")
        .role("text")
        .label(reason.clone())
        .center_y()
        .gap(theme.font_size * 0.5)
        .child(
            icon(icons::get("icons/circle-alert.svg"), ERROR_ICON)
                .color(theme.danger.color())
                .a11y_hidden(),
        )
        .child(
            text(reason)
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate()
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        )
}

/// Starts a sign-in the way the provider asks for it. One method goes straight through, a
/// server asks for its address and credentials, and anything with a choice to make puts the
/// choice up in a dialog.
fn start_sign_in(
    ui: &mut Ui,
    slug: &'static str,
    provider: &'static str,
    options: &[SignIn],
    web_sign_in: bool,
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
        [SignIn::Credentials { .. }] => open_credentials(ui, slug),
        [SignIn::Secret] if buttons == 1 => start_manual(ui, slug, provider),
        [method] if buttons == 1 => {
            let method = (*method).clone();
            ui.store.session.sign_in(slug, method, &mut ui.store.cx);
        }
        [] => {}
        _ => {
            let state = st(ui);
            state.sign_in_for = Some((slug, provider));
            state.sign_in_running = false;
        }
    }
}

fn close_sign_in(ui: &mut Ui) {
    let state = st(ui);
    state.sign_in_for = None;
    state.sign_in_running = false;
}

/// Stops the sign-in under way, and empties whatever its dialogs held.
fn abandon(ui: &mut Ui) {
    clear_secret(ui);
    clear_credentials(ui);
    ui.store.session.cancel_sign_in(&mut ui.store.cx);
}

fn open_credentials(ui: &mut Ui, slug: &'static str) {
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
    ui.store.session.sign_in(
        slug,
        SignIn::Credentials {
            server,
            username,
            password,
        },
        &mut ui.store.cx,
    );
}

fn start_manual(ui: &mut Ui, slug: &'static str, provider: &'static str) {
    let state = st(ui);
    state.manual_secret = Some((slug, provider));
    state.secret.focused = true;
    ui.store
        .session
        .sign_in_with_cookies(slug, &mut ui.store.cx);
}

fn clear_secret(ui: &mut Ui) {
    let state = st(ui);
    state.manual_secret = None;
    state.secret.clear();
}

fn submit_secret(ui: &mut Ui) {
    let text = st(ui).secret.text.clone();
    if text.trim().is_empty() {
        return;
    }
    clear_secret(ui);
    ui.store.session.submit_input(text, &mut ui.store.cx);
}

/// Every dialog a sign-in raises, topmost only: a prompt the sign-in raised hides the choice
/// behind it, and the choice holds its place until that prompt arrives.
fn dialogs(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let state = &ui.screens.settings.accounts;
    let session = &ui.store.session;
    let prompt = match session.state() {
        SessionState::Authorizing(Some(prompt)) => Some(prompt),
        _ => None,
    };
    let manual_secret = state
        .manual_secret
        .filter(|_| matches!(prompt, Some(SignInPrompt::Secret)));
    let accounts = match prompt {
        Some(SignInPrompt::Accounts(accounts)) => Some(accounts),
        _ => None,
    };
    let code = match prompt {
        Some(SignInPrompt::Code { code, url }) => Some((code, url)),
        _ => None,
    };
    let taken = accounts.is_some()
        || code.is_some()
        || manual_secret.is_some()
        || state.credentials_for.is_some();
    // the choice comes down once the sign-in it started is over
    let over = state.sign_in_running && !session.is_pending();
    let sign_in_for = state.sign_in_for.filter(|_| !taken && !over);

    if let Some(accounts) = accounts {
        account_modal(theme, accounts, layer);
    }
    if let Some((code, url)) = code {
        code_prompt(theme, code, url, layer);
    }
    if let Some((slug, provider)) = manual_secret {
        secret_prompt(ui, slug, provider, layer);
    }
    if state.credentials_for.is_some() {
        credentials_prompt(ui, layer);
    }
    if let Some((slug, provider)) = sign_in_for {
        sign_in_prompt(ui, slug, provider, layer);
    }
}

/// The choice of sign-in methods, for a provider that has more than one.
fn sign_in_prompt(ui: &Ui, slug: &'static str, provider: &'static str, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let pending = ui.store.session.is_pending();
    let info = ui.store.session.providers().find(|info| info.slug == slug);
    let (options, web_sign_in) = info
        .map(|info| (info.options, info.web_sign_in))
        .unwrap_or_default();
    let buttons = options
        .into_iter()
        .filter(|option| offered(option, false))
        .flat_map(|method| method_buttons(theme, slug, provider, method, web_sign_in, pending))
        .enumerate()
        .map(|(index, button)| {
            button
                .w_full()
                .transition(Transition::offset(0., 6.).combined(Transition::opacity()))
                .enter_delay(0.04 * index as f32)
        });

    Modal::<Ui>::new(
        "settings-sign-in-prompt",
        t!("login-choose-title", provider = provider),
    )
    .width(CHOICE_WIDTH)
    .close_button()
    .detail(t!("login-choose-detail", provider = provider))
    .child(
        col()
            .w_full()
            .justify_center()
            .gap(theme.font_size * 0.75)
            .min_h(theme.metrics.control * CHOICES)
            .children(buttons),
    )
    .on_dismiss(act(close_sign_in))
    .build(theme, layer);
}

/// The buttons that start one sign-in method. A cookie sign-in yields the browser window only
/// where a backend can draw one, and always the manual paste beside it.
fn method_buttons(
    theme: &Theme,
    slug: &'static str,
    provider: &'static str,
    method: SignIn,
    web_sign_in: bool,
    pending: bool,
) -> Vec<El> {
    let mut buttons = Vec::new();
    let manual = matches!(method, SignIn::Secret);
    if !manual || web_sign_in {
        buttons.push(method_button(theme, slug, provider, method, pending));
    }
    if manual {
        buttons.push(
            Button::new(format!("connect-{slug}-cookies-manual"))
                .label(t!("login-connect-cookies"))
                .secondary()
                .disabled(pending)
                .on_press(act(move |ui: &mut Ui| {
                    st(ui).sign_in_running = true;
                    start_manual(ui, slug, provider);
                }))
                .build(theme)
                .justify_center(),
        );
    }
    buttons
}

fn method_button(
    theme: &Theme,
    slug: &'static str,
    provider: &'static str,
    method: SignIn,
    pending: bool,
) -> El {
    let (key, label) = match &method {
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
    Button::new(key)
        .label(label)
        .primary()
        .disabled(pending)
        .on_press(act(move |ui: &mut Ui| {
            st(ui).sign_in_running = true;
            match &method {
                SignIn::Credentials { .. } => open_credentials(ui, slug),
                method => {
                    let method = method.clone();
                    ui.store.session.sign_in(slug, method, &mut ui.store.cx);
                }
            }
        }))
        .build(theme)
        .justify_center()
}

/// The dialog a server sign-in asks for its address and credentials in.
fn credentials_prompt(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let state = &ui.screens.settings.accounts;
    let field = |key: &str, edit: &TextEdit, hint: &str, access: fn(&mut Ui) -> &mut TextEdit| {
        ui::Input::new(key, edit, i18n::lookup(hint, None), access)
            .on_submit(act_tx(|ui: &mut Ui, _: &mut Tx| submit_credentials(ui)))
    };
    Modal::<Ui>::new("settings-server-prompt", t!("login-server-title"))
        .width(SERVER_WIDTH)
        .detail(t!("login-server-detail"))
        .child(
            field(
                "settings-server",
                &state.server,
                "login-server-hint",
                |ui| &mut st(ui).server,
            )
            .build(theme),
        )
        .child(
            field(
                "settings-username",
                &state.username,
                "login-username-hint",
                |ui| &mut st(ui).username,
            )
            .build(theme),
        )
        .child(
            field(
                "settings-password",
                &state.password,
                "login-password-hint",
                |ui| &mut st(ui).password,
            )
            .masked(true)
            .build(theme),
        )
        .action(
            Button::new("settings-cancel-server")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(clear_credentials))
                .build(theme),
        )
        .action(
            Button::new("settings-submit-server")
                .label(t!("login-server-submit"))
                .primary()
                .on_press(act(submit_credentials))
                .build(theme),
        )
        .on_dismiss(act(clear_credentials))
        .build(theme, layer);
}

/// The manual cookie paste, around the field over this page's own `TextEdit`.
fn secret_prompt(ui: &Ui, slug: &'static str, provider: &'static str, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let secret = ui::Input::new(
        "settings-cookie-secret",
        &ui.screens.settings.accounts.secret,
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

/// The code a device sign-in is waiting on. Closing the dialog cancels the sign-in.
fn code_prompt(theme: &Theme, code: &str, url: &str, layer: &mut Vec<El>) {
    Modal::<Ui>::new("settings-code-prompt", t!("login-device-code", url = url))
        .width(CHOICE_WIDTH)
        .close_button()
        .detail(t!("login-authorizing"))
        .child(device_code(code, theme))
        .action(
            Button::new("settings-cancel-code")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(abandon))
                .build(theme),
        )
        .on_dismiss(act(abandon))
        .build(theme, layer);
}

/// The profiles an account holds, to pick the one to sign in as.
fn account_modal(theme: &Theme, accounts: &[AccountChoice], layer: &mut Vec<El>) {
    account_picker(
        accounts,
        |ui: &mut Ui, id: String| ui.store.session.submit_input(id, &mut ui.store.cx),
        |ui: &mut Ui| ui.store.session.cancel_sign_in(&mut ui.store.cx),
        theme,
        layer,
    );
}

/// The icon a provider's mark is drawn from. Kept here until `shared` carries the one every
/// screen uses.
pub(super) fn provider_logo(slug: &str) -> &'static str {
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

/// The brand colour of the provider `slug`'s mark. Tidal's mark is plain black or white, so it
/// takes the foreground, as does any provider without a colour of its own.
pub(super) fn provider_brand(slug: &str, theme: &Theme) -> ui::Hsla {
    match slug {
        "spotify" => ui::rgb(SPOTIFY).into(),
        "youtube" => ui::rgb(YOUTUBE).into(),
        "deezer" => ui::rgb(DEEZER).into(),
        "apple" => ui::rgb(APPLE).into(),
        "subsonic" => ui::rgb(SUBSONIC).into(),
        _ => theme.foreground,
    }
}

/// The providers' own brand colours, which no theme changes.
const SPOTIFY: u32 = 0x1ed760;
const YOUTUBE: u32 = 0xff0033;
const DEEZER: u32 = 0xa238ff;
const APPLE: u32 = 0xfa2d48;
const SUBSONIC: u32 = 0xffa31a;
