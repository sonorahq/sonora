use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Pixels, Render, StyleRefinement, WeakEntity, Window, div,
};

/// What a part draws, from its page's own state.
type Build<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>) -> AnyElement>;

/// A piece of a page drawn as a view of its own and laid out cached, so that whatever animates
/// beside it renders the page again without building the piece. It rebuilds when the page
/// notifies, and stands as tall as its content measured the last time it drew. Cached, its
/// content rounds to device pixels from the part's own origin, so on a half-pixel origin an edge
/// that falls between pixels may land one device pixel away from where inline layout puts it.
pub(crate) struct Part<V: 'static> {
    page: WeakEntity<V>,
    build: Build<V>,
    height: Pixels,
    /// Whether the last two frames the part drew measured the same height. Until they do, the
    /// part is laid out inline at the height its content takes, so a group unfolding on a spring
    /// pushes its neighbours on the frame it grows rather than overlapping them for one.
    held: bool,
    /// Whether the page notified since the part last drew, so it builds again this frame and may
    /// change its height.
    fresh: bool,
}

impl<V: 'static> Part<V> {
    pub(crate) fn new(
        page: &Entity<V>,
        build: impl Fn(&mut V, &mut Window, &mut Context<V>) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(page, |part, _, cx| {
            part.fresh = true;
            cx.notify();
        })
        .detach();
        Self {
            page: page.downgrade(),
            build: Rc::new(build),
            height: Pixels::ZERO,
            held: false,
            fresh: true,
        }
    }

    /// Lays the part out inline the next time it shows, for a page whose width or text size
    /// changed, which changes the part's height before it can measure it.
    pub(crate) fn unsettle(&mut self) {
        self.held = false;
    }

    /// `part` as its page places it: cached at the height it measured, or inline while that
    /// height is not known to hold.
    pub(crate) fn shown(part: &Entity<Self>, cx: &App) -> AnyElement {
        let state = part.read(cx);
        match state.held && !state.fresh {
            true => part
                .clone()
                .cached(StyleRefinement::default().w_full().h(state.height))
                .into_any_element(),
            false => part.clone().into_any_element(),
        }
    }
}

impl<V: 'static> Render for Part<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fresh = false;
        let build = self.build.clone();
        let content = self
            .page
            .update(cx, |page, cx| build(page, window, cx))
            .unwrap_or_else(|_| div().into_any_element());
        let part = cx.entity().downgrade();
        div()
            .w_full()
            .child(content)
            .on_children_prepainted(move |bounds, window, cx| {
                let Some(height) = bounds.first().map(|bounds| bounds.size.height) else {
                    return;
                };
                let moved = part
                    .update(cx, |part, _| {
                        part.held = part.height == height;
                        part.height = height;
                        !part.held
                    })
                    .unwrap_or(false);
                // a part whose height moved draws again next frame to see whether it held
                if moved {
                    let part = part.clone();
                    window.on_next_frame(move |_, cx| {
                        part.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            })
    }
}
