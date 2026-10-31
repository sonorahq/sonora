use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{Div, Stateful};

use crate::{Destination, navigate};

pub trait Link: Sized {
    fn link(self, to: Destination) -> Self;
}

impl Link for Stateful<Div> {
    fn link(self, to: Destination) -> Self {
        // The role alone makes the link a node; its name stays with the
        // caller, which owns the label the link wraps.
        self.role(Role::Link)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                navigate(to.clone(), cx);
            })
    }
}
