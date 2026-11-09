//! A replica of the activity card Discord shows in a profile for the status Sonora sets, in
//! Discord's own dark theme, sizes and type. It follows every setting on the page and draws the
//! status the way `state::discord` would send it for a track from listening history. Screen
//! readers hear it as a named group of its text lines, buttons and provider badge.
//!
//! The clock and the progress bar count on through live values a ticker thread sets while the
//! card is on screen, so the view never runs for them.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use i18n::t;
use rune::{Color, Cx, El, Live, LiveText, TextStyle, Transition, Tx, col, el, icon, row, text};
use state::DiscordName;
use ui::{Cover, Hsla, TABULAR, Weight};

use super::{DIMMED, Ticker, framed, picked};
use crate::Ui;
use crate::screens::settings::Layout;
use crate::screens::settings::accounts::{provider_brand, provider_logo};

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
const CARD_WIDTH: f32 = 372.;
const CARD_PAD: f32 = 12.;
const CARD_RADIUS: f32 = 8.;
/// How much room the popout leaves around the card.
const SURFACE_PAD: f32 = 20.;
/// The large image, its corners and the gap to the text beside it.
const ART: f32 = 120.;
const ART_RADIUS: f32 = 8.;
const ART_GAP: f32 = 12.;
/// The provider badge, the ring of card colour around it, and how far it hangs past the art.
const BADGE_SIZE: f32 = 32.;
const BADGE_RING: f32 = 4.;
const BADGE_OVERHANG: f32 = 4.;
/// Sonora's mark on the application icon, as a share of the icon.
const APP_MARK_SHARE: f32 = 0.56;
/// A provider's mark on the badge, as a share of the badge.
const BADGE_MARK_SHARE: f32 = 0.5;
/// Discord's type sizes: the header, the title, the artist and album lines, the times and the
/// buttons.
const HEADER_SIZE: f32 = 14.;
const TITLE_SIZE: f32 = 16.;
const TEXT_SIZE: f32 = 14.;
const TIME_SIZE: f32 = 14.;
const BUTTON_SIZE: f32 = 14.;
/// The progress bar's height, and the room between it and the times either side.
const BAR_HEIGHT: f32 = 4.;
const BAR_GAP: f32 = 12.;
/// The room between the album line and the times under it.
const TIME_GAP: f32 = 8.;
/// A button's height and corners, and the gap between two.
const BUTTON_HEIGHT: f32 = 32.;
const BUTTON_RADIUS: f32 = 8.;
const BUTTON_GAP: f32 = 8.;
/// How far through the track the card starts, as a share of its length, before it counts on.
const START_SHARE: f32 = 0.3;
/// What the status calls itself when the setting names Sonora, Discord's name for the app.
const SONORA: &str = "Sonora";
/// How often the clock is read while the card counts on. The labels only change once a second,
/// and an unchanged one wakes nothing.
const TICK: Duration = Duration::from_millis(250);

/// What the status shows about time: a bar to the end of the track, the time since listening
/// started when the details are hidden, or nothing while paused.
#[derive(Clone, Copy, PartialEq)]
enum Timing {
    Bar { total: Duration },
    Elapsed,
    Still,
}

/// What the Discord page's preview keeps: when the card started counting, the live labels and
/// bar it counts on, and the thread that moves them while the card is on screen.
pub(in crate::screens::settings) struct State {
    clock: Arc<Clock>,
    ticker: Ticker,
}

/// What the ticker thread reads and writes.
struct Clock {
    /// When the card started counting, which settings opening last set.
    since: Mutex<Instant>,
    /// What the card shows of time, which the view keeps up to date.
    timing: Mutex<Timing>,
    elapsed: LiveText,
    total: LiveText,
    /// The bar's fill, in the track's own points, and the track's width as `f32` bits.
    fill: Live<[f32; 4]>,
    width: AtomicU32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            clock: Arc::new(Clock {
                since: Mutex::new(Instant::now()),
                timing: Mutex::new(Timing::Still),
                elapsed: LiveText::new("00:00"),
                total: LiveText::new("00:00"),
                fill: Live::new([0., 0., 0., BAR_HEIGHT]),
                width: AtomicU32::new(0),
            }),
            ticker: Ticker::default(),
        }
    }
}

