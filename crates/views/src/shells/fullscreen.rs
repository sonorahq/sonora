//! Being ported from the GPUI version (git: crates/views/src/shells/fullscreen.rs).

use rune::{Cx, El, el};

use crate::Ui;

/// The fullscreen player's own view state.
#[derive(Default)]
pub struct State {}

pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    el().key("fullscreen").w_full().h_full().fill(ui.theme.background.color())
}
