use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    Context, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, StyledText, Window,
    div,
};
use i18n::t;
use music::{Encoding, TrackSignal};
use router::{Destination, Link, navigate};
use state::{Playback, PlayerBarLayout, QueuePosition, Sonora};
use ui::{
    ActiveTheme as _, AudioQualityBadge, Badge, InlineLink, InlineLinks, Perch, Pinnable, Sparkle,
    Tails as _, Tipped as _,
};

use crate::shared::pins::Pinned as _;
use crate::shared::transport::like;

/// Opens the track menu of the view that draws the details at the pointer's window position.
type OpenMenu<V> = fn(&mut V, Point<Pixels>, &mut Window, &mut Context<V>);

/// Creates the player's format badge from the encoding and sample rate the engine reported.
pub(crate) fn audio_quality_badge(
    signal: Option<&TrackSignal>,
    id: &'static str,
) -> Option<AudioQualityBadge> {
    let signal = signal?;
    let encoding = signal.encoding?;
    let format = match encoding {
        Encoding::Flac { .. } => "FLAC",
        Encoding::Alac { .. } => "ALAC",
        Encoding::Pcm { .. } => "PCM",
        Encoding::Aac { .. } => "AAC",
        Encoding::Vorbis { .. } => "VORBIS",
        Encoding::Mp3 { .. } => "MP3",
        Encoding::Opus { .. } => "OPUS",
        Encoding::Other => return None,
    };
    let lossy_bitrate = match encoding {
        Encoding::Aac { kbps }
        | Encoding::Vorbis { kbps }
        | Encoding::Mp3 { kbps }
        | Encoding::Opus { kbps } => (kbps > 0).then_some(kbps),
        _ => None,
    };
    let lossless = match encoding {
        Encoding::Flac { depth, rate } => Some((
            depth,
            match rate > 0 {
                true => rate,
                false => signal.rate,
            },
        )),
        Encoding::Alac { depth } | Encoding::Pcm { depth } => Some((depth, signal.rate)),
        _ => None,
    };
    let hi_res = lossless.is_some_and(|(depth, rate)| depth >= 24 && rate > 44_100);
    let (label, description) = match lossless {
        Some((depth, rate)) if depth > 0 && rate > 0 => {
            let rate = sample_rate(rate);
            (
                t!(
                    "player-audio-quality-lossless",
                    format = format,
                    depth = i64::from(depth),
                    rate = rate.clone()
                ),
                t!(
                    "player-audio-quality-lossless-spoken",
                    format = format,
                    depth = i64::from(depth),
                    rate = rate
                ),
            )
        }
        Some((depth, _)) if depth > 0 => (
            t!(
                "player-audio-quality-depth",
                format = format,
                depth = i64::from(depth)
            ),
            t!(
                "player-audio-quality-depth-spoken",
                format = format,
                depth = i64::from(depth)
            ),
        ),
        Some((_, rate)) if rate > 0 => {
            let rate = sample_rate(rate);
            (
                t!(
                    "player-audio-quality-rate",
                    format = format,
                    rate = rate.clone()
                ),
                t!(
                    "player-audio-quality-rate-spoken",
                    format = format,
                    rate = rate
                ),
            )
        }
        _ => match lossy_bitrate {
            Some(kbps) => (
                t!(
                    "player-audio-quality-lossy",
                    format = format,
                    kbps = i64::from(kbps)
                ),
                t!(
                    "player-audio-quality-lossy-spoken",
                    format = format,
                    kbps = i64::from(kbps)
                ),
            ),
            None => (
                t!("player-audio-quality-format", format = format),
                t!("player-audio-quality-format-spoken", format = format),
            ),
        },
    };
    let description = match hi_res {
        true => t!(
            "player-audio-quality-hires-spoken",
            quality = description.to_string()
        ),
        false => description,
    };
    let badge = AudioQualityBadge::new(id, label, description);
    Some(match hi_res {
        true => badge.hi_res(),
        false => badge,
    })
}

/// Formats a sample rate in kilohertz without trailing zeroes.
fn sample_rate(rate: u32) -> String {
    match rate % 1000 {
        0 => (rate / 1000).to_string(),
        remainder => format!("{}.{:03}", rate / 1000, remainder)
            .trim_end_matches('0')
            .to_owned(),
    }
}