impl State {
    /// Settings opened, so the card counts from a new start.
    pub(super) fn opened(&self) {
        *self
            .clock
            .since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Stops the clock, for a card that left the screen.
    pub(in crate::screens::settings) fn hide(&mut self) {
        self.ticker.stop();
    }

    fn show(&mut self) {
        let clock = self.clock.clone();
        clock.tick();
        self.ticker.start(TICK, move || clock.tick());
    }
}

impl Clock {
    /// Brings the labels and the bar up to the time now.
    fn tick(&self) {
        let elapsed = self
            .since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed();
        let timing = *self.timing.lock().unwrap_or_else(PoisonError::into_inner);
        match timing {
            Timing::Bar { total } => {
                let at = position(total, elapsed);
                self.elapsed.set(elapsed_clock(at));
                self.total.set(elapsed_clock(total));
                let width = f32::from_bits(self.width.load(Ordering::Relaxed));
                let share = at.as_secs_f32() / total.as_secs_f32().max(1.);
                self.fill.set([0., 0., (width * share).round(), BAR_HEIGHT]);
            }
            Timing::Elapsed => {
                self.elapsed.set(
                    t!(
                        "settings-discord-preview-elapsed",
                        time = elapsed_clock(elapsed)
                    )
                    .into_owned(),
                );
            }
            Timing::Still => {}
        }
    }
}

/// The Discord page's preview, the card Discord shows for the status Sonora sets. It shows a
/// track picked from listening history, playing while Sonora plays or has nothing loaded and
/// paused while Sonora is paused, and dims only while Discord presence is disabled.
pub(in crate::screens::settings) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layout: &Layout) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings.previews.discord;
    let sample = picked(ui);
    let settings = &ui.store.settings;
    let playback = &ui.store.playback;
    let paused = playback.track().is_some() && !playback.wants_playing();
    let on = settings.discord_presence();
    let hidden = settings.discord_without_details();
    let local_art = settings.artwork_for_local_files();

    let session = &ui.store.session;
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

    let listening = match settings.discord_name() {
        DiscordName::Sonora => SONORA.to_owned(),
        DiscordName::Provider => provider
            .map(|(_, _, listening)| listening.to_owned())
            .unwrap_or_else(|| SONORA.to_owned()),
        DiscordName::Music => t!("settings-discord-preview-music").into_owned(),
        DiscordName::Title => sample.title.clone(),
        DiscordName::Artist => sample.artist.clone(),
        DiscordName::ArtistTitle => format!("{} - {}", sample.artist, sample.title),
    };
    let timing = match (paused, hidden) {
        (true, _) => Timing::Still,
        (false, true) => Timing::Elapsed,
        (false, false) => Timing::Bar {
            total: sample.duration.max(Duration::from_secs(1)),
        },
    };
    *state
        .clock
        .timing
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = timing;
    state.clock.tick();
    let art = match hidden || local && !local_art {
        true => None,
        false => sample.large_cover(&ui.store),
    };
    let badge = settings
        .discord_badge()
        .then_some(provider)
        .flatten()
        .map(|(slug, name, _)| (slug, name));
    let mut buttons = Vec::new();
    if settings.discord_provider_button()
        && !hidden
        && !local
        && let Some((_, name, _)) = provider
    {
        buttons.push(t!("settings-discord-preview-listen-on", provider = name).into_owned());
    }
    if settings.discord_sonora_button() {
        buttons.push(t!("get-sonora").into_owned());
    }
    let (title, artist, album) = match hidden {
        true => (t!("discord-listening").into_owned(), None, None),
        false => (
            sample.title.clone(),
            Some(sample.artist.clone()),
            (!sample.album.is_empty()).then(|| sample.album.clone()),
        ),
    };

    let header = t!(
        "settings-discord-preview-listening",
        name = listening.as_str()
    )
    .into_owned();
    let mut card = col()
        .key("discord-preview")
        .role("group")
        .label(t!("settings-discord-preview"))
        .w(CARD_WIDTH)
        .padding(CARD_PAD)
        .gap(CARD_PAD)
        .radius(CARD_RADIUS)
        .fill(hex(CARD))
        .opacity(match on {
            true => 1.,
            false => DIMMED,
        })
        .motion(ui::Springs::BASE)
        .child(
            spoken(
                "discord-preview-header",
                header,
                HEADER_SIZE,
                Weight::NORMAL,
                HEADER,
            )
            .truncate(),
        )
        .child(
            row()
                .center_y()
                .gap(ART_GAP)
                .child(image(ui, art, badge))
                .child(details(state, title, artist, album, timing)),
        );
    if !buttons.is_empty() {
        let count = buttons.len();
        card = card.child(
            col()
                .key("discord-preview-buttons")
                .gap(BUTTON_GAP)
                .children(buttons.into_iter().enumerate().map(|(index, label)| {
                    row()
                        .key(format!("discord-preview-button-{index}"))
                        .role("button")
                        .label(label.clone())
                        .position_in_set(index + 1, count)
                        .h(BUTTON_HEIGHT)
                        .w_full()
                        .center()
                        .radius(BUTTON_RADIUS)
                        .fill(hex(BUTTON))
                        .hover(hex(BUTTON_HOVER))
                        .press_scale(ui::PRESS_SCALE)
                        .transition(Transition::offset(0., -6.).combined(Transition::opacity()))
                        .child(
                            text(label)
                                .font(TextStyle::new(BUTTON_SIZE, Weight::MEDIUM))
                                .color(hex(BUTTON_TEXT))
                                .a11y_hidden(),
                        )
                })),
        );
    }

    framed(
        "discord-preview-frame",
        row()
            .key("discord-surface")
            .w_full()
            .justify_center()
            .padding(SURFACE_PAD)
            .fill(theme.background.color())
            .on_visible(|ui: &mut Ui, shown: bool, _: &mut Tx| {
                let state = &mut ui.screens.settings.previews.discord;
                match shown {
                    true => state.show(),
                    false => state.hide(),
                }
            })
            .child(card),
        theme,
    )
}

