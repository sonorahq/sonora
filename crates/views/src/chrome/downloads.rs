use gpui::prelude::*;
use gpui::{Context, Entity, Window, div, px};
use i18n::t;
use router::{Destination, navigate};
use state::{Downloads, Sonora, Subject};
use ui::{ActiveTheme as _, Button, MenuItem, Picker, Popovers};

/// The title bar's button for the albums, playlists and songs downloading: each with how far it
/// has come and, for a whole album or playlist, the tracks running now. It draws nothing while
/// no download is under way.
pub(crate) struct DownloadsButton {
    downloads: Entity<Downloads>,
    popovers: Popovers,
}

impl DownloadsButton {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let downloads = Sonora::global(cx).downloads.clone();
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();

        Self {
            downloads,
            popovers: Popovers::default(),
        }
    }
}

impl Render for DownloadsButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.downloads.read(cx).active();
        if active.is_empty() {
            self.popovers.close();
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let mut items = Vec::new();
        for (at, set) in active.iter().enumerate() {
            let icon = match set.subject {
                Subject::Album => "icons/disc-3.svg",
                Subject::Playlist => "icons/list-music.svg",
                Subject::Song => "icons/music.svg",
            };
            let (subject, id) = (set.subject, set.id.clone());
            let downloads = self.downloads.clone();
            let popovers = self.popovers.clone();
            let percent = format!("{}%", (set.share * 100.).floor() as usize);
            let detail = match (subject, set.eta) {
                (Subject::Song, _) => None,
                (_, Some(eta)) => Some(
                    t!(
                        "downloads-count-eta",
                        done = set.done,
                        total = set.total,
                        minutes = eta.as_secs().div_ceil(60).max(1) as usize
                    )
                    .to_string(),
                ),
                _ => Some(t!("downloads-count", done = set.done, total = set.total).to_string()),
            };
            items.push(
                MenuItem::new(("download-set", at), set.name.clone())
                    .map(|item| match set.cover.clone() {
                        Some(cover) => item.artwork(Some(cover)),
                        None => item.icon(icon),
                    })
                    .map(|item| match detail {
                        Some(detail) => item
                            .detail(div().text_color(theme.muted_foreground).child(detail)),
                        None => item,
                    })
                    .trailing(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w(px(36.))
                                    .text_right()
                                    .text_color(theme.foreground)
                                    .child(percent),
                            )
                            .child(
                                Button::new(("download-cancel", at))
                                    .ghost()
                                    .small()
                                    .icon("icons/x.svg")
                                    .tooltip("downloads-cancel")
                                    .on_click({
                                        let (downloads, id) = (downloads.clone(), id.clone());
                                        move |_, _, cx| {
                                            downloads.update(cx, |downloads, cx| match subject {
                                                Subject::Album => {
                                                    downloads.cancel_set(state::Collection::Album, &id, cx)
                                                }
                                                Subject::Playlist => downloads.cancel_set(
                                                    state::Collection::Playlist,
                                                    &id,
                                                    cx,
                                                ),
                                                Subject::Song => downloads.cancel_song(&id, cx),
                                            });
                                        }
                                    }),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        let destination = match subject {
                            Subject::Album => Destination::Album(id.clone().into()),
                            Subject::Playlist => Destination::Playlist(id.clone().into()),
                            Subject::Song => Destination::Song(id.clone().into()),
                        };
                        popovers.close();
                        navigate(destination, cx);
                        cx.refresh_windows();
                    }),
            );
            for (track, (name, fraction)) in set.current.iter().enumerate() {
                let mut item = MenuItem::new(("download-track", at * 1000 + track), name.clone())
                    .icon("icons/download.svg")
                    .disabled();
                if let Some(fraction) = fraction {
                    item = item.trailing(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(format!("{:.0}%", fraction * 100.)),
                    );
                }
                items.push(item);
            }
            if at + 1 < active.len() {
                items.push(MenuItem::separator(("download-separator", at)));
            }
        }

        Picker::icon("downloads", &self.popovers, "icons/download.svg")
            .tooltip("downloads-active")
            .tint(theme.foreground)
            .width(Picker::WIDE)
            .items(items)
            .into_any_element()
    }
}
