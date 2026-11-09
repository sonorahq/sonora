//! The Playback page: how tracks play (loudness, gapless, the sleep timer, staying awake), the
//! level a service streams at, what a server converts tracks into, and the Widevine module a
//! protected service needs. The Sessions and Spotify Connect groups that follow on the page are
//! drawn by their own files.

use std::time::Duration;

use i18n::t;
use music::drm::Origin;
use music::{Codec, Encoding, Offer, Quality, Transcode};
use rune::{Cx, El, Transition, Tx, col, el, row, text};
use state::{CdmState, Sleep};
use ui::{
    Button, MenuItem, OptionCard, Picker, Scrubber, ScrubberState, Text, Theme, Weight, font,
};

use super::{Layout, Row, columns, grid, group, segmented, switch};
use crate::shared::confirm::{self, Kind};
use crate::{Ui, act};

/// How many quality cards a row is laid out for, however few levels a service has.
const QUALITY_COLUMNS: usize = 4;
const STREAM_BITRATES: &str = "stream-bitrates";
/// How wide the sleep slider and the time beside it stand, in covers.
const SLEEP_WIDTH: f32 = 1.;
const SLEEP_LABEL: f32 = 0.7;
/// The shortest and longest timer the slider sets, in minutes.
const SLEEP_MIN_MINUTES: u64 = 1;
const SLEEP_MAX_MINUTES: u64 = 120;
const SLEEP_MAGNETS: [u64; 4] = [15, 30, 45, 60];
const SLEEP_MAGNET_WEIGHT: usize = 4;
/// The slot at the far right of the slider, end of track. Off is slot 0 and every minute
/// between takes one slot, or `SLEEP_MAGNET_WEIGHT` for a magnet.
const SLEEP_LAST: usize = (SLEEP_MAX_MINUTES - SLEEP_MIN_MINUTES + 1) as usize
    + SLEEP_MAGNETS.len() * (SLEEP_MAGNET_WEIGHT - 1)
    + 1;
/// How strongly the empty part of a slider's track shows against the muted text.
const TRACK_EMPTY: f32 = 0.3;
/// How tall a quality card's signal bars stand at their lowest and how much they rise to the
/// tallest, as shares of the thumbnail.
const BAR_LOW: f32 = 0.2;
const BAR_RISE: f32 = 0.5;
/// How tall an option card's thumbnail stands, in list rows, which the bars are a share of.
const THUMB: f32 = 1.5;
/// How long each bar of a quality card waits after the one before it to light, in seconds.
const BAR_STAGGER: f32 = 0.03;

/// What the Playback page keeps between frames: the sleep slider and where a drag of it points.
#[derive(Default)]
pub struct State {
    pub sleep: ScrubberState,
    /// The timer a drag of the sleep slider points at, until it lets go.
    pending_sleep: Option<Option<Sleep>>,
}

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.settings.playback
}

/// The Playback page's own groups. One quality level is nothing to choose between, so it is
/// stated in a row rather than offered as cards.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let store = &ui.store;
    let capabilities = store.session.capabilities();
    let playback = &store.playback;
    let mut groups = vec![group(
        theme,
        Some(("settings-group-playing", "icons/play.svg")),
        None,
        [
            switch(
                theme,
                "normalisation",
                t!("settings-normalisation"),
                playback.normalisation(),
                |ui, on| ui.store.set_normalisation(on),
            ),
            switch(
                theme,
                "gapless",
                t!("settings-gapless"),
                playback.gapless(),
                |ui, on| ui.store.set_gapless(on),
            ),
            sleep_row(ui),
            switch(
                theme,
                "stay-awake",
                t!("settings-stay-awake"),
                store.settings.stay_awake(),
                |ui, on| ui.store.settings.set_stay_awake(on, &mut ui.store.cx),
            ),
        ],
    )];
    let quality = Some(("settings-stream-quality", "icons/disc-3.svg"));
    match capabilities.qualities.len() {
        0 => {}
        1 => groups.push(group(theme, quality, None, [stream_quality_row(ui)])),
        _ => groups.push(group(theme, quality, Some(quality_cards(ui, layer, layout)), [])),
    }
    if capabilities.transcode {
        groups.push(group(
            theme,
            Some(("settings-group-streaming", "icons/refresh-cw.svg")),
            None,
            [stream_format_row(ui), stream_bitrate_row(ui, layer)],
        ));
    }
    if store.drm.shown(&store.session) {
        groups.push(group(
            theme,
            Some(("settings-group-protected", "icons/lock.svg")),
            None,
            [widevine_row(ui)],
        ));
    }
    groups
}

