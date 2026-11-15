use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, IntoElement, RenderOnce, Window, prelude::*, px};
use i18n::t;
use ui::{Button, Field, Form, Input, Modal};

type Action = Rc<dyn Fn(&(), &mut Window, &mut App)>;

/// The fields of a sign-in to a server of the user's own: its address, the username and the
/// password. The screen that offers the sign-in keeps them, so what is typed survives a redraw.
#[derive(Clone)]
pub(crate) struct Credentials {
    server: Entity<Input>,
    username: Entity<Input>,
    password: Entity<Input>,
}

impl Credentials {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            server: cx.new(|cx| Input::new("login-server-hint", cx).name("login-server-label")),
            username: cx.new(|cx| Input::new("", cx).name("login-username-hint")),
            password: cx.new(|cx| Input::new("", cx).name("login-password-hint").masked()),
        }
    }

    /// Empties every field.
    pub(crate) fn clear(&self, cx: &mut App) {
        for field in [&self.server, &self.username, &self.password] {
            field.update(cx, |input, cx| input.set_text("", cx));
        }
    }

    /// The address, username and password typed for provider `slug`, or `None` while one of
    /// them is missing. A Jellyfin account may have no password, and leaving out both the
    /// username and the password signs in to Jellyfin by Quick Connect.
    pub(crate) fn filled(&self, slug: &str, cx: &App) -> Option<(String, String, String)> {
        let server = self.server.read(cx).text().to_string();
        let username = self.username.read(cx).text().to_string();
        let password = self.password.read(cx).text().to_string();
        let jellyfin = slug == "jellyfin";
        let quick_connect = jellyfin && username.trim().is_empty() && password.is_empty();
        let missing = server.trim().is_empty()
            || (username.trim().is_empty() && !quick_connect)
            || (password.is_empty() && !jellyfin);
        (!missing).then_some((server, username, password))
    }
}

/// The dialog that asks for a server's address and the account on it.
#[derive(IntoElement)]
pub(crate) struct CredentialsPrompt {
    fields: Credentials,
    provider: &'static str,
    submit: Option<Action>,
    cancel: Option<Action>,
}

impl CredentialsPrompt {
    pub(crate) fn new(fields: &Credentials) -> Self {
        Self {
            fields: fields.clone(),
            provider: "subsonic",
            submit: None,
            cancel: None,
        }
    }

    /// Names the provider signing in, whose title and explanation the dialog shows. Subsonic's
    /// is the default.
    pub(crate) fn provider(mut self, slug: &'static str) -> Self {
        self.provider = slug;
        self
    }

    pub(crate) fn on_submit(
        mut self,
        handler: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.submit = Some(Rc::new(handler));
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

impl RenderOnce for CredentialsPrompt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            fields,
            provider,
            submit,
            cancel,
        } = self;
        let (title, detail) = match provider {
            "jellyfin" => (t!("login-jellyfin-title"), t!("login-jellyfin-detail")),
            _ => (t!("login-server-title"), t!("login-server-detail")),
        };
        let run = |action: &Option<Action>| {
            let action = action.clone();
            move |_: &(), window: &mut Window, cx: &mut App| {
                if let Some(action) = &action {
                    action(&(), window, cx);
                }
            }
        };
        let (submitted, cancelled) = (run(&submit), run(&cancel));
        let (button_submit, button_cancel, dismissed) = (run(&submit), run(&cancel), run(&cancel));

        Modal::new("server-prompt", title)
            .w(px(560.))
            .detail(detail)
            .child(
                Form::new("server-form")
                    .field(Field::new(&fields.server, cx).label(t!("login-server-label")))
                    .field(Field::new(&fields.username, cx).label(t!("login-username-hint")))
                    .field(Field::new(&fields.password, cx).label(t!("login-password-hint")))
                    .on_submit(submitted)
                    .on_dismiss(cancelled),
            )
            .action(
                Button::new("cancel-server")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(move |_, window, cx| button_cancel(&(), window, cx)),
            )
            .action(
                Button::new("submit-server")
                    .label(t!("login-server-submit"))
                    .primary()
                    .on_click(move |_, window, cx| button_submit(&(), window, cx)),
            )
            .on_dismiss(dismissed)
    }
}
