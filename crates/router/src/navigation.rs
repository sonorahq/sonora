use crate::{Destination, SettingsPage};

/// The pages visited, with Back and Forward through them. Each move answers the destination
/// it arrived at, or `None` when it went nowhere, so the window can start the page's transition.
pub struct Navigation {
    trail: Vec<Destination>,
    at: usize,
    /// The settings destination last on show, which opening settings again returns to.
    last_settings: Option<Destination>,
}

impl Navigation {
    pub fn new(start: Destination) -> Self {
        Self {
            trail: vec![start],
            at: 0,
            last_settings: None,
        }
    }

    pub fn current(&self) -> Destination {
        self.trail[self.at].clone()
    }

    pub fn can_go_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.at + 1 < self.trail.len()
    }

    pub fn go(&mut self, destination: Destination) -> Option<Destination> {
        if self.current() == destination {
            return None;
        }

        let replacing = self.current() == Destination::Fullscreen;
        self.trail.truncate(self.at + 1);
        match replacing {
            true => self.trail[self.at] = destination,
            false => {
                self.trail.push(destination);
                self.at = self.trail.len() - 1;
            }
        }
        self.arrive()
    }

    pub fn back(&mut self) -> Option<Destination> {
        if !self.can_go_back() {
            return None;
        }
        self.at -= 1;
        self.arrive()
    }

    /// Replaces the current page without leaving its stale destination in navigation history.
    pub fn replace(&mut self, destination: Destination) -> Option<Destination> {
        if self.current() == destination {
            return None;
        }
        self.trail[self.at] = destination;
        self.arrive()
    }

    /// Returns from a drill-in to `parent`. Stepping back keeps the trail free of a parent,
    /// drill-in, parent loop when the page before shows the parent's settings tab, which also
    /// puts that tab back where it was scrolled to. Anywhere else the drill-in is replaced so Back
    /// does not reopen it.
    pub fn up(&mut self, parent: Destination) -> Option<Destination> {
        let tab = |destination: &Destination| destination.settings_page().map(SettingsPage::tab);
        let before = self.at.checked_sub(1).map(|at| &self.trail[at]);
        match before.is_some_and(|before| {
            *before == parent || (tab(before).is_some() && tab(before) == tab(&parent))
        }) {
            true => self.back(),
            false => self.replace(parent),
        }
    }

    /// Opens settings on the destination last shown there, or on General the first time.
    pub fn open_settings(&mut self) -> Option<Destination> {
        let destination = self
            .last_settings
            .clone()
            .unwrap_or(Destination::Settings(SettingsPage::General));
        self.go(destination)
    }

    pub fn forward(&mut self) -> Option<Destination> {
        if !self.can_go_forward() {
            return None;
        }
        self.at += 1;
        self.arrive()
    }

    fn arrive(&mut self) -> Option<Destination> {
        if self.current().in_settings() {
            self.last_settings = Some(self.current());
        }
        Some(self.current())
    }
}
