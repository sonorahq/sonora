use gpui::prelude::*;
use gpui::{Context, Pixels, Point, Render, SharedString, Window};
use serde::{Deserialize, Serialize};

use crate::drag::Ghost;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinKind {
    Album,
    Artist,
    Playlist,
    /// A folder of playlists. It opens, but nothing plays it.
    Folder,
    Song,
}

impl PinKind {
    pub fn round(self) -> bool {
        matches!(self, Self::Artist)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Album => "icons/disc-3.svg",
            Self::Artist => "icons/user.svg",
            Self::Playlist => "icons/list.svg",
            Self::Folder => "icons/folder.svg",
            Self::Song => "icons/music.svg",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Album => "kind-album",
            Self::Artist => "kind-artist",
            Self::Playlist => "kind-playlist",
            Self::Folder => "kind-folder",
            Self::Song => "kind-song",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub kind: PinKind,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

impl Pin {
    pub fn new(kind: PinKind, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            title: title.into(),
            cover: None,
        }
    }

    pub fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub fn same(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }

    pub fn label(&self) -> SharedString {
        SharedString::from(self.title.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub list: &'static str,
    pub index: usize,
    pub revision: u64,
}

impl Spot {
    pub fn new(list: &'static str, index: usize) -> Self {
        Self {
            list,
            index,
            revision: 0,
        }
    }

    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }
}

#[derive(Clone)]
pub struct DraggedPin {
    pub pin: Pin,
    from: Option<Spot>,
    position: Point<Pixels>,
}

impl DraggedPin {
    fn new(pin: Pin, from: Option<Spot>) -> Self {
        Self {
            pin,
            from,
            position: Point::default(),
        }
    }

    pub fn spot(&self, list: &str) -> Option<Spot> {
        self.from.filter(|spot| spot.list == list)
    }

    fn at(mut self, position: Point<Pixels>) -> Self {
        self.position = position;
        self
    }
}

impl Render for DraggedPin {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Ghost::new(self.position, self.pin.label())
            .art(self.pin.cover.clone())
            .fallback(self.pin.kind.icon())
            .when(self.pin.kind.round(), Ghost::circle)
    }
}

pub trait Pinnable: Sized {
    fn pin(self, pin: Pin) -> Self;
    fn pin_from(self, pin: Pin, spot: Spot) -> Self;
}

impl<T: StatefulInteractiveElement> Pinnable for T {
    fn pin(self, pin: Pin) -> Self {
        haul(self, DraggedPin::new(pin, None))
    }

    fn pin_from(self, pin: Pin, spot: Spot) -> Self {
        haul(self, DraggedPin::new(pin, Some(spot)))
    }
}

fn haul<T: StatefulInteractiveElement>(element: T, dragged: DraggedPin) -> T {
    element.on_drag(dragged, |dragged, position, _, cx| {
        cx.new(|_| dragged.clone().at(position))
    })
}