/// The detail of the Widevine row for its state, which a search reads.
#[allow(dead_code)]
pub(super) fn widevine_detail(ui: &Ui) -> String {
    let (detail, _) = widevine_copy(ui.store.drm.state());
    i18n::lookup(detail, None).into_owned()
}

/// The stream level of a service that streams every track at one level, stated rather than
/// offered since there is nothing to choose between.
fn stream_quality_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let offers = ui.store.session.capabilities().qualities;
    let current = ui.store.playback.quality().within(offers);
    let encoding = offers
        .iter()
        .find(|offer| offer.quality == current)
        .map(encoding_label)
        .unwrap_or_default();
    Row::new(
        "stream-quality",
        i18n::lookup(quality_key(current), None),
        ui::spoken("settings-stream-quality-encoding", encoding, theme)
            .font(font(theme, Text::Label, Weight::NORMAL))
            .color(theme.muted_foreground.color()),
    )
    .build(theme)
}

/// One card per level the signed-in service streams at, lowest first, each saying what it
/// streams as. A saved level the service lacks shows as the one it plays instead.
fn quality_cards(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let offers = ui.store.session.capabilities().qualities;
    let current = ui.store.playback.quality().within(offers);
    let mut ladder = offers.to_vec();
    ladder.sort_by_key(|offer| offer.quality);
    let steps = ladder.len();
    let cards: Vec<El> = ladder
        .into_iter()
        .enumerate()
        .map(|(rank, offer)| {
            let quality = offer.quality;
            OptionCard::new(format!("quality-{}", quality.id()))
                .thumb(signal(theme, rank, steps))
                .label(i18n::lookup(quality_key(quality), None))
                .detail(encoding_label(&offer))
                .selected(quality == current)
                .on_press(act(move |ui: &mut Ui| ui.store.set_quality(quality)))
                .build(theme, layer)
        })
        .collect();
    // a short ladder keeps the width of a longer one rather than stretching across
    col().key("settings-row-stream-quality").w_full().child(grid(
        "quality",
        columns(steps.max(QUALITY_COLUMNS), layout.room()),
        cards,
    ))
}

/// The format a server converts tracks into, or the one it sends, as one segmented control.
fn stream_format_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let transcode = ui.store.playback.transcode();
    let formats = segmented(
        theme,
        "stream-formats",
        t!("settings-stream-format"),
        std::iter::once(None)
            .chain(Codec::ALL.map(Some))
            .map(|codec| (codec, i18n::lookup(codec_key(codec), None).into_owned())),
        transcode.codec,
        move |ui, codec| ui.store.set_transcode(Transcode { codec, ..transcode }),
    );
    Row::new("stream-format", t!("settings-stream-format"), formats).build(theme)
}

/// The highest bitrate a server sends, with about how much an hour takes under each.
fn stream_bitrate_row(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let transcode = ui.store.playback.transcode();
    let picker = Picker::new(
        STREAM_BITRATES,
        &ui.screens.settings.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        bitrate_label(transcode.bitrate),
    )
    .name(t!("settings-stream-bitrate"))
    .width(Picker::<Ui>::REGULAR)
    .items(
        std::iter::once(None)
            .chain(Transcode::BITRATES.map(Some))
            .map(|bitrate| {
                let id = bitrate.map_or_else(|| "unlimited".to_owned(), |kbps| kbps.to_string());
                MenuItem::new(format!("bitrate-{id}"), bitrate_label(bitrate))
                    .selected(bitrate == transcode.bitrate)
                    .detail(
                        text(bitrate_detail(bitrate))
                            .font(font(theme, Text::Small, Weight::NORMAL))
                            .color(theme.muted_foreground.color()),
                    )
                    .on_press(act(move |ui: &mut Ui| {
                        ui.store.set_transcode(Transcode {
                            bitrate,
                            ..transcode
                        })
                    }))
            }),
    )
    .build(theme, layer);
    Row::new("stream-bitrate", t!("settings-stream-bitrate"), picker).build(theme)
}

