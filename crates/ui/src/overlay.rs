//! What every floating surface shares: the layer they are mounted into, the scrim that closes
//! them, where they sit beside their trigger and how they come and go.
//!
//! A surface renders in two parts. Its trigger stays where the view puts it, keyed so the
//! surface can find it, and the surface itself goes into a `layer: &mut Vec<El>` that the view
//! mounts over everything with [`root`]. The surface is `El::anchored` to its trigger, so it
//! follows layout, flips when it does not fit, and escapes any clipping ancestor.

use rune::{Align, El, Side, SpringPair, Transition, Vec2, el};

use crate::motion::Springs;

/// How far a surface shrinks while it fades in and out, about the side it opens from.
const RISE: f32 = 0.96;

/// Where a surface sits against its trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    Top,
    TopStart,
    TopEnd,
    Bottom,
    #[default]
    BottomStart,
    BottomEnd,
    Left,
    Right,
    RightStart,
    LeftStart,
}

impl Placement {
    /// The side of the trigger and the alignment along it, as `El::anchored` takes them.
    pub fn split(self) -> (Side, Align) {
        match self {
            Self::Top => (Side::Top, Align::Center),
            Self::TopStart => (Side::Top, Align::Start),
            Self::TopEnd => (Side::Top, Align::End),
            Self::Bottom => (Side::Bottom, Align::Center),
            Self::BottomStart => (Side::Bottom, Align::Start),
            Self::BottomEnd => (Side::Bottom, Align::End),
            Self::Left => (Side::Left, Align::Center),
            Self::Right => (Side::Right, Align::Center),
            Self::RightStart => (Side::Right, Align::Start),
            Self::LeftStart => (Side::Left, Align::Start),
        }
    }
}

/// The view's content with its floating surfaces over it. Call it once, at the root of the
/// view, with every surface the components pushed into the layer.
pub fn root(base: El, layer: impl IntoIterator<Item = El>) -> El {
    el().key("ui-root")
        .w_full()
        .h_full()
        .child(base.w_full().h_full())
        .children(layer)
}

/// A transparent target over the whole window under an open surface: a press outside the
/// surface lands here and closes it.
pub fn scrim<S: 'static>(key: impl Into<String>, close: impl Fn(&mut S) + 'static) -> El {
    el().key(key)
        .role("none")
        .absolute(0., 0.)
        .w_full()
        .h_full()
        .on_press(close)
}

/// The entrance and exit of a surface opening from `side` of its trigger: it fades and grows
/// out of the edge that touches the trigger, on the quick spring.
pub fn rise(side: Side, align: Align) -> Transition {
    Transition::scale_anchor(RISE, origin(side, align))
        .springs(SpringPair::new(Springs::QUICK, Springs::CONTROL))
}

/// The entrance of a surface with no trigger, such as a dialog: it grows from its middle.
pub fn bloom() -> Transition {
    Transition::scale(RISE).springs(SpringPair::new(Springs::QUICK, Springs::CONTROL))
}

/// Places `content` on the `placement` side of the element keyed `anchor`, `gap` points away,
/// and gives it the surface entrance. Above or below its trigger it is at most as tall as the
/// window leaves it, so a long list inside it should scroll.
pub fn anchor(content: El, anchor: &str, placement: Placement, gap: f32) -> El {
    let (side, align) = placement.split();
    let placed = match side {
        Side::Top | Side::Bottom => content.anchored_fit(anchor, side, align, gap),
        Side::Left | Side::Right => content.anchored(anchor, side, align, gap),
    };
    placed.transition(rise(side, align))
}

/// The point of a surface that stays put while it grows in, in fractions of its size.
fn origin(side: Side, align: Align) -> Vec2 {
    let along = match align {
        Align::Start => 0.,
        Align::Center => 0.5,
        Align::End => 1.,
    };
    match side {
        Side::Top => Vec2::new(along, 1.),
        Side::Bottom => Vec2::new(along, 0.),
        Side::Left => Vec2::new(1., along),
        Side::Right => Vec2::new(0., along),
    }
}
