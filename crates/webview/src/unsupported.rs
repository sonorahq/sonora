//! The backend for platforms without a sign-in window yet. macOS, Windows and Linux each have
//! one; anything else fits the same seven calls they implement.

use anyhow::{Result, bail};

use crate::{Cookie, Target};

pub(crate) fn supported() -> bool {
    false
}

pub(crate) struct Window;

impl Window {
    pub(crate) fn open(_target: &Target) -> Result<Self> {
        bail!("this platform has no sign-in window yet")
    }

    pub(crate) fn closed(&self) -> bool {
        true
    }

    pub(crate) fn host(&self) -> Option<String> {
        None
    }

    pub(crate) fn fetch(&mut self) -> Option<Vec<Cookie>> {
        None
    }

    pub(crate) fn load(&self, _url: &str) {}

    pub(crate) fn run(&self, _script: &str) {}

    pub(crate) fn close(&self) {}
}