/// The sleep timer as a slider in its row with the time it is set to before it: off at the
/// left, end of track at the right, minutes in between with the quarter hours widened. The
/// label follows a drag at once, and the timer is armed when the drag lets go.
fn sleep_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings.playback;
    let current = state
        .pending_sleep
        .unwrap_or_else(|| ui.store.playback.sleep());
    let label = sleep_label(current);
    let control = row()
        .center_y()
        .gap(theme.font_size * 0.75)
        .child(
            ui::spoken("settings-sleep-value", label.clone(), theme)
                .w(theme.metrics.cover * SLEEP_LABEL)
                .align(rune::Align::End)
                .color(match current {
                    Some(_) => theme.foreground.color(),
                    None => theme.muted_foreground.color(),
                }),
        )
        .child(
            col()
                .key("settings-sleep-slider")
                .w(theme.metrics.cover * SLEEP_WIDTH)
                .child(
                    Scrubber::new(
                        "sleep",
                        &state.sleep,
                        sleep_slot(current) as f32 / SLEEP_LAST as f32,
                    )
                    .name(t!("settings-sleep"))
                    .value(label)
                    .colors(
                        theme.progress_bar,
                        theme.muted_foreground.opacity(TRACK_EMPTY),
                        theme.foreground,
                    )
                    .on_move(|ui: &mut Ui, fraction: f32, _: &mut Tx| {
                        st(ui).pending_sleep = Some(sleep_at_fraction(fraction));
                    })
                    .on_release(|ui: &mut Ui, _: &mut Tx| {
                        let Some(sleep) = st(ui).pending_sleep.take() else {
                            return;
                        };
                        ui.store.set_sleep(sleep);
                        ui.settle();
                    })
                    .build(theme),
                ),
        );
    Row::new("sleep", t!("settings-sleep"), control).build(theme)
}

/// The Widevine module row, which only appears while the current provider is one whose tracks
/// need the module and this build has a host for one. Sonora uses a browser's copy when one is
/// here and otherwise offers Google's download, so the row says where that stands. The
/// download is offered by hand whenever Google's copy is not the one in use, because a
/// browser's copy can be one this host cannot open. While the download is on offer the
/// explanation wraps below the row, since pressing Download is the consent to fetching
/// Google's module.
fn widevine_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = ui.store.drm.state().clone();
    let (detail, note) = widevine_copy(&state);
    let offerable = matches!(
        state,
        CdmState::Declined | CdmState::Missing | CdmState::Ready(Origin::Installed)
    );
    let removable = matches!(state, CdmState::Ready(Origin::Fetched));
    let busy = matches!(state, CdmState::Offering | CdmState::Installing | CdmState::Looking);
    let detail = i18n::lookup(detail, None).into_owned();

    let mut action = row()
        .center_y()
        .gap(theme.font_size * 0.5)
        .child(
            ui::spoken("settings-widevine-note", i18n::lookup(note, None), theme)
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .transition(Transition::opacity()),
        );
    if busy {
        action = action.child(
            ui::Spinner::new()
                .size(theme.text(Text::Label))
                .build(theme)
                .transition(Transition::opacity()),
        );
    }
    if offerable {
        action = action.child(
            Button::new("fetch-widevine")
                .label(t!("settings-widevine-fetch"))
                .small()
                .outline()
                .on_press(act(|ui: &mut Ui| ui.store.drm.download(&mut ui.store.cx)))
                .build(theme),
        );
    }
    if removable {
        action = action.child(
            Button::new("uninstall-widevine")
                .label(t!("settings-widevine-uninstall"))
                .small()
                .ghost()
                .on_press(act(|ui: &mut Ui| {
                    confirm::ask(ui, Kind::Widevine, |ui: &mut Ui| {
                        ui.store.drm.uninstall(&mut ui.store.cx)
                    })
                }))
                .build(theme),
        );
    }
    let mut block = col()
        .key("settings-widevine")
        .w_full()
        .child(Row::new("widevine", t!("settings-widevine"), action).build(theme));
    if offerable {
        block = block.child(
            text(detail.clone())
                .key("settings-widevine-detail")
                .role("paragraph")
                .label(detail)
                .wrap()
                .pb(theme.font_size * 1.25)
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .transition(Transition::opacity()),
        );
    }
    block
}

