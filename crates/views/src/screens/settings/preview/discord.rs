//! A replica of the activity card Discord shows in a profile for the status Sonora sets, in
//! Discord's own dark theme, sizes and type. It follows every setting on the page and draws the
//! status the way `state::discord` would send it for a track from listening history. Screen
//! readers hear it as a named group of its text lines, buttons and provider badge.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, Pixels, SharedString, Window, div, px, relative,
    rgb, svg,
};
use i18n::t;
use state::DiscordName;
use ui::ActiveTheme as _;
use ui::{Artwork, tabular};

use super::Tick;
use crate::screens::settings::SettingsView;
use crate::shared::{provider_brand, provider_logo};

/// The face Discord's cards are set in here.
const FONT: &str = "Inter";
/// The activity card's own fill.
const CARD: u32 = 0x2b2d31;
/// The card's header line, "Listening to …".
const HEADER: u32 = 0xdbdee1;
/// The track's title.
const TITLE: u32 = 0xf2f3f5;
/// The artist line.
const TEXT: u32 = 0xdbdee1;
/// The times either side of the progress bar.
const MUTED: u32 = 0x949ba4;
/// The progress bar's track and its fill.
const BAR: u32 = 0x4e5058;
const BAR_FILL: u32 = 0xdbdee1;
/// A secondary button and its label.
const BUTTON: u32 = 0x4e5058;
const BUTTON_HOVER: u32 = 0x6d6f78;
const BUTTON_TEXT: u32 = 0xffffff;
/// Sonora's application icon, which Discord shows in place of art the status leaves out: the
/// mark in black on white.
const APP_ICON: u32 = 0xffffff;
const APP_MARK: u32 = 0x000000;
/// The badge's disc is the provider's own colour with its mark in white on it.
const BADGE_MARK: u32 = 0xffffff;
/// How wide the card stands and how far its content sits from its edge.
const CARD_WIDTH: Pixels = px(372.);
const CARD_PAD: Pixels = px(12.);
const CARD_RADIUS: Pixels = px(8.);
/// How much room the popout leaves around the card.
const SURFACE_PAD: Pixels = px(20.);
/// The large image, its corners and the gap to the text beside it.
const ART: Pixels = px(120.);
const ART_RADIUS: Pixels = px(8.);
const ART_GAP: Pixels = px(12.);
/// The provider badge, the ring of card colour around it, and how far it hangs past the art.
const BADGE_SIZE: Pixels = px(32.);
const BADGE_RING: Pixels = px(4.);
const BADGE_OVERHANG: Pixels = px(4.);
/// Sonora's mark on the application icon, as a share of the icon.
const APP_MARK_SHARE: f32 = 0.56;
/// A provider's mark on the badge, as a share of the badge.
const BADGE_MARK_SHARE: f32 = 0.5;
/// Discord's type sizes: the header, the title, the artist and album lines, the times and the
/// buttons.
const HEADER_SIZE: Pixels = px(14.);
const TITLE_SIZE: Pixels = px(16.);
const TEXT_SIZE: Pixels = px(14.);
const TIME_SIZE: Pixels = px(14.);
const BUTTON_SIZE: Pixels = px(14.);
/// The progress bar's height, and the room between it and the times either side.
const BAR_HEIGHT: Pixels = px(4.);
const BAR_GAP: Pixels = px(12.);
/// The room between the album line and the times under it.
const TIME_GAP: Pixels = px(8.);
/// A button's height and corners, and the gap between two.
const BUTTON_HEIGHT: Pixels = px(32.);
const BUTTON_RADIUS: Pixels = px(8.);
const BUTTON_GAP: Pixels = px(8.);
/// How far through the track the card starts, as a share of its length, before it counts on.
const START_SHARE: f32 = 0.3;
/// How much of the card shows while Discord presence is disabled.
const DIMMED: f32 = 0.4;
/// What the status calls itself when the setting names Sonora, Discord's name for the app.
const SONORA: &str = "Sonora";

/// What the Discord page's preview keeps between frames.
pub(in crate::screens::settings) struct DiscordPreview {
    /// When the card started counting, which settings opening last set.
    since: Cell<Instant>,
    /// The second-hand that moves the elapsed time on.
    tick: Tick,
}

/// What the status shows about time: a bar to the end of the track, the time since listening
/// started when the details are hidden, or nothing while paused.
enum Timing {
    Bar { elapsed: Duration, total: Duration },
    Elapsed(Duration),
    Still,
}

impl DiscordPreview {
    pub(in crate::screens::settings) fn new(_cx: &mut Context<SettingsView>) -> Self {
        Self {
            since: Cell::new(Instant::now()),
            tick: Tick::default(),
        }
    }

