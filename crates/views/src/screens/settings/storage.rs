use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, div};
use i18n::t;
use state::{Network, Sonora};
use ui::{ActiveTheme as _, Button, Switch};

use super::{Group, SettingsView, Slot};
use crate::shared::confirm::{Confirm, Kind};

/// The bytes in a gigabyte, the unit the cache limit is set in.
const GIGABYTE: u64 = 1 << 30;
const MEGABYTE: u64 = 1 << 20;

impl SettingsView {
    /// The storage section: the cache of played tracks with what it takes and a way to empty
    /// it, offline mode, and what the downloads take. The cache settings show only for a provider whose played
    /// tracks are kept, while the space each takes always shows, since files stay on disk across
    /// providers.
    pub(super) fn storage_groups(&self, cx: &App) -> Vec<Group> {
        let mut cache = Vec::new();
        if self.session.read(cx).capabilities().keeps {
            cache.push(Slot::KeepTracks);
            if self.playback.read(cx).keep_tracks() {
                cache.push(Slot::CacheSize);
            }
        }
        cache.push(Slot::CacheUsage);
        vec![
            Group::new("settings-group-cache", "icons/rotate-ccw-clock.svg").rows(cache),
            Group::new("settings-group-downloads", "icons/download.svg")
                .rows([Slot::OfflineMode, Slot::DownloadsUsage]),
        ]
    }

    /// What the cached tracks take on disk, with a button that removes them all. Downloads stay.
    pub(super) fn cache_usage_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let used = self.playback.read(cx).usage().map(|(cache, _)| cache);
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .child(ui::spoken("cache-used", size(used)).text_color(theme.muted_foreground))
            .child(
                Button::new("clear-cache")
                    .label(t!("settings-cache-clear"))
                    .small()
                    .outline()
                    .disabled(used.is_none_or(|used| used == 0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.playback
                            .update(cx, |playback, cx| playback.clear_cache(cx));
                    })),
            );
        self.row(t!("settings-cache-used"), control.into_any_element())
    }

    /// Offline mode, which keeps Sonora to what was downloaded until it is turned off again.
    pub(super) fn offline_mode_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = Network::offline(cx);
        self.row(
            t!("settings-offline-mode"),
            Switch::new("offline-mode", on)
                .name(t!("settings-offline-mode"))
                .on_click(move |_, _, cx| {
                    let network = Network::global(cx);
                    network.update(cx, |network, cx| network.set_offline(!on, cx));
                })
                .into_any_element(),
        )
    }

    /// What the downloads take on disk, with a button that removes them all once confirmed.
    pub(super) fn downloads_usage_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let used = self
            .playback
            .read(cx)
            .usage()
            .map(|(_, downloads)| downloads);
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .child(ui::spoken("downloads-used", size(used)).text_color(theme.muted_foreground))
            .child(
                Button::new("remove-downloads")
                    .label(t!("settings-downloads-remove"))
                    .small()
                    .outline()
                    .disabled(used.is_none_or(|used| used == 0))
                    .on_click(|_, _, cx| {
                        Confirm::ask(
                            Kind::Downloads,
                            |cx| {
                                let downloads = Sonora::global(cx).downloads.clone();
                                downloads.update(cx, |downloads, cx| downloads.remove_all(cx));
                            },
                            cx,
                        );
                    }),
            );
        self.row(t!("settings-downloads-used"), control.into_any_element())
    }
}

/// A size in bytes as a short label: gigabytes to one decimal from a gigabyte up, whole
/// megabytes below. Nothing measured yet reads as measuring.
fn size(bytes: Option<u64>) -> SharedString {
    let Some(bytes) = bytes else {
        return t!("settings-size-measuring");
    };
    let mut args = i18n::FluentArgs::new();
    match bytes >= GIGABYTE {
        true => {
            args.set("size", format!("{:.1}", bytes as f64 / GIGABYTE as f64));
            i18n::lookup("settings-size-gb", Some(&args))
        }
        false => {
            args.set("size", (bytes / MEGABYTE) as i64);
            i18n::lookup("settings-size-mb", Some(&args))
        }
    }
}
