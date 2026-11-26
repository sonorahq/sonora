use gpui::prelude::*;
use gpui::{App, Context, Entity, Global, MouseDownEvent, Pixels, Point, Window, div};
use state::{Playback, Sonora};
use ui::{Popup, Scrollbar};

use crate::shared::menus::{Item, ItemMenu};

/// The one context menu every album, playlist, artist and track card opens, drawn once at the
/// root of the workspace. A card factory attaches it, so no screen has to keep menu state of
/// its own for a card to have one.
pub(crate) struct CardMenu {
    menus: ItemMenu,
    open: Option<Opened>,
}

/// The item a menu is open on, the playback its entries act through, and where it was asked for.
struct Opened {
    item: Item,
    playback: Entity<Playback>,
    at: Point<Pixels>,
}

struct Installed(Entity<CardMenu>);

impl Global for Installed {}

impl CardMenu {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let menu = cx.new(|cx: &mut Context<Self>| {
                let library = Sonora::global(cx).library.clone();
                cx.observe(&library, |this, _, cx| {
                    if this.open.is_some() {
                        cx.notify();
                    }
                })
                .detach();
                let host = cx.entity_id();
                let scrollbar = cx.new(|_| Scrollbar::inset().watching(host));
                Self {
                    menus: ItemMenu::new(scrollbar, cx),
                    open: None,
                }
            });
            cx.set_global(Installed(menu));
        }
        cx.global::<Installed>().0.clone()
    }

    /// The right-click handler a card takes, which opens the menu of `item` where the pointer is.
    pub(crate) fn opener(
        item: Item,
        playback: Entity<Playback>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        move |event, _, cx| {
            let opened = Opened {
                item: item.clone(),
                playback: playback.clone(),
                at: event.position,
            };
            Self::entity(cx).update(cx, |this, cx| {
                this.menus.reset(cx);
                this.open = Some(opened);
                cx.notify();
            });
        }
    }
}

impl Render for CardMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div();
        };
        let menu = open
            .item
            .menu(&self.menus, open.playback.clone(), false, cx);

        div().child(
            Popup::new(open.at, menu).on_close(cx.listener(|this, _, _, cx| {
                this.open = None;
                cx.notify();
            })),
        )
    }
}
