//! A replica of the activity card Discord shows in a profile for the status Sonora sets, in
//! Discord's own dark theme, sizes and type. It follows every setting on the page and draws the
//! status the way `state::discord` would send it for a track from listening history. It is also
//! where those settings change: the status name opens a menu of names, and the provider badge
//! and the buttons turn on and off where they stand, drawn as dashed outlines with a plus while
//! off. Screen readers hear it as a named group of its text lines, the name's picker and a
//! toggle button for the badge and for each button.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, Pixels, SharedString, Window, div,
    px, relative, rgb, rgba, svg,
};
use i18n::t;
use state::DiscordName;
use ui::ActiveTheme as _;
use ui::Glowing as _;
use ui::{Artwork, Menu, MenuItem, Picker, Popover, Text, tabular};

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
/// What the status calls itself when the setting names Sonora, Discord's name for the app.
const SONORA: &str = "Sonora";
/// The dashed outline and the plus of a badge or button that is off, white at low strength
/// over the card.
const GHOST_EDGE: u32 = 0xffffff4d;
const GHOST_TEXT: u32 = 0xffffff8c;
/// How large the plus on a badge or button that is off draws.
const PLUS: Pixels = px(14.);
/// The popover key of the status name's menu, how far the header's hover fill reaches past
/// its text, and the fill itself, white at low strength over the card.
const NAME_MENU: &str = "discord-name";
const NAME_INSET: Pixels = px(4.);
const NAME_HOVER: u32 = 0xffffff14;

/// What a click on a part of the card does.
type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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

/// The Discord page's card, the one Discord shows for the status Sonora sets. It shows a
/// track picked from listening history, playing while Sonora plays or has nothing loaded and
/// paused while Sonora is paused. While the status is on its name, badge and buttons change
/// where they stand, and a line under the card says so. While it is off the card dims and
/// shows only what the status would.
pub(in crate::screens::settings) fn preview(
    state: &DiscordPreview,
    view: &SettingsView,
    window: &Window,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let theme = *cx.theme();
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
    // a local track belongs to the local files, so the button it would have stood for is the
    // one of the service in use, and there is none to offer while no service is
    let listen_on = match local {
        true => session.provider_name(),
        false => provider.map(|(_, name, _)| name),
    };

    let listening: SharedString = match name {
        DiscordName::Sonora => SONORA.into(),
        DiscordName::Provider => provider
            .map(|(_, _, listening)| SharedString::from(listening))
            .unwrap_or_else(|| SONORA.into()),
        DiscordName::Music => t!("settings-discord-preview-music"),
        DiscordName::Title => sample.title.clone(),
        DiscordName::Artist => sample.artist.clone(),
        // an album with no artist named falls back the way the status itself does
        DiscordName::AlbumArtist => match sample.album_artist.trim().is_empty() {
            true => t!("settings-discord-preview-music"),
            false => sample.album_artist.clone(),
        },
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
    let badge = provider.map(|(slug, name, _)| Badge {
        slug,
        name,
        brand: provider_brand(slug, cx),
        on: badge,
        press: on.then(|| -> Press {
            Box::new(cx.listener(move |this, _, _, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.set_discord_badge(!badge, cx));
            }))
        }),
    });
    // the provider's button never goes out with hidden details or a local file, so the card
    // leaves it out for them, except that a local track picked for the card still offers it
    // while the card can be changed, as the card is the one place it is turned on and off
    let mut buttons = Vec::new();
    if !hidden
        && (!local || on)
        && let Some(name) = listen_on
    {
        buttons.push(Link {
            id: "discord-preview-provider-button",
            label: t!("settings-discord-preview-listen-on", provider = name),
            on: provider_button,
            press: on.then(|| -> Press {
                Box::new(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_discord_provider_button(!provider_button, cx)
                    });
                }))
            }),
        });
    }
    buttons.push(Link {
        id: "discord-preview-sonora-button",
        label: t!("get-sonora"),
        on: sonora_button,
        press: on.then(|| -> Press {
            Box::new(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_sonora_button(!sonora_button, cx)
                });
            }))
        }),
    });
    // with the status off the card shows only what the status would
    buttons.retain(|link| link.on || link.press.is_some());

    // the header names the status the way Discord does, cut short within the card, and opens
    // the menu of names while the card can be changed
    let header = ui::spoken(
        "discord-preview-header",
        t!("settings-discord-preview-listening", name = &listening),
    )
    .min_w_0()
    .text_size(HEADER_SIZE)
    .text_color(rgb(HEADER))
    .truncate();
    let header = match on {
        true => Popover::new(NAME_MENU, view.popovers.clone())
            .name(t!("settings-discord-name"))
            .face(
                div()
                    .id("discord-preview-name")
                    .flex()
                    .items_center()
                    .gap(NAME_INSET)
                    .min_w_0()
                    .mx(-NAME_INSET)
                    .px(NAME_INSET)
                    .rounded(BUTTON_RADIUS)
                    .hover(|style| style.bg(rgba(NAME_HOVER)))
                    .child(header)
                    .child(
                        svg()
                            .path(icons::path("icons/chevron-down.svg"))
                            .size(PLUS)
                            .flex_none()
                            .text_color(rgb(HEADER)),
                    ),
            )
            .menu(
                Menu::new("discord-name-menu")
                    .name(t!("settings-discord-name"))
                    .w(Picker::NARROW)
                    .left_0()
                    .items(DiscordName::ALL.map(|choice| {
                        MenuItem::new(choice.id(), i18n::lookup(choice.key(), None))
                            .selected(choice == name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.update(cx, |settings, cx| {
                                    settings.set_discord_name(choice, cx)
                                });
                                cx.notify();
                            }))
                    })),
            )
            .commands()
            .into_any_element(),
        false => header.into_any_element(),
    };

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
        .child(header)
        .child(
            div()
                .flex()
                .items_center()
                .gap(ART_GAP)
                .child(image(art, badge))
                .child(details(title, artist, album, timing)),
        )
        .when(!buttons.is_empty(), |card| {
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(BUTTON_GAP)
                    .children(buttons.into_iter().map(link)),
            )
        });

    super::framed(
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(CARD_PAD)
            .p(SURFACE_PAD)
            .bg(theme.background)
            .child(card.when(!on, |card| card.opacity(super::DIMMED)))
            .when(on, |surface| {
                surface.child(
                    ui::spoken("discord-preview-hint", t!("settings-discord-preview-hint"))
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground),
                )
            }),
        cx,
    )
    .into_any_element()
}

