use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable as _, Pixels, Render, SharedString,
    Window, div, px,
};
use ui::Button;
use ui::{Dismiss, Input};

const WIDEST: Pixels = px(280.);

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
    /// Where focus was before the field took it, handed back when the field closes.
    restore: Option<FocusHandle>,
}

impl Toolbar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            Input::new("common-search", cx)
                .icon("icons/search.svg")
                .compact()
        });

        cx.observe(&input, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
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
            restore: None,
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
        }
        window.focus(&own, cx);
        cx.notify();
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.open {
            true => self.close(window, cx),
            false => self.focus(window, cx),
        }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_end()
            .gap_1()
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.close(window, cx)))
            .children(tools)
            .when(self.apply.is_some(), |this| {
                this.when(self.open, |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .max_w(WIDEST)
                            .child(self.input.clone()),
                    )
                })
                .child(
                    Button::new("search-toggle")
                        .icon(match self.open {
                            true => "icons/x.svg",
                            false => "icons/search.svg",
                        })
                        .tooltip(match self.open {
                            true => "common-dismiss",
                            false => "common-search",
                        })
                        .small()
                        .ghost()
                        .aria_expanded(self.open)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx))),
                )
            })
    }
}
