use i18n::SharedString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinKind {
    Album,
    Artist,
    Playlist,
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
            Self::Song => "icons/music.svg",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Album => "kind-album",
            Self::Artist => "kind-artist",
            Self::Playlist => "kind-playlist",
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
    /// An opaque tag for where a dragged row was picked up, which ui never reads. Never stored.
    #[serde(skip)]
    pub origin: Option<String>,
}

impl Pin {
    pub fn new(kind: PinKind, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            title: title.into(),
            cover: None,
            origin: None,
        }
    }

    pub fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub fn origin(mut self, origin: Option<String>) -> Self {
        self.origin = origin;
        self
    }

    pub fn same(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }

    pub fn label(&self) -> SharedString {
        SharedString::Owned(self.title.clone())
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