/// The large image with the provider's badge at its bottom right, or Sonora's application icon
/// when the status carries no art, as Discord falls back to it. The badge, given as the
/// provider's slug and name, reads to screen readers as an image named after the provider.
fn image(ui: &Ui, art: Option<String>, badge: Option<(&'static str, &'static str)>) -> El {
    let theme = &ui.theme;
    let picture = match art {
        Some(url) => Cover::new(ui.artwork(), Some(url.as_str()))
            .size(ART)
            .radius(ART_RADIUS)
            .build(theme),
        None => row()
            .size(ART, ART)
            .center()
            .radius(ART_RADIUS)
            .fill(hex(APP_ICON))
            .child(icon(icons::get("icons/sonora.svg"), ART * APP_MARK_SHARE).color(hex(APP_MARK))),
    };
    let mut frame = el()
        .key("discord-preview-image")
        .size(ART, ART)
        .child(picture.key("picture").transition(Transition::opacity()));
    if let Some((slug, name)) = badge {
        let brand: Hsla = provider_brand(slug, theme);
        let outer = BADGE_SIZE + BADGE_RING * 2.;
        frame = frame.child(
            row()
                .key("discord-preview-badge")
                .role("img")
                .label(name)
                .pin(None, None, Some(-BADGE_OVERHANG), Some(-BADGE_OVERHANG))
                .size(outer, outer)
                .center()
                .radius(outer / 2.)
                .fill(hex(CARD))
                .transition(Transition::scale(0.6).combined(Transition::opacity()))
                .child(
                    row()
                        .size(BADGE_SIZE, BADGE_SIZE)
                        .center()
                        .radius(BADGE_SIZE / 2.)
                        .fill(brand.color())
                        .child(
                            icon(
                                icons::get(provider_logo(slug)),
                                BADGE_SIZE * BADGE_MARK_SHARE,
                            )
                            .icon_one_color()
                            .color(hex(BADGE_MARK))
                            .a11y_hidden(),
                        ),
                ),
        );
    }
    frame
}

