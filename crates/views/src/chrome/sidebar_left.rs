//! Being ported from the GPUI version (git: crates/views/src/chrome/sidebar_left.rs).

use rune::{Cx, El, el};
use state::AppSettings;

use crate::Ui;

/// The sidebar's own view state.
#[derive(Default)]
pub struct State {}

impl State {
    pub fn new(_settings: &AppSettings) -> Self {
        Self::default()
    }
}

pub(crate) fn view(_ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    el().key("sidebar-left")
}
