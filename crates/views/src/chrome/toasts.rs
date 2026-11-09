//! Being ported from the GPUI version (git: crates/views/src/chrome/toasts.rs).

use rune::{Cx, El, el};

use crate::Ui;

/// This part's own view state.
#[derive(Default)]
pub struct State {}

pub(crate) fn view(_ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    el().key("toasts")
}
