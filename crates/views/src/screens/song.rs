use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Pixels, Point, Render, SharedString, WeakEntity,
    Window, div, px,
};
use i18n::t;
use music::{Credit, Track};
use router::{Destination, Link as _};
use state::{Playback, SongDetail, Sonora};
use ui::{
    ActiveTheme as _, Avatar, Button, Fact, InfoCard, Initials, Popup, Scrollbar, Scroller,
    Skeleton, Tails as _, Text, clock,
};

use crate::shared::about::{AboutArtist, about_modal};
use crate::shared::cells;
use crate::shared::hero::{
    HeroMetaStrip, HeroPlayButton, PageHero, copyright_notice, release_date_label,
};
use crate::shared::menus::ItemMenu;
use crate::shared::pins::Pinned as _;
use crate::shared::trouble;

const PANEL: Pixels = px(300.);
const TITLE_SKELETON: Pixels = px(240.);
const META_SKELETON: Pixels = px(180.);
const ACTION_SKELETON: Pixels = px(96.);
const FACT_SKELETON: Pixels = px(120.);

pub(crate) struct SongView {
    detail: Entity<SongDetail>,
    playback: Entity<Playback>,
    scrollbar: Entity<Scrollbar>,
    about_bar: Entity<Scrollbar>,
    about_open: bool,
    track_menu: ItemMenu,
    context_menu: Option<Point<Pixels>>,
    me: WeakEntity<Self>,
}

impl SongView {
    fn language_label(code: &str) -> SharedString {
        match code {
            "ar" => t!("language-ar"),
            "de" => t!("language-de"),
            "en" => t!("language-en"),
            "es" => t!("language-es"),
            "fr" => t!("language-fr"),
            "hi" => t!("language-hi"),
            "it" => t!("language-it"),
            "ja" => t!("language-ja"),
            "ko" => t!("language-ko"),
            "pt" => t!("language-pt"),
            "ru" => t!("language-ru"),
            "tr" => t!("language-tr"),
            "uk" => t!("language-uk"),
            "zh" => t!("language-zh"),
            "zxx" => t!("language-zxx"),
            _ => SharedString::from(code.to_owned()),
        }
    }

    pub(crate) fn new(
        detail: Entity<SongDetail>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&detail, |this, _, cx| {
            this.scrollbar
                .read(cx)
                .scroll()
                .set_offset(gpui::Point::default());
            this.about_open = false;
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));

