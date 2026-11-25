use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    Context, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString, StyledText, Window,
    div,
};
use i18n::t;
use router::{Destination, Link, navigate};
use state::{Playback, QueuePosition};
use ui::{
    ActiveTheme as _, Badge, InlineLink, InlineLinks, Perch, Pinnable, Sparkle, Tails as _,
    Tipped as _,
};

use crate::shared::pins::Pinned as _;
use crate::shared::transport::like;

/// Opens the track menu of the view that draws the details at the pointer's window position.
type OpenMenu<V> = fn(&mut V, Point<Pixels>, &mut Window, &mut Context<V>);

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
    let artists = theme.text(ui::Text::Small);
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
                .text_size(artists)
                .when(centered, |this| this.justify_center())
                .truncate()
                .on_click(|id, cx| {
                    navigate(Destination::Artist(id), cx);
                }),
            )
        })
}
