//! Whether this machine can decrypt protected tracks.
//!
//! Sonora neither ships a Widevine module nor downloads one: Google licenses it to browser and
//! device vendors and publishes nothing anyone else may pass on. Almost every machine has a
//! copy that came with a browser, so [`Drm`] looks for one at startup and that is the whole
//! story. With none found, protected providers keep their metadata and refuse to play.

use gpui::{Context, Task};
use music::drm::{self, Origin};

use crate::Io;

/// Where the search for a Widevine module stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CdmState {
    /// Still looking, which is only the first moment of a run.
    Looking,
    /// One is here, and where it came from.
    Ready(Origin),
    /// Nothing on this machine has one.
    Missing,
}

/// The system Widevine module as app state.
pub struct Drm {
    state: CdmState,
    io: Io,
    task: Option<Task<()>>,
}

impl Drm {
    pub fn new(io: Io, cx: &mut Context<Self>) -> Self {
        let mut drm = Self {
            state: CdmState::Missing,
            io,
            task: None,
        };
        if drm::supported() {
            drm.look(cx);
        }
        drm
    }

    pub fn state(&self) -> &CdmState {
        &self.state
    }

    /// Whether this build has a host for a module at all. False means the platform has no
    /// prebuilt host, so nothing about a module is worth showing the user.
    pub fn supported(&self) -> bool {
        drm::supported()
    }

    /// Looks again, off the foreground because it walks a handful of directories.
    pub fn look(&mut self, cx: &mut Context<Self>) {
        self.state = CdmState::Looking;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let found = io.spawn_blocking(drm::find).await.ok().flatten();
            this.update(cx, |this, cx| {
                this.task = None;
                this.state = match found {
                    Some(found) => CdmState::Ready(found.origin),
                    None => CdmState::Missing,
                };
                cx.notify();
            })
            .ok();
        }));
    }
}