        Self {
            detail,
            playback,
            scrollbar: cx.new(|_| Scrollbar::new(gpui::ScrollHandle::new()).watching(me)),
            about_bar: cx.new(|_| Scrollbar::new(gpui::ScrollHandle::new()).watching(me)),
            about_open: false,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            context_menu: None,
            me: cx.weak_entity(),
        }
    }

    fn hero(&self, track: &Track, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let cover = self
            .detail
            .read(cx)
            .album()
            .and_then(|album| album.album.cover_large.clone())
            .or_else(|| track.cover.clone());
        let album = self.detail.read(cx).album().map(|detail| &detail.album);
        let release = album
            .map(|album| release_date_label(&album.release_date))
            .filter(|release| !release.is_empty());

        let mut meta = HeroMetaStrip::new().item(cells::artist_links(
            "song-artists",
            track.artist_refs.clone(),
            track.artists.clone(),
            theme.muted_foreground,
        ));
        if let Some(release) = release {
            meta = meta.text(release);
        }
        meta = meta.text(clock(track.duration));

        let actions = div()
            .flex()
            .items_center()
            .gap_3()
            .child(HeroPlayButton::new(
                "play-song",
                t!("song-play"),
                vec![track.clone()],
                self.playback.clone(),
            ))
            .when_some(track.album_id.clone(), |this, album_id| {
                this.child(
                    Button::new("open-album")
                        .label(t!("song-view-album"))
                        .outline()
                        .on_click(move |_, _, cx| {
                            router::navigate(Destination::Album(album_id.clone().into()), cx);
                        }),
                )
            });

        let pin = track.pin().map(|pin| pin.cover(cover.clone()));
        let view = self.me.clone();

        PageHero::new("song-hero", track.name.clone())
            .pin(pin)
            .cover(cover)
            .eyebrow(t!("song-eyebrow"))
            .meta(meta)
            .actions(actions)
            .explicit(track.explicit)
            .drag_start(move |event, window, cx| {
                window.prevent_default();
                view.update(cx, |this, cx| {
                    this.track_menu.reset(cx);
                    this.context_menu = Some(event.position);
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }

    fn menu(&self, cx: &mut Context<Self>) -> Option<Popup> {
        let position = self.context_menu?;
        let track = self.detail.read(cx).track().cloned()?;

        Some(
            Popup::new(position, self.track_menu.for_track(&track, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                },
            )),
        )
    }

    fn overview(&self, track: &Track, cx: &Context<Self>) -> AnyElement {
        let album = self.detail.read(cx).album();
        let album_name = album
            .map(|detail| detail.album.name.clone())
            .unwrap_or_else(|| track.album.clone());
        let release = album
            .map(|detail| detail.album.release_date.clone())
            .unwrap_or_default();
        let release = match release.is_empty() {
            true => t!("common-unknown"),
            false => release_date_label(&release),
        };
        let label = album
            .map(|detail| detail.album.label.clone())
            .filter(|value| !value.is_empty())
            .map(SharedString::from)
            .unwrap_or_else(|| t!("common-not-provided"));
        let number = match (track.disc_number, track.track_number) {
            (disc, number) if disc > 1 => t!("song-disc-track", disc = disc, track = number),
            (_, number) if number > 0 => t!("song-track", track = number),
            _ => t!("common-not-provided"),
        };
        let streams = self
            .detail
            .read(cx)
            .playcount()
            .map(cells::count)
            .unwrap_or_else(|| t!("common-not-available"));
        let facts = [
            (t!("song-album"), SharedString::from(album_name)),
            (t!("song-released"), release),
            (t!("song-streams"), streams),
            (t!("song-position"), number),
            (t!("song-label"), label),
            (
                t!("song-popularity"),
                t!("song-popularity-value", value = track.popularity),
            ),
        ];
        InfoCard::new(t!("song-about"))
            .stretch()
            .child(
                div().flex().flex_col().children(
                    facts.into_iter().enumerate().map(|(index, (name, value))| {
                        Fact::new(name, value).striped(index % 2 == 1)
                    }),
                ),
            )
            .into_any_element()
    }

    fn credits(&self, track: &Track, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let rows: Vec<_> = if track.credits.is_empty() {
            track
                .artist_refs
                .iter()
                .map(|artist| Credit {
                    name: artist.name.clone(),
                    role: t!("song-performed-by").to_string(),
                    id: artist.id.clone(),
                })
                .collect()
        } else {
            track.credits.clone()
        };
        let portraits = self.detail.read(cx).portraits().clone();
        InfoCard::new(t!("song-credits"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(rows.into_iter().enumerate().map(|(index, credit)| {
                        let portrait = credit.id.as_ref().and_then(|id| portraits.get(id)).cloned();
                        let avatar = match portrait {
                            Some(portrait) => Avatar::new(Some(portrait))
                                .size(theme.metrics.thumb)
                                .into_any_element(),
                            None => Initials::new(credit.name.clone(), theme.metrics.thumb)
                                .into_any_element(),
                        };
                        let row = div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .gap_3()
                            .px(theme.metrics.pad)
                            .py(theme.metrics.pad / 2.)
                            .rounded(theme.radius)
                            .child(avatar)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .tails()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(credit.name),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_size(theme.text(Text::Small))
                                            .text_color(theme.muted_foreground)
                                            .child(credit.role),
                                    ),
                            );

                        match credit.id {
                            Some(id) => row
                                .id(("song-credit", index))
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.secondary_hover))
                                .rounded(theme.radius)
                                .link(Destination::Artist(id.into()))
                                .into_any_element(),
                            None => row.into_any_element(),
                        }
                    })),
            )
            .into_any_element()
    }

    fn discovery(&self, track: &Track, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let tags = track.tags.clone();
        let languages = match track.languages.is_empty() {
            true => t!("common-not-provided"),
            false => SharedString::from(
                track
                    .languages
                    .iter()
                    .map(|language| Self::language_label(language))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        };
        InfoCard::new(t!("song-details"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().when_else(
                        tags.is_empty(),
                        |this| this.child(Fact::new(t!("song-genres"), t!("common-not-available"))),
                        |this| {
                            this.child(
                                div()
                                    .flex()
                                    .items_start()
                                    .justify_between()
                                    .gap_4()
                                    .min_w_0()
                                    .px(theme.metrics.pad)
                                    .py(theme.metrics.pad / 2.)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_color(theme.muted_foreground)
                                            .child(t!("song-genres")),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w_0()
                                            .flex_wrap()
                                            .justify_end()
                                            .gap_2()
                                            .children(tags.into_iter().map(|tag| {
                                                div()
                                                    .max_w_full()
                                                    .min_w_0()
                                                    .truncate()
                                                    .px_3()
                                                    .py_1()
                                                    .rounded_full()
                                                    .bg(theme.secondary)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .text_size(theme.text(Text::Small))
                                                    .child(tag)
                                            })),
                                    ),
                            )
                        },
                    ))
                    .child(Fact::new(t!("song-language"), languages).striped(true))
                    .child(Fact::new(
                        t!("song-content"),
                        match track.explicit {
                            true => t!("song-explicit"),
                            false => t!("song-clean"),
                        },
                    )),
            )
            .into_any_element()
    }

    fn loading(&self, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let line = || Skeleton::new().w_full().h(theme.metrics.pad);
        let panel = || {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .min_w(PANEL)
                .flex_1()
                .p(theme.metrics.pad)
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(Skeleton::new().w(FACT_SKELETON).h(theme.metrics.pad))
                .children((0..5).map(|_| line()))
        };

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_5()
                    .child(Skeleton::new().size(theme.metrics.cover))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .child(Skeleton::new().w(FACT_SKELETON).h(theme.metrics.pad))
                            .child(Skeleton::new().w(TITLE_SKELETON).h(theme.metrics.control))
                            .child(Skeleton::new().w(META_SKELETON).h(theme.metrics.pad))
                            .child(Skeleton::new().w(ACTION_SKELETON).h(theme.metrics.control)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_stretch()
                    .gap_5()
                    .child(panel())
                    .child(panel()),
            )
            .into_any_element()
    }

    fn artist_profile(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.read(cx);
        let artist = detail.artist()?;

        Some(
            AboutArtist::new("song-artist-profile", artist.name.clone())
                .cover(artist.cover_large.clone())
                .biography(artist.biography.clone())
                .on_open(cx.listener(|this, _, _, cx| {
                    this.about_open = true;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn about_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.about_open {
            return None;
        }

        let detail = self.detail.read(cx);
        let artist = detail.artist()?;
        let artist_id = detail.track()?.artist_refs.first()?.id.clone();

        Some(
            about_modal(
                artist.name.clone().into(),
                artist.biography.clone(),
                artist_id.map(Into::into),
                &self.about_bar,
                cx,
            )
            .action(
                Button::new("song-about-close")
                    .label(t!("common-dismiss"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.about_open = false;
                        cx.notify();
                    })),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| {
                this.about_open = false;
                cx.notify();
            }))
            .into_any_element(),
        )
    }
}

impl SongView {
    /// The page a song shows instead of its hero when it cannot be read: the No connection
    /// state as soon as the network is gone, and the failure of its own load otherwise.
    /// Opening the same song again is the retry.
    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.detail.read(cx).id()?.to_owned();
        let reason = match trouble::unreachable(&id, cx) {
            true => None,
            false => Some(self.detail.read(cx).error()?.to_owned()),
        };
        let detail = self.detail.clone();

        Some(
            trouble::lost(
                "song-lost",
                t!("trouble-not-loaded"),
                reason.as_deref(),
                move |_, _, cx| {
                    let id = id.clone();
                    detail.update(cx, |detail, cx| detail.open(&id, cx));
                },
            )
            .size_full()
            .into_any_element(),
        )
    }
}