/// What the Widevine row says: the detail key for its state, and the note beside it.
pub(super) fn widevine_copy(state: &CdmState) -> (&'static str, &'static str) {
    match state {
        CdmState::Looking => ("settings-widevine-detail", "settings-widevine-looking"),
        CdmState::Ready(Origin::Configured) => {
            ("settings-widevine-detail", "settings-widevine-configured")
        }
        CdmState::Ready(Origin::Installed) => {
            ("settings-widevine-detail", "settings-widevine-installed")
        }
        CdmState::Ready(Origin::Fetched) => {
            ("settings-widevine-detail", "settings-widevine-fetched")
        }
        CdmState::Wanted | CdmState::Offered(_) => {
            ("settings-widevine-none", "settings-widevine-asking")
        }
        CdmState::Offering => ("settings-widevine-none", "settings-widevine-fetching"),
        CdmState::Installing => ("settings-widevine-none", "settings-widevine-installing"),
        CdmState::Declined | CdmState::Missing => {
            ("settings-widevine-none", "settings-widevine-missing")
        }
    }
}

fn sleep_slot(sleep: Option<Sleep>) -> usize {
    match sleep {
        None => 0,
        Some(Sleep::EndOfTrack) => SLEEP_LAST,
        Some(Sleep::After(after)) => minute_slot(after.as_secs() / 60),
    }
}

fn sleep_at_fraction(fraction: f32) -> Option<Sleep> {
    let slot = (fraction.clamp(0., 1.) * SLEEP_LAST as f32).round() as usize;
    match slot {
        0 => None,
        SLEEP_LAST => Some(Sleep::EndOfTrack),
        slot => Some(Sleep::After(Duration::from_secs(slot_minute(slot) * 60))),
    }
}

fn minute_slot(minutes: u64) -> usize {
    let minutes = minutes.clamp(SLEEP_MIN_MINUTES, SLEEP_MAX_MINUTES);
    let earlier_magnets = SLEEP_MAGNETS
        .iter()
        .filter(|magnet| **magnet < minutes)
        .count();
    let width = match SLEEP_MAGNETS.contains(&minutes) {
        true => SLEEP_MAGNET_WEIGHT,
        false => 1,
    };
    (minutes - SLEEP_MIN_MINUTES + 1) as usize
        + earlier_magnets * (SLEEP_MAGNET_WEIGHT - 1)
        + (width - 1) / 2
}

fn slot_minute(slot: usize) -> u64 {
    let mut first = 1;
    for minute in SLEEP_MIN_MINUTES..=SLEEP_MAX_MINUTES {
        let width = match SLEEP_MAGNETS.contains(&minute) {
            true => SLEEP_MAGNET_WEIGHT,
            false => 1,
        };
        if slot < first + width {
            return minute;
        }
        first += width;
    }
    SLEEP_MAX_MINUTES
}

fn sleep_label(sleep: Option<Sleep>) -> String {
    match sleep {
        Some(Sleep::EndOfTrack) => t!("settings-sleep-end-of-track"),
        Some(Sleep::After(after)) => t!("settings-sleep-minutes", count = after.as_secs() / 60),
        None => t!("settings-sleep-off"),
    }
    .into_owned()
}

/// The Fluent key naming a stream level.
pub(super) fn quality_key(quality: Quality) -> &'static str {
    match quality {
        Quality::HiRes => "settings-stream-quality-hi-res",
        Quality::Lossless => "settings-stream-quality-lossless",
        Quality::High => "settings-stream-quality-high",
        Quality::Normal => "settings-stream-quality-normal",
        Quality::Low => "settings-stream-quality-low",
    }
}