    /// Settings opened, so the card counts from a new start.
    pub(super) fn opened(&self) {
        self.since.set(Instant::now());
    }
}

/// The Discord page's preview, the card Discord shows for the status Sonora sets. It shows a
/// track picked from listening history, playing while Sonora plays or has nothing loaded and
/// paused while Sonora is paused, and dims only while Discord presence is disabled.
pub(in crate::screens::settings) fn preview(
    state: &DiscordPreview,
    view: &SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Option<AnyElement> {
    let sample = view.previews.picked(cx);
    let settings = view.settings.read(cx);
    let playback = view.playback.read(cx);
    let paused = playback.track().is_some() && !playback.wants_playing();
    let on = settings.discord_presence();
    let name = settings.discord_name();
    let badge = settings.discord_badge();
    let hidden = settings.discord_without_details();
    let sonora_button = settings.discord_sonora_button();
    let provider_button = settings.discord_provider_button();
    let local_art = settings.artwork_for_local_files();

    let session = view.session.read(cx);
    let provider = sample
        .id
        .as_deref()
        .and_then(|id| session.provider_for(id))
        .map(|provider| (provider.slug(), provider.name(), provider.listening_to()))
        .or_else(|| {
            let slug = session.provider_slug()?;
            let name = session.provider_name()?;
            Some((slug, name, name))
        });
    let local = sample.id.as_deref().is_some_and(music::is_local_id);

    let listening: SharedString = match name {
        DiscordName::Sonora => SONORA.into(),
        DiscordName::Provider => provider
            .map(|(_, _, listening)| SharedString::from(listening))
            .unwrap_or_else(|| SONORA.into()),
        DiscordName::Music => t!("settings-discord-preview-music"),
        DiscordName::Title => sample.title.clone(),
        DiscordName::Artist => sample.artist.clone(),
        DiscordName::ArtistTitle => format!("{} - {}", sample.artist, sample.title).into(),
    };
    let elapsed = state.since.get().elapsed();
    let timing = match (paused, hidden) {
        (true, _) => Timing::Still,
        (false, true) => Timing::Elapsed(elapsed),
        (false, false) => {
            let total = sample.duration.max(Duration::from_secs(1));
            let start = total.mul_f32(START_SHARE);
            let at = (start + elapsed).as_secs() % total.as_secs().max(1);
            Timing::Bar {
                elapsed: Duration::from_secs(at),
                total,
            }
        }
    };
    if on && !matches!(timing, Timing::Still) {
        let next = Duration::from_secs(1) - Duration::from_nanos(u64::from(elapsed.subsec_nanos()));
        state.tick.after(next, window, cx);
    }
    let art = match hidden || local && !local_art {
        true => None,
        false => sample.large_cover(cx),
    };
    let badge = badge
        .then_some(provider)
        .flatten()
        .map(|(slug, name, _)| (slug, name));
    let mut buttons = Vec::new();
    if provider_button
        && !hidden
        && !local
        && let Some((_, name, _)) = provider
    {
        buttons.push(t!("settings-discord-preview-listen-on", provider = name));
    }
    if sonora_button {
        buttons.push(t!("get-sonora"));
    }

    let (title, artist, album) = match hidden {
        true => (t!("discord-listening"), None, None),
        false => (
            sample.title.clone(),
            Some(sample.artist.clone()),
            (!sample.album.is_empty()).then(|| sample.album.clone()),
        ),
    };
    let card = div()
        .id("discord-preview")
        .role(Role::Group)
        .aria_label(t!("settings-discord-preview"))
        .w(CARD_WIDTH)
        .p(CARD_PAD)
        .flex()
        .flex_col()
        .gap(CARD_PAD)
        .rounded(CARD_RADIUS)
        .bg(rgb(CARD))
        .font_family(FONT)
        .child(
            ui::spoken(
                "discord-preview-header",
                t!("settings-discord-preview-listening", name = &listening),
            )
            .text_size(HEADER_SIZE)
            .text_color(rgb(HEADER))
            .truncate(),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(ART_GAP)
                .child(image(art, badge, cx))
                .child(details(title, artist, album, timing)),
        )
        .when(!buttons.is_empty(), |card| {
            card.child(div().flex().flex_col().gap(BUTTON_GAP).children(
                buttons.into_iter().enumerate().map(|(index, label)| {
                    div()
                        .id(("discord-preview-button", index))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .h(BUTTON_HEIGHT)
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(BUTTON_RADIUS)
                        .bg(rgb(BUTTON))
                        .hover(|style| style.bg(rgb(BUTTON_HOVER)))
                        .text_size(BUTTON_SIZE)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(BUTTON_TEXT))
                        .child(label)
                }),
            ))
        });

    Some(
        super::framed(
            div()
                .w_full()
                .flex()
                .justify_center()
                .p(SURFACE_PAD)
                .bg(cx.theme().background)
                .child(card.when(!on, |card| card.opacity(DIMMED))),
            cx,
        )
        .into_any_element(),
    )
}

