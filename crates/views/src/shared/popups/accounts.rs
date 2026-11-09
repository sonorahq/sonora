//! The dialog a sign-in raises when the account it reached holds several profiles to choose from.

use rune::{Cursor, El, col, row, text};
use music::AccountChoice;
use ui::{Button, Initials, Modal, Text, Theme, Weight};

use crate::Ui;

/// How large each account's initials stand.
const AVATAR: f32 = 32.;
/// How wide the dialog stands.
const WIDTH: f32 = 440.;
const BORDER: f32 = 1.;

/// Puts the account choice up in `layer`: one row per account, each running `pick` with the
/// account's id, and a Cancel that runs `cancel`, as a press outside or Escape does. Both run
/// inside `act`, so they reach the store.
pub(crate) fn account_picker(
    accounts: &[AccountChoice],
    pick: impl Fn(&mut Ui, String) + Clone + 'static,
    cancel: impl Fn(&mut Ui) + Clone + 'static,
    theme: &Theme,
    layer: &mut Vec<El>,
) {
    let total = accounts.len();
    let rows = accounts.iter().enumerate().map(|(place, account)| {
        let pick = pick.clone();
        let id = account.id.clone();
        let mut words = col().grow(1.).shrink().min_w(0.).child(
            text(account.name.clone())
                .font(ui::font(theme, Text::Body, Weight::MEDIUM))
                .color(theme.popover_foreground.color())
                .truncate()
                .min_w(0.),
        );
        if let Some(detail) = &account.detail {
            words = words.child(
                text(detail.clone())
                    .font(ui::font(theme, Text::Small, Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .truncate()
                    .min_w(0.),
            );
        }
        let mut entry = row()
            .key(format!("account-{}", account.id))
            .role("button")
            .label(account.name.clone())
            .position_in_set(place + 1, total)
            .focusable(true)
            .center_y()
            .gap(theme.font_size * 0.75)
            .w_full()
            .padding(theme.font_size * 0.5)
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .cursor(Cursor::Pointer)
            .hover(theme.secondary_hover.color())
            .press_scale(ui::PRESS_SCALE_CARD)
            .on_press(crate::act(move |ui: &mut Ui| pick(ui, id.clone())))
            .child(Initials::new(account.name.clone(), AVATAR).build(theme))
            .child(words);
        if let Some(detail) = &account.detail {
            entry = entry.description(detail.clone());
        }
        entry
    });
    let cancelled = cancel.clone();

    Modal::<Ui>::new("account-picker", i18n::t!("login-account-title"))
        .width(WIDTH)
        .detail(i18n::t!("login-account-detail"))
        .child(col().w_full().gap(theme.font_size * 0.5).children(rows))
        .action(
            Button::new("cancel-account")
                .ghost()
                .label(i18n::t!("common-cancel"))
                .on_press(crate::act(move |ui: &mut Ui| cancelled(ui)))
                .build(theme),
        )
        .on_dismiss(crate::act(move |ui: &mut Ui| cancel(ui)))
        .build(theme, layer);
}
