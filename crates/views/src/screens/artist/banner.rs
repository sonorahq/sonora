//! The picture behind an artist's hero: the provider's banner, or a wash of the portrait's
//! colours when it keeps none.

use std::f32::consts::PI;

use music::Artist;
use rune::{El, el};
use ui::Cover;

use crate::Ui;

/// The least and the most room the banner adds above the hero, as shares of the cover's edge.
/// In between, the banner box keeps `BANNER_BOX` as its aspect, so a narrow page lifts the hero
/// less.
const RISE_LEAST: f32 = 0.35;
const RISE_MOST: f32 = 1.;
const BANNER_BOX: f32 = 2.7;
/// The room above the hero when only a blurred portrait stands in for the banner. The wash has
/// no proportions to keep, so it stays this tall at any width.
const RISE_WASH: f32 = 0.6;
/// The aspect the banner picture is drawn at. Spotify's banners are this wide, and the box is a
/// little wider still, so the picture loses a strip at its top and bottom but never its sides.
const ASPECT: f32 = 2.33;
/// How much of what the banner box cannot show is cut off above rather than below. Banners keep
/// their subject a little above the middle.
const ANCHOR: f32 = 0.35;
/// The tallest the banner picture grows, as a share of the banner's height. On a page wide
/// enough to grow it further it keeps this size in the middle, and both its sides fade into a
/// washed out copy over the room beside it, up to `SIDE_FADE` of its width.
const OVERFLOW: f32 = 1.25;
const SIDE_FADE: f32 = 0.2;
/// How much wider than the page the banner is drawn, so it still reaches both edges while the
/// width it was measured from lags a resize by a frame.
const SLACK: f32 = 0.06;
/// The blurs the banner steps through on its way down, each with the share of the stretch from
/// the hero's top to the banner's bottom at which it has fully taken over from the one above.
/// Everything above the hero stays sharp, and the blur grows slowly at first so it has no
/// visible start.
const BLURS: [(f32, f32); 7] = [
    (0.25, 1.),
    (0.375, 2.5),
    (0.5, 4.5),
    (0.625, 7.5),
    (0.75, 11.5),
    (0.875, 17.),
    (1., 24.),
];
/// How far above its own mark each blur starts fading in, as a share of the same stretch. It is
/// longer than the gap between marks, so each blur is still fading in when the next starts.
const RAMP: f32 = 0.25;
/// How much of the page colour lies over the whole banner, so it sits back behind the page.
const DIM: f32 = 0.3;
/// The shade that deepens toward the banner's bottom on top of `DIM`: where it starts, as a share
/// of the way down to the hero's top, and how much of the page colour it reaches at the bottom.
const SHADE_FROM: f32 = 0.5;
const SHADE: f32 = 0.85;
/// The blur over a portrait standing in for a missing banner, enough to leave only its colours.
const WASH: f32 = 48.;

/// How far the hero sits below the page's top inset to leave the banner room, `page` being the
/// page's whole width. Whole points, so the hero does not shimmer as the window resizes.
pub(super) fn rise(artist: Option<&Artist>, page: f32, inset: f32, cover: f32) -> f32 {
    match artist.is_some_and(|artist| artist.banner.is_some()) {
        true => (page / BANNER_BOX - inset - cover)
            .max(cover * RISE_LEAST)
            .min(cover * RISE_MOST),
        false => cover * RISE_WASH,
    }
    .round()
}

/// The artist's banner, `height` tall across the `page` wide box it fills from the page's top
/// left corner, with the hero's top `hero` below its own. It is dimmed throughout and stays sharp
/// above the hero, then blurs, darkens and fades harder until it is gone at its bottom. Without a
/// banner the portrait stands in, blurred to a wash of its colours so it does not repeat the
/// cover beside it. None when the artist has neither, or has only a portrait while blurs are
/// off.
pub(super) fn view(ui: &Ui, artist: &Artist, page: f32, height: f32, hero: f32) -> Option<El> {
    let theme = &ui.theme;
    let effects = ui::effects();
    let (url, wash) = match &artist.banner {
        Some(banner) => (banner.as_str(), None),
        None if effects => (artist.cover_large.as_deref()?, Some(WASH)),
        None => return None,
    };
    let artwork = ui.artwork();
    // Everything is drawn a little wider than the box, centred in it, and the box clips the rest.
    let width = page * (1. + SLACK);
    let background = theme.background;
    let shade = el()
        .key("banner-shade")
        .pin(Some(0.), Some(0.), Some(0.), Some(0.))
        .fill(background.opacity(DIM).color())
        .child(
            el().pin(Some(0.), Some(0.), Some(0.), Some(0.))
                .linear_gradient(
                    PI,
                    [
                        (hero / height * SHADE_FROM, background.opacity(0.).color()),
                        (1., background.opacity(SHADE).color()),
                    ],
                ),
        );
    // Every copy reaches past the box by twice the widest blur, so no blur pulls in the
    // transparent edge, and they all share one geometry so they line up.
    let bleed = wash.unwrap_or(BLURS[BLURS.len() - 1].1) * 2.;
    let natural = width / ASPECT;
    let tall = match wash {
        Some(_) => natural,
        None => natural.min(height * OVERFLOW),
    }
    .max(height);
    let across = (tall * ASPECT).min(width);
    // The fade grows from nothing with the room either side, so a page whose width sits near the
    // point where the picture stops filling it does not flick the fade on and off.
    let side = bleed + ((width - across) / 2.).min(across * SIDE_FADE);
    let lifted = (tall - height) * ANCHOR;
    let left = (page - across) / 2. - bleed;
    let picture = |key: String, top: f32| {
        el().key(key)
            .absolute(left, -top - lifted - bleed)
            .clip_fade(side, 0., side, 0.)
            .child(
                Cover::new(artwork, Some(url))
                    .size(across + bleed * 2.)
                    .wide(tall + bleed * 2.)
                    .overlay()
                    .radius(0.)
                    .build(theme),
            )
    };

    let mut banner = el()
        .key("artist-banner")
        .pin(Some(0.), Some(0.), Some(0.), None)
        .h(height)
        .clip_fade(0., 0., 0., height - hero)
        .a11y_hidden();
    banner = match wash {
        Some(wash) => banner.child(picture("banner-wash".into(), 0.).blur(wash)),
        None => {
            // The same picture lies under the banner across the whole box, washed out like a
            // missing banner's portrait, so a picture narrower than the page has its colours
            // either side.
            let spread = WASH * 2.;
            if effects {
                banner = banner.child(
                    el().key("banner-backdrop")
                        .absolute((page - width) / 2. - spread, -spread)
                        .blur(WASH)
                        .child(
                            Cover::new(artwork, Some(url))
                                .size(width + spread * 2.)
                                .wide(height + spread * 2.)
                                .overlay()
                                .radius(0.)
                                .build(theme),
                        ),
                );
            }
            banner = banner.child(picture("banner-picture".into(), 0.));
            if effects {
                let below = height - hero;
                banner = banner.children(BLURS.iter().enumerate().map(|(step, &(mark, blur))| {
                    let top = hero + below * (mark - RAMP);
                    el().key(format!("banner-blur-{step}"))
                        .pin(Some(0.), Some(top), Some(0.), Some(0.))
                        .clip_fade(0., below * RAMP, 0., 0.)
                        .child(picture(format!("banner-step-{step}"), top).blur(blur))
                }));
            }
            banner
        }
    };
    Some(banner.child(shade))
}
