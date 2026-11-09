//! Being ported from the GPUI version (git: crates/views/src/screens/user).

use rune::{Cx, El, col, text};

use crate::Ui;

/// This screen's own view state.
#[derive(Default)]
pub struct State {}

pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    col()
        .key("screen-user")
        .grow(1.)
        .padding(24.)
        .child(text("user").color(ui.theme.muted_foreground.color()))
}
