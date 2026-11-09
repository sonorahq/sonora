//! The dialogs a sign-in raises, shared by the login screen and the Providers settings: the
//! account choice and the manual cookie sign-in.

mod accounts;
mod cookie;

pub(crate) use accounts::account_picker;
pub(crate) use cookie::{CookiePrompt, cookie_hint};
