use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div};
use i18n::t;
use router::{Destination, SettingsPage, navigate};
use state::Blocked;
use ui::{ActiveTheme as _, Button, Text};

use super::rows::drill;
use super::{Group, SettingsView, Slot};

impl SettingsView {
    pub(super) fn blocked_groups(&self) -> Vec<Group> {
        vec![Group::untitled().rows([Slot::BlockedEntries])]
    }

    /// The row that opens the blocked page, saying how much is blocked.
    pub(super) fn blocklist_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.settings.read(cx).blocklist().entries().len();
        let value = match count {
            0 => t!("settings-blocklist-empty"),
            count => t!("settings-blocklist-count", count = count),
        };

        drill("blocklist", t!("settings-blocklist"), Some(value), cx)
            .on_click(|_, _, cx| navigate(Destination::Settings(SettingsPage::Blocked), cx))
            .into_any_element()
    }

    /// The blocked page's list: one row per blocked song or artist with the button that
    /// unblocks it, or a note that nothing is blocked.
    pub(super) fn blocked_entries(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let entries = self.settings.read(cx).blocklist().entries().to_vec();
        let empty = entries.is_empty().then(|| {
            div()
                .flex()
                .items_center()
                .h(theme.metrics.list_row)
                .text_color(theme.muted_foreground)
                .child(t!("settings-blocked-none"))
        });
        let items = entries
            .into_iter()
            .enumerate()
            .map(|(index, blocked)| Self::blocked_item(index, blocked, cx))
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .children(items)
            .children(empty)
            .into_any_element()
    }

    /// One blocked song or artist: its name, what kind of entry it is, and the button that
    /// unblocks it.
    fn blocked_item(index: usize, blocked: Blocked, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let kind = match &blocked {
            Blocked::Artist { .. } => t!("settings-blocked-artist"),
            Blocked::Song { artists, .. } if artists.is_empty() => t!("settings-blocked-song"),
            Blocked::Song { artists, .. } => t!("settings-blocked-song-by", artists = artists),
        };

        div()
            .id(("blocked-item", index))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .h(theme.metrics.list_row)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .truncate()
                            .child(SharedString::from(blocked.name().to_owned())),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .text_size(theme.text(Text::Label))
                            .child(kind),
                    ),
            )
            .child(
                Button::new(("unblock", index))
                    .ghost()
                    .icon("icons/trash-2.svg")
                    .size_8()
                    .px_0()
                    .tooltip("settings-unblock")
                    .tint(theme.muted_foreground)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.playback
                            .update(cx, |playback, cx| playback.unblock(&blocked, cx));
                    })),
            )
            .into_any_element()
    }
}