/// The large image with the provider's badge at its bottom right, or Sonora's application
/// icon when the status carries no art, as Discord falls back to it. The badge, given as the
/// provider's slug and name, reads to screen readers as an image named after the provider.
fn image(art: Option<String>, badge: Option<(&'static str, &'static str)>, cx: &gpui::App) -> Div {
    let picture = match art {
        Some(url) => Artwork::new(Some(url))
            .size(ART)
            .corner_radius(ART_RADIUS)
            .into_any_element(),
        None => div()
            .size(ART)
            .flex()
            .items_center()
            .justify_center()
            .rounded(ART_RADIUS)
            .bg(rgb(APP_ICON))
            .child(
                svg()
                    .path(icons::path("icons/sonora.svg"))
                    .size(ART * APP_MARK_SHARE)
                    .text_color(rgb(APP_MARK)),
            )
            .into_any_element(),
    };

    div()
        .relative()
        .flex_none()
        .size(ART)
        .child(picture)
        .when_some(badge, |this, (slug, name)| {
            let brand: Hsla = provider_brand(slug, cx);
            let outer = BADGE_SIZE + BADGE_RING * 2.;
            this.child(
                div()
                    .id("discord-preview-badge")
                    .role(Role::Image)
                    .aria_label(name)
                    .absolute()
                    .right(-BADGE_OVERHANG)
                    .bottom(-BADGE_OVERHANG)
                    .size(outer)
                    .p(BADGE_RING)
                    .rounded_full()
                    .bg(rgb(CARD))
                    .child(
                        div()
                            .size(BADGE_SIZE)
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(brand)
                            .child(
                                svg()
                                    .path(icons::path(provider_logo(slug)))
                                    .size(BADGE_SIZE * BADGE_MARK_SHARE)
                                    .text_color(rgb(BADGE_MARK)),
                            ),
                    ),
            )
        })
}

/// The text beside the image: the title, the artist, the album, and the time the status shows.
fn details(
    title: SharedString,
    artist: Option<SharedString>,
    album: Option<SharedString>,
    timing: Timing,
) -> Div {
    let time = |id: &'static str, value: Duration| {
        ui::spoken(id, elapsed_clock(value))
            .flex_none()
            .text_size(TIME_SIZE)
            .text_color(rgb(MUTED))
            .font_features(tabular())
    };

    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            ui::spoken("discord-preview-title", title)
                .text_size(TITLE_SIZE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TITLE))
                .truncate(),
        )
        .children(artist.map(|artist| {
            ui::spoken("discord-preview-artist", artist)
                .text_size(TEXT_SIZE)
                .text_color(rgb(TEXT))
                .truncate()
        }))
        .children(album.map(|album| {
            ui::spoken("discord-preview-album", album)
                .text_size(TEXT_SIZE)
                .text_color(rgb(TEXT))
                .truncate()
        }))
        .map(|this| match timing {
            Timing::Bar { elapsed, total } => this.child(
                div()
                    .mt(TIME_GAP)
                    .flex()
                    .items_center()
                    .gap(BAR_GAP)
                    .child(time("discord-preview-elapsed", elapsed))
                    .child(
                        div()
                            .flex_1()
                            .h(BAR_HEIGHT)
                            .rounded_full()
                            .bg(rgb(BAR))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(elapsed.as_secs_f32() / total.as_secs_f32()))
                                    .rounded_full()
                                    .bg(rgb(BAR_FILL)),
                            ),
                    )
                    .child(time("discord-preview-total", total)),
            ),
            Timing::Elapsed(elapsed) => this.child(
                ui::spoken(
                    "discord-preview-elapsed",
                    t!(
                        "settings-discord-preview-elapsed",
                        time = elapsed_clock(elapsed)
                    ),
                )
                .mt_1()
                .text_size(TIME_SIZE)
                .text_color(rgb(MUTED))
                .font_features(tabular()),
            ),
            Timing::Still => this,
        })
}

/// Time since listening started the way Discord counts it, minutes and seconds both padded.
fn elapsed_clock(value: Duration) -> String {
    let total = value.as_secs();
    match total / 3600 {
        0 => format!("{:02}:{:02}", total / 60, total % 60),
        hours => format!("{hours:02}:{:02}:{:02}", total % 3600 / 60, total % 60),
    }
}
