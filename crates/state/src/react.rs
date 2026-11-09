//! Where every change reaches the models that follow it. Each model that follows another has a
//! `react_*` method on [`Store`] in its own file; this is the one list of them, in the order
//! they run.

use crate::{Change, Store};

impl Store {
    pub(crate) fn react(&mut self, change: &Change) {
        let _ = change;
    }
}