impl Render for SongView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(failure) = self.failure(cx) {
            return div().relative().size_full().child(failure);
        }

        let theme = *cx.theme();
        let (track, loading) = {
            let detail = self.detail.read(cx);
            (detail.track().cloned(), detail.is_loading())
        };

        div()
            .relative()
            .size_full()
            .child(
                Scroller::new("song-page", &self.scrollbar)
                    .px(theme.metrics.inset)
                    .py(theme.metrics.inset)
                    .when(loading && track.is_none(), |this| {
                        this.child(self.loading(cx))
                    })
                    .when_some(track, |this, track| {
                        this.child(self.hero(&track, cx))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_stretch()
                                    .gap_5()
                                    .child(
                                        div()
                                            .min_w(PANEL)
                                            .flex_1()
                                            .child(self.overview(&track, cx)),
                                    )
                                    .child(
                                        div()
                                            .min_w(PANEL)
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap_5()
                                            .child(self.credits(&track, cx))
                                            .child(self.discovery(&track, cx)),
                                    ),
                            )
                            .when_some(self.artist_profile(cx), |this, profile| {
                                this.child(div().pt_5().child(profile))
                            })
                            .when_some(
                                self.detail
                                    .read(cx)
                                    .album()
                                    .and_then(|album| album.album.copyrights.first())
                                    .cloned(),
                                |this, copyright| {
                                    this.child(
                                        div()
                                            .pt_5()
                                            .min_w_0()
                                            .text_size(theme.text(Text::Tiny))
                                            .text_color(theme.muted_foreground)
                                            .child(copyright_notice(&copyright)),
                                    )
                                },
                            )
                    }),
            )
            .children(self.menu(cx))
            .children(self.about_dialog(cx))
    }
}
