use std::rc::Rc;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, IntoElement, RenderOnce, Window, div, prelude::*,
};
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
        let fields = Self {
            server: cx.new(|cx| Input::new("login-server-hint", cx).name("login-server-label")),
            username: cx.new(|cx| Input::new("", cx).name("login-username-hint")),
            password: cx.new(|cx| Input::new("", cx).name("login-password-hint").masked()),
        };
        // a field marked as missing loses the mark as soon as something is typed into it
        for field in fields.all() {
            cx.observe(field, |field, cx| {
                if !field.read(cx).text().trim().is_empty() {
                    field.update(cx, |input, cx| input.set_invalid(false, cx));
                }
            })
            .detach();
        }
        fields
    }

    fn all(&self) -> [&Entity<Input>; 3] {
        [&self.server, &self.username, &self.password]
    }

    /// Empties every field and clears the missing marks.
    pub(crate) fn clear(&self, cx: &mut App) {
        for field in self.all() {
            field.update(cx, |input, cx| {
                input.set_text("", cx);
                input.set_invalid(false, cx);
            });
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

    /// Marks every field in `needed` that is still empty and moves the focus to the first of
    /// them. Returns whether all of them hold something.
    fn check(&self, needed: &[&Entity<Input>], window: &mut Window, cx: &mut App) -> bool {
        let empty: Vec<&Entity<Input>> = needed
            .iter()
            .copied()
            .filter(|field| field.read(cx).text().trim().is_empty())
            .collect();
        for field in &empty {
            field.update(cx, |input, cx| input.set_invalid(true, cx));
        }
        if let Some(first) = empty.first() {
            let focus = first.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        empty.is_empty()
    }
}

/// The dialog that asks for a server's address and the account on it. A Jellyfin server also
/// offers Quick Connect, which needs only the address.
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

    /// Names the provider signing in, whose title and explanation the dialog shows.
    /// Subsonic's is the default.
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
        let jellyfin = provider == "jellyfin";
        let (title, detail) = match jellyfin {
            true => (t!("login-jellyfin-title"), t!("login-jellyfin-detail")),
            false => (t!("login-server-title"), t!("login-server-detail")),
        };
        // the sign-in itself, which needs the password everywhere but on Jellyfin. On Jellyfin
        // empty account fields ask for Quick Connect, which keeps it in reach of Enter.
        let sign_in: Action = {
            let fields = fields.clone();
            let submit = submit.clone();
            Rc::new(move |_, window, cx| {
                let blank = |field: &Entity<Input>| field.read(cx).text().is_empty();
                let quick = jellyfin && blank(&fields.username) && blank(&fields.password);
                let needed = match (jellyfin, quick) {
                    (true, true) => vec![&fields.server],
                    (true, false) => vec![&fields.server, &fields.username],
                    (false, _) => vec![&fields.server, &fields.username, &fields.password],
                };
                if !fields.check(&needed, window, cx) {
                    return;
                }
                if let Some(submit) = &submit {
                    submit(&(), window, cx);
                }
            })
        };
        // Quick Connect needs only the address, and empty account fields are what ask for it
        let quick_connect = {
            let fields = fields.clone();
            let submit = submit.clone();
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                if !fields.check(&[&fields.server], window, cx) {
                    return;
                }
                for field in [&fields.username, &fields.password] {
                    field.update(cx, |input, cx| {
                        input.set_text("", cx);
                        input.set_invalid(false, cx);
                    });
                }
                if let Some(submit) = &submit {
                    submit(&(), window, cx);
                }
            }
        };
        let dismiss = move |_: &(), window: &mut Window, cx: &mut App| {
            if let Some(cancel) = &cancel {
                cancel(&(), window, cx);
            }
        };
        let (submitted, clicked) = (sign_in.clone(), sign_in);

        Modal::new("server-prompt", title)
            .detail(detail)
            .close_button()
            .child(
                Form::new("server-form")
                    .field(Field::new(&fields.server, cx).label(t!("login-server-label")))
                    .field(Field::new(&fields.username, cx).label(t!("login-username-hint")))
                    .field(Field::new(&fields.password, cx).label(t!("login-password-hint")))
                    .on_submit(move |_, window, cx| submitted(&(), window, cx))
                    .on_dismiss(dismiss.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .pt_2()
                    .child(
                        Button::new("submit-server")
                            .primary()
                            .large()
                            .w_full()
                            .label(t!("login-server-submit"))
                            .on_click(move |_, window, cx| clicked(&(), window, cx)),
                    )
                    .when(jellyfin, |this| {
                        this.child(
                            Button::new("quick-connect")
                                .outline()
                                .large()
                                .w_full()
                                .label(t!("login-quick-connect"))
                                .on_click(quick_connect),
                        )
                    }),
            )
            .on_dismiss(dismiss)
    }
}
