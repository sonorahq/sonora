//! A side panel of a set width with a grip along its inner edge that resizes it.

use std::rc::Rc;

use rune::{Cursor, Drag, El, Phase, Tx, col, el};

use crate::theme::Theme;

/// The grip's width, how far it reaches past the panel's edge, and how much further it moves
/// out of a scrollbar's way.
const GRIP: f32 = 12.;
const GRIP_INSET: f32 = -6.;
const GRIP_CLEARANCE: f32 = 4.;
const BORDER: f32 = 1.;

type Resize<S> = Rc<dyn Fn(&mut S, f32, &mut Tx)>;

/// Which side of the window a panel stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A panel. Its width is the owner's state; a drag on the grip reports each new width, held
/// between the limits and snapped to whole points, to [`Panel::on_resize`].
pub struct Panel<S> {
    key: String,
    side: Side,
    width: f32,
    min: f32,
    max: f32,
    reach: Option<f32>,
    fill: bool,
    clearance: bool,
    grip_bottom: f32,
    resize: Option<Resize<S>>,
    children: Vec<El>,
}

impl<S: 'static> Panel<S> {
    pub fn new(key: impl Into<String>, side: Side, width: f32) -> Self {
        Self {
            key: key.into(),
            side,
            width,
            min: 0.,
            max: f32::MAX,
            reach: None,
            fill: false,
            clearance: false,
            grip_bottom: 0.,
            resize: None,
            children: Vec::new(),
        }
    }

    pub fn limits(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// How wide a drag may make the panel, when that is less than its limit.
    pub fn reach(mut self, reach: f32) -> Self {
        self.reach = Some(reach);
        self
    }

    /// Takes the room its row leaves instead of its width, and drops the grip.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Moves the grip clear of a scrollbar along the panel's inner edge.
    pub fn clears_scrollbar(mut self) -> Self {
        self.clearance = true;
        self
    }

    /// Ends the grip `bottom` above the panel's foot, so chrome over the panel's lower edge
    /// keeps its own pointer.
    pub fn grip_above(mut self, bottom: f32) -> Self {
        self.grip_bottom = bottom;
        self
    }

    pub fn on_resize(mut self, f: impl Fn(&mut S, f32, &mut Tx) + 'static) -> Self {
        self.resize = Some(Rc::new(f));
        self
    }

    pub fn child(mut self, child: El) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = El>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let width = self.width.clamp(self.min, self.max);
        let edge = match self.side {
            Side::Left => [0., BORDER, 0., 0.],
            Side::Right => [0., 0., 0., BORDER],
        };
        let mut panel = col()
            .key(self.key.clone())
            .h_full()
            .border_sides(edge, theme.sidebar_border.color())
            .children(self.children);
        panel = match self.fill {
            true => panel.grow(1.).min_w(0.),
            false => panel.w(width),
        };
        let Some(resize) = self.resize.filter(|_| !self.fill) else {
            return panel;
        };
        let (min, max) = (self.min, self.reach.unwrap_or(self.max));
        let side = self.side;
        let clearance = match self.clearance {
            true => GRIP_CLEARANCE,
            false => 0.,
        };
        let inset = GRIP_INSET - clearance;
        let grip = match side {
            Side::Left => el().pin(None, Some(0.), Some(inset), Some(self.grip_bottom)),
            Side::Right => el().pin(Some(inset), Some(0.), None, Some(self.grip_bottom)),
        }
        .key(format!("{}-grip", self.key))
        .role("separator")
        .w(GRIP)
        .cursor(Cursor::ResizeColumn)
        .raise()
        .on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
            tx.instant();
            if drag.phase == Phase::Down {
                drag.data = width;
                return;
            }
            let dragged = match side {
                Side::Left => drag.data + drag.dx,
                Side::Right => drag.data - drag.dx,
            };
            resize(s, dragged.clamp(min, max).round(), tx);
        });
        panel.child(grip)
    }
}
