//! The context menu every album, playlist, artist and track card opens. The menu itself is the
//! one [`super::Menus`] holds and the workspace draws, so no screen keeps menu state of its own
//! for a card to have one.

use music::GenreItem;
use rune::{PointerPress, Tx};

use super::{Target, open};
use crate::Ui;

/// The secondary-press handler a card takes (`ui::Card::menu`), which opens the menu of `item`
/// where the pointer is. A genre has no menu, so its card gets a handler that does nothing.
pub(crate) fn opener(item: &GenreItem) -> impl Fn(&mut Ui, PointerPress, &mut Tx) + 'static {
    let target = Target::of(item);
    move |ui: &mut Ui, press: PointerPress, _: &mut Tx| {
        if let Some(target) = &target {
            open(ui, target.clone(), press);
        }
    }
}