/// The playing track's title and artist links with its explicit badge and heart, the way the
/// player bar draws them beside its cover. A sparkle leads the title when smart shuffle
/// recommended the song. The fullscreen player draws the same block where the
/// bar has it, so the two read as one. `centered` keeps the title centred without moving the
/// heart into it, and a right click on the title calls `open_menu`.
pub(crate) fn track_details<V: 'static>(
    track: Option<&music::Track>,
    playback: &Entity<Playback>,
    centered: bool,
    open_menu: OpenMenu<V>,
    cx: &mut Context<V>,
) -> gpui::Div {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let artist_size = theme.text(ui::Text::Small);
    let explicit = track.is_some_and(|track| track.explicit);
    let recommended = track.is_some()
        && playback
            .read(cx)
            .shown_queue(cx)
            .recommended(QueuePosition::Current, cx);
    let title = match track {
        Some(track) => {
            let name = SharedString::from(track.name.clone());
            let text = StyledText::new(name.clone());
            let layout = text.layout().clone();
            div()
                .id("now-playing-track")
                .when_some(track.pin(), Pinnable::pin)
                .map(|this| match track.album_id.clone() {
                    Some(album_id) => this
                        .aria_label(name.clone())
                        .hover(|style| style.underline())
                        .link(Destination::Album(album_id.into())),
                    None => this.role(Role::Label).aria_value(name.clone()),
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        open_menu(this, event.position, window, cx);
                    }),
                )
                .child(text)
                .tip_clipped(name, layout, Perch::Above)
                .min_w_0()
                .truncate()
                .tails()
        }
        None => div()
            .id("now-playing-album")
            .role(Role::Status)
            .aria_label(t!("player-nothing-playing"))
            .child(t!("player-nothing-playing"))
            .min_w_0()
            .text_color(muted)
            .truncate(),
    };
    let actions = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .when(explicit, |this| {
            this.child(div().flex_none().child(Badge::explicit()))
        })
        .child(like(track, playback, cx));
    let quality_badge = (Sonora::global(cx).settings.read(cx).player_bar_badge()
        && Sonora::global(cx).settings.read(cx).player_bar_layout() != PlayerBarLayout::Compact)
        .then(|| {
            audio_quality_badge(
                playback.read(cx).track_signal().and_then(|signal| signal),
                "player-bar-audio-quality",
            )
        })
        .flatten();
    let headline = match centered {
        true => {
            // Equal clearance on both sides keeps the title itself centered. Actions sit
            // outside its measured width and remain reachable even for a truncated title.
            let clearance = theme.metrics.control_small
                * match explicit {
                    true => 2.,
                    false => 1.,
                }
                + theme.metrics.pad;
            div().w_full().flex().justify_center().px(clearance).child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .min_h(theme.text(ui::Text::Body) * ui::LEADING)
                    .when(recommended, |this| this.child(Sparkle::new()))
                    .child(title)
                    .when_some(quality_badge, |this, badge| this.child(badge))
                    .child(
                        actions
                            .absolute()
                            .left(gpui::relative(1.))
                            .ml(theme.metrics.pad * 0.5)
                            .top_0()
                            .bottom_0(),
                    ),
            )
        }
        // The row is one line of the title tall, so the heart beside it reaches past it rather
        // than pushing the artist line down.
        false => div()
            .w_full()
            .flex()
            .min_w_0()
            .items_center()
            .h(theme.text(ui::Text::Body) * ui::LEADING)
            .gap_1()
            .when(recommended, |this| this.child(Sparkle::new()))
            .child(title)
            .when_some(quality_badge, |this, badge| this.child(badge))
            .child(actions),
    };
    div()
        .flex()
        .flex_col()
        .line_height(gpui::relative(ui::LEADING))
        .gap(theme.metrics.pad / 2.)
        .when(centered, |this| this.items_center())
        .justify_center()
        .flex_1()
        .min_w_0()
        .child(headline)
        .when_some(track, |this, track| {
            this.child(
                InlineLinks::new(
                    "now-playing-artist",
                    track.artist_refs.iter().map(|artist| {
                        InlineLink::new(artist.name.clone(), artist.id.clone().map(Into::into))
                    }),
                    track.artists.clone(),
                    muted,
                )
                .text_size(artist_size)
                .when(centered, |this| this.justify_center())
                .truncate()
                .on_click(|id, cx| {
                    navigate(Destination::Artist(id), cx);
                }),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rates_read_in_kilohertz() {
        assert_eq!(sample_rate(44_100), "44.1");
        assert_eq!(sample_rate(22_050), "22.05");
        assert_eq!(sample_rate(96_000), "96");
    }
}