/// What a level streams as, such as "FLAC, up to 24-bit 192 kHz" or "AAC, 320 kbps".
pub(super) fn encoding_label(offer: &Offer) -> String {
    let lossy = |codec: &str, kbps: u32| match offer.up_to {
        true => t!(
            "settings-stream-quality-lossy-up-to",
            codec = codec.to_owned(),
            kbps = i64::from(kbps)
        ),
        false => t!(
            "settings-stream-quality-lossy",
            codec = codec.to_owned(),
            kbps = i64::from(kbps)
        ),
    };
    match offer.encoding {
        Encoding::Flac { depth, rate } => {
            // 44100 reads as 44.1 and 192000 as 192
            let khz = match rate % 1000 {
                0 => (rate / 1000).to_string(),
                _ => format!("{:.1}", f64::from(rate) / 1000.0),
            };
            match offer.up_to {
                true => t!(
                    "settings-stream-quality-flac-up-to",
                    depth = i64::from(depth),
                    rate = khz
                ),
                false => t!(
                    "settings-stream-quality-flac",
                    depth = i64::from(depth),
                    rate = khz
                ),
            }
        }
        Encoding::Aac { kbps } => lossy("AAC", kbps),
        Encoding::Vorbis { kbps } => lossy("Ogg Vorbis", kbps),
        Encoding::Mp3 { kbps } => lossy("MP3", kbps),
        Encoding::Opus { kbps } => lossy("Opus", kbps),
        // No service offers these. They name a local file's encoding in the signal path.
        Encoding::Alac { .. } => t!("signal-path-codec-alac"),
        Encoding::Pcm { .. } => t!("signal-path-codec-pcm"),
        Encoding::Other => t!("signal-path-codec-other"),
    }
    .into_owned()
}

/// The Fluent key naming a stream format, where `None` keeps the format the server sends.
pub(super) fn codec_key(codec: Option<Codec>) -> &'static str {
    match codec {
        None => "settings-stream-format-original",
        Some(Codec::Mp3) => "settings-stream-format-mp3",
        Some(Codec::Opus) => "settings-stream-format-opus",
        Some(Codec::Aac) => "settings-stream-format-aac",
    }
}

pub(super) fn bitrate_label(bitrate: Option<u32>) -> String {
    match bitrate {
        Some(kbps) => t!("settings-stream-bitrate-value", kbps = i64::from(kbps)),
        None => t!("settings-stream-bitrate-unlimited"),
    }
    .into_owned()
}

/// About how much data an hour at a bitrate ceiling takes, said under it in the picker.
pub(super) fn bitrate_detail(bitrate: Option<u32>) -> String {
    match bitrate {
        // kilobits a second over an hour, in megabytes
        Some(kbps) => t!(
            "settings-stream-bitrate-hourly",
            megabytes = i64::from(kbps * 3600 / 8 / 1000)
        ),
        None => t!("settings-stream-bitrate-unlimited-detail"),
    }
    .into_owned()
}

/// A rising row of `steps` bars with the first `rank + 1` lit, the way a quality card shows how
/// far up the ladder its level stands. The bars grow up from the bottom, one after another, as
/// the card first shows.
fn signal(theme: &Theme, rank: usize, steps: usize) -> El {
    let pad = theme.metrics.pad;
    // the card's thumbnail stands one and a half list rows tall unless told otherwise
    let height = theme.metrics.list_row * THUMB;
    row()
        .w_full()
        .h_full()
        .items_end()
        .justify_center()
        .gap(pad / 2.)
        .pb(pad * 1.5)
        .children((0..steps).map(|step| {
            let share = BAR_LOW + BAR_RISE * (step + 1) as f32 / steps as f32;
            col().w(pad).h_full().justify_end().child(
                el().key(format!("bar-{step}"))
                    .w(pad)
                    .h((height * share).round())
                    .radius(pad / 4.)
                    .motion(ui::Springs::BASE)
                    .fill(match step <= rank {
                        true => theme.foreground.color(),
                        false => theme.muted.color(),
                    })
                    .transition(Transition::offset(0., pad).combined(Transition::opacity()))
                    .enter_delay(BAR_STAGGER * step as f32),
            )
        }))
}