/// The provider's badge on the large image: the provider by slug and name, whether the status
/// carries it, and what a click on it does while the card can be changed.
struct Badge {
    slug: &'static str,
    name: &'static str,
    /// The provider's own colour, which the badge's disc is filled with.
    brand: Hsla,
    on: bool,
    press: Option<Press>,
}

/// One button under the status: its id, its label, whether the status carries it, and what a
/// click on it does while the card can be changed.
struct Link {
    id: &'static str,
    label: SharedString,
    on: bool,
    press: Option<Press>,
}

/// A button under the status the way Discord draws it while `on`, or a dashed outline with a
/// plus while off. One that can be changed is a toggle button to screen readers, and a plain
/// button otherwise.
fn link(link: Link) -> impl IntoElement {
    let Link {
        id,
        label,
        on,
        press,
    } = link;
    let editable = press.is_some();

    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .when(editable, |this| {
            this.aria_toggled(gpui::accesskit::Toggled::from(on))
        })
        .h(BUTTON_HEIGHT)
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded(BUTTON_RADIUS)
        .text_size(BUTTON_SIZE)
        .font_weight(FontWeight::MEDIUM)
        .map(|this| match on {
            true => this.bg(rgb(BUTTON)).text_color(rgb(BUTTON_TEXT)),
            false => this
                .border_1()
                .border_dashed()
                .border_color(rgba(GHOST_EDGE))
                .text_color(rgba(GHOST_TEXT))
                .child(
                    svg()
                        .path(icons::path("icons/plus.svg"))
                        .size(PLUS)
                        .flex_none()
                        .text_color(rgba(GHOST_TEXT)),
                ),
        })
        .child(label)
        .when_some(press, |this, press| {
            this.cursor_pointer()
                .on_click(move |event, window, cx| press(event, window, cx))
        })
        .glowed(match on {
            true => rgb(BUTTON_HOVER).into(),
            false => rgb(BUTTON).into(),
        })
}

/// The large image with the provider's badge at its bottom right, or Sonora's application
/// icon when the status carries no art, as Discord falls back to it. A badge the status leaves
/// out is drawn as a dashed circle with a plus while it can be changed, and not at all
/// otherwise.
fn image(art: Option<String>, badge: Option<Badge>) -> Div {
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
    let badge = badge.filter(|badge| badge.on || badge.press.is_some());

    div()
        .relative()
        .flex_none()
        .size(ART)
        .child(picture)
        .when_some(badge, |this, badge| {
            let Badge {
                slug,
                name,
                brand,
                on,
                press,
            } = badge;
            let editable = press.is_some();
            let outer = BADGE_SIZE + BADGE_RING * 2.;
            let disc = match on {
                true => div()
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
                false => div()
                    .size(BADGE_SIZE)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(rgba(GHOST_EDGE))
                    .child(
                        svg()
                            .path(icons::path("icons/plus.svg"))
                            .size(PLUS)
                            .text_color(rgba(GHOST_TEXT)),
                    ),
            };
            this.child(
                div()
                    .id("discord-preview-badge")
                    .map(|this| match editable {
                        true => this
                            .role(Role::Button)
                            .aria_label(t!("settings-discord-badge"))
                            .aria_description(name)
                            .aria_toggled(gpui::accesskit::Toggled::from(on)),
                        false => this.role(Role::Image).aria_label(name),
                    })
                    .absolute()
                    .right(-BADGE_OVERHANG)
                    .bottom(-BADGE_OVERHANG)
                    .size(outer)
                    .p(BADGE_RING)
                    .rounded_full()
                    .bg(rgb(CARD))
                    .child(disc)
                    .when_some(press, |this, press| {
                        this.cursor_pointer()
                            .on_click(move |event, window, cx| press(event, window, cx))
                    }),
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
