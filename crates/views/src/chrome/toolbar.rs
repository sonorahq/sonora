use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable as _, Render, SharedString,
    Subscription, Window, div,
};
use ui::{ActiveTheme as _, Button, SMALL_INSET};
use ui::{Dismiss, Input};

type Apply = Box<dyn Fn(&str, &mut App)>;
type Tools = Box<dyn Fn(&App) -> Vec<AnyElement>>;

pub(crate) trait Searchable: 'static {
    fn search(&mut self, query: &str, cx: &mut Context<Self>)
    where
        Self: Sized;

    fn hint() -> SharedString
    where
        Self: Sized,
    {
        "common-search".into()
    }
}

pub(crate) trait Tooled: 'static {
    fn toolbar(&self) -> Entity<Toolbar>;

    fn tools(&self, cx: &App) -> Vec<AnyElement>;
}

pub(crate) struct Toolbar {
    input: Entity<Input>,
    apply: Option<Apply>,
    tools: Option<Tools>,
    open: bool,
    query: String,
    /// Where focus was before the field took it, handed back when the field closes.
    restore: Option<FocusHandle>,
    /// Closes the search when it loses focus empty. A new focus replaces it, so they never pile up.
    blur: Option<Subscription>,
}

impl Toolbar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            Input::new("common-search", cx)
                .icon("icons/search.svg")
                .icon_foreground()
                .compact()
                .tucked()
                .transparent()
        });

        cx.observe(&input, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            if this.query == query {
                return;
            }
            this.query.clone_from(&query);
            if let Some(apply) = &this.apply {
                apply(&query, cx);
            }
            cx.notify();
        })
        .detach();

        Self {
            input,
            apply: None,
            tools: None,
            open: false,
            query: String::new(),
            restore: None,
            blur: None,
        }
    }

    pub fn searchable<V: Searchable + Tooled>(
        view: &Entity<V>,
        cx: &mut Context<V>,
    ) -> Entity<Self> {
        let view = view.clone();
        cx.new(|cx| {
            let mut toolbar = Self::new(cx);
            toolbar.bind(&view, cx);
            toolbar.wire(&view, cx);
            toolbar
        })
    }

    pub fn tooled<V: Tooled>(view: &Entity<V>, cx: &mut Context<V>) -> Entity<Self> {
        let view = view.clone();
        cx.new(|cx| {
            let mut toolbar = Self::new(cx);
            toolbar.wire(&view, cx);
            toolbar
        })
    }

    pub fn bind<V: Searchable>(&mut self, view: &Entity<V>, cx: &mut Context<Self>) {
        let target = view.downgrade();
        self.apply = Some(Box::new(move |query, cx| {
            let query = query.to_owned();
            target.update(cx, |view, cx| view.search(&query, cx)).ok();
        }));
        self.input
            .update(cx, |input, cx| input.set_hint(V::hint(), cx));
        cx.notify();
    }

    /// Sets the localized search hint for the current page.
    pub fn set_hint(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_hint(key, cx));
    }

    pub fn wire<V: Tooled>(&mut self, view: &Entity<V>, cx: &mut Context<Self>) {
        let source = view.downgrade();
        self.tools = Some(Box::new(move |cx| {
            source
                .upgrade()
                .map(|view| view.read(cx).tools(cx))
                .unwrap_or_default()
        }));

        cx.observe(view, |_, _, cx| cx.notify()).detach();
        cx.notify();
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.apply.is_none() {
            return;
        }

        self.open = true;
        let own = self.input.read(cx).focus_handle(cx);
        if !own.is_focused(window) {
            self.restore = window.focused(cx);
            self.blur = Some(cx.on_blur(&own, window, |this, window, cx| {
                if this.open && this.input.read(cx).text().is_empty() {
                    this.close(window, cx);
                }
            }));
        }
        window.focus(&own, cx);
        cx.notify();
    }

    /// Shows a restored query when its owning playlist changes.
    pub fn switch_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.open = !query.is_empty();
        self.restore = None;
        self.query = query.to_owned();
        self.input.update(cx, |input, cx| input.set_text(query, cx));
        cx.notify();
    }

    /// Empties and hides the field, and hands focus back to where it was before the field took
    /// it. Focus left on a field that is no longer drawn reaches no handler, shortcuts included.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.input.update(cx, |input, cx| input.set_text("", cx));
        let restore = self.restore.take();
        if self.input.read(cx).focus_handle(cx).is_focused(window) {
            match restore {
                Some(focus) => window.focus(&focus, cx),
                None => window.blur(cx),
            }
        }
        cx.notify();
    }
}

impl Render for Toolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tools = match self.tools.as_ref() {
            Some(tools) => tools(cx),
            None => Vec::new(),
        };

        div()
            .id("toolbar")
            .role(Role::Toolbar)
            .when(self.apply.is_some(), |this| {
                this.aria_label(i18n::lookup("common-search", None))
            })
            .flex()
            .min_w_0()
            .flex_shrink(1.)
            .items_center()
            .gap_1()
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.close(window, cx)))
            .children(tools)
            .when(self.apply.is_some() && !self.open, |this| {
                this.child(
                    Button::new("search-toggle")
                        .icon("icons/search.svg")
                        .tooltip("common-search")
                        .small()
                        .ghost()
                        .tint(cx.theme().foreground)
                        .aria_expanded(false)
                        .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx))),
                )
            })
            .when(self.apply.is_some() && self.open, |this| {
                let theme = *cx.theme();
                let input = self.input.read(cx);
                let active = !input.text().is_empty();
                let focused = input.focus_handle(cx).is_focused(window);
                let width = ui::text_width(input.text().to_owned(), window)
                    .max(ui::text_width(input.placeholder(), window))
                    + theme.metrics.control * 2.
                    + theme.metrics.pad * 3.
                    + SMALL_INSET;
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .w(width)
                        .min_w_0()
                        .flex_shrink(1.)
                        .h(theme.metrics.control_small)
                        .px(SMALL_INSET)
                        .rounded_full()
                        .border_1()
                        .border_color(match focused {
                            true => theme.primary,
                            false => theme.border,
                        })
                        .bg(theme.secondary)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(theme.metrics.control_small)
                                .rounded_full()
                                .mr_1()
                                .when(active, |field| {
                                    field
                                        .bg(super::tools::active_filter_background(cx))
                                        .text_color(theme.primary)
                                })
                                .child(self.input.clone()),
                        )
                        .child(
                            Button::new("search-dismiss")
                                .icon("icons/x.svg")
                                .tooltip("common-dismiss")
                                .small()
                                .ghost()
                                .px_1()
                                .when(active, |button| button.tint(theme.primary))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                                ),
                        ),
                )
            })
    }
}
