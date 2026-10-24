use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div};
use i18n::t;
use router::{Destination, SettingsPage, navigate};
use state::{BlockKind, Sonora};
use ui::{ActiveTheme as _, Button};

use super::rows::{Row, drill};
use super::{Group, SettingsView, Slot};

impl SettingsView {
    pub(super) fn blocked_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-blocked-songs", "icons/music.svg").rows([Slot::BlockedSongs]),
            Group::new("settings-blocked-artists", "icons/user.svg").rows([Slot::BlockedArtists]),
        ]
    }

    /// The row that opens the blocked page, saying how many songs and artists are blocked.
    pub(super) fn blocked_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = Sonora::global(cx).blocks.read(cx).entries(cx).count();
        let value = match count {
            0 => t!("settings-blocked-empty"),
            count => t!("settings-blocked-count", count = count),
        };

        drill("blocked", t!("settings-blocked"), Some(value), cx)
            .on_click(|_, _, cx| navigate(Destination::Settings(SettingsPage::Blocked), cx))
            .into_any_element()
    }

    /// The blocked page's list of one kind, newest first: one row per block with the button that
    /// lifts it, and a song's artists under its name.
    pub(super) fn blocked_list(&self, kind: BlockKind, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let blocks = Sonora::global(cx).blocks.clone();
        let id = match kind {
            BlockKind::Song => "unblock-song",
            BlockKind::Artist => "unblock-artist",
        };

        let items = blocks
            .read(cx)
            .entries(cx)
            .filter(|entry| entry.kind == kind)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let name = SharedString::from(entry.name.clone());
                let artists =
                    (!entry.artists.is_empty()).then(|| SharedString::from(entry.artists.clone()));
                let blocks = blocks.clone();
                let unblock = Button::new((id, index))
                    .ghost()
                    .icon("icons/x.svg")
                    .size_8()
                    .px_0()
                    .tooltip("settings-unblock")
                    .tint(theme.muted_foreground)
                    .on_click(move |_, _, cx| {
                        blocks.update(cx, |blocks, cx| blocks.unblock(&entry, cx));
                    });
                let row = Row::new(name, unblock.into_any_element());
                match artists {
                    Some(artists) => row.detail(artists).into_any_element(),
                    None => row.into_any_element(),
                }
            })
            .collect::<Vec<_>>();
        let empty = items.is_empty().then(|| {
            div()
                .flex()
                .items_center()
                .h(theme.metrics.list_row)
                .text_color(theme.muted_foreground)
                .child(match kind {
                    BlockKind::Song => t!("settings-blocked-songs-none"),
                    BlockKind::Artist => t!("settings-blocked-artists-none"),
                })
        });

        div()
            .flex()
            .flex_col()
            .children(items)
            .children(empty)
            .into_any_element()
    }
}