/// The text beside the image: the title, the artist, the album, and the time the status shows.
fn details(
    state: &State,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    timing: Timing,
) -> El {
    let clock = &state.clock;
    let time = |key: &str, live: &LiveText| {
        text(live.get().to_string())
            .key(key)
            .role("text")
            .label(live.get().to_string())
            .live_text(live)
            .font(TextStyle::new(TIME_SIZE, Weight::NORMAL).features(TABULAR))
            .color(hex(MUTED))
    };
    let mut words = col()
        .key("discord-preview-details")
        .grow(1.)
        .shrink()
        .min_w(0.)
        .child(
            spoken(
                "discord-preview-title",
                title,
                TITLE_SIZE,
                Weight::SEMIBOLD,
                TITLE,
            )
            .truncate(),
        );
    if let Some(artist) = artist {
        words = words.child(
            spoken(
                "discord-preview-artist",
                artist,
                TEXT_SIZE,
                Weight::NORMAL,
                TEXT,
            )
            .truncate(),
        );
    }
    if let Some(album) = album {
        words = words.child(
            spoken(
                "discord-preview-album",
                album,
                TEXT_SIZE,
                Weight::NORMAL,
                TEXT,
            )
            .truncate(),
        );
    }
    match timing {
        Timing::Bar { .. } => words.child(
            row()
                .key("discord-preview-timing")
                .mt(TIME_GAP)
                .center_y()
                .gap(BAR_GAP)
                .transition(Transition::opacity())
                .child(time("discord-preview-elapsed", &clock.elapsed))
                .child(
                    el().key("discord-preview-bar")
                        .grow(1.)
                        .h(BAR_HEIGHT)
                        .radius(BAR_HEIGHT / 2.)
                        .fill(hex(BAR))
                        .on_layout_tx(|ui: &mut Ui, rect: [f32; 4], tx: &mut Tx| {
                            let clock = &ui.screens.settings.previews.discord.clock;
                            clock.width.store(rect[2].to_bits(), Ordering::Relaxed);
                            clock.tick();
                            tx.live_only();
                        })
                        .child(
                            el().key("fill")
                                .absolute(0., 0.)
                                .h(BAR_HEIGHT)
                                .w(0.)
                                .radius(BAR_HEIGHT / 2.)
                                .fill(hex(BAR_FILL))
                                .live_rect(&clock.fill),
                        ),
                )
                .child(time("discord-preview-total", &clock.total)),
        ),
        Timing::Elapsed => words.child(
            time("discord-preview-elapsed", &clock.elapsed)
                .mt(4.)
                .transition(Transition::opacity()),
        ),
        Timing::Still => words,
    }
}

/// A line of the card in Discord's own size, weight and colour, which screen readers reach.
fn spoken(key: &str, label: String, size: f32, weight: f32, ink: u32) -> El {
    text(label.clone())
        .key(key)
        .role("text")
        .label(label)
        .font(TextStyle::new(size, weight))
        .color(hex(ink))
}

/// Where the card's clock stands `elapsed` after it started, looping round a track `total`
/// long from `START_SHARE` of the way in.
fn position(total: Duration, elapsed: Duration) -> Duration {
    let start = total.mul_f32(START_SHARE);
    Duration::from_secs((start + elapsed).as_secs() % total.as_secs().max(1))
}

/// Time since listening started the way Discord counts it, minutes and seconds both padded.
fn elapsed_clock(value: Duration) -> String {
    let total = value.as_secs();
    match total / 3600 {
        0 => format!("{:02}:{:02}", total / 60, total % 60),
        hours => format!("{hours:02}:{:02}:{:02}", total % 3600 / 60, total % 60),
    }
}

/// One of Discord's own colours, which no theme changes.
fn hex(value: u32) -> Color {
    Hsla::from(ui::rgb(value)).color()
}
