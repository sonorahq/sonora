use gpui::{Context, EventEmitter};

use crate::{Destination, SettingsPage};

pub enum NavigationEvent {
    Moved(Destination),
}

pub struct Navigation {
    trail: Vec<Destination>,
    at: usize,
    /// The settings destination last on show, which opening settings again returns to.
    last_settings: Option<Destination>,
}

impl EventEmitter<NavigationEvent> for Navigation {}

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

    pub fn go(&mut self, destination: Destination, cx: &mut Context<Self>) {
        if self.current() == destination {
            return;
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
        self.arrive(cx);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if !self.can_go_back() {
            return;
        }
        self.at -= 1;
        self.arrive(cx);
    }

    /// Replaces the current page without leaving its stale destination in navigation history.
    pub fn replace(&mut self, destination: Destination, cx: &mut Context<Self>) {
        if self.current() == destination {
            return;
        }
        self.trail[self.at] = destination;
        self.arrive(cx);
    }

    /// Returns from a drill-in to `parent`. Stepping back keeps the trail free of a parent,
    /// drill-in, parent loop when the page before shows the parent's settings tab, which also
    /// puts that tab back where it was scrolled to. Anywhere else the drill-in is replaced so Back
    /// does not reopen it.
    pub fn up(&mut self, parent: Destination, cx: &mut Context<Self>) {
        let tab = |destination: &Destination| destination.settings_page().map(SettingsPage::tab);
        let before = self.at.checked_sub(1).map(|at| &self.trail[at]);
        match before.is_some_and(|before| {
            *before == parent || (tab(before).is_some() && tab(before) == tab(&parent))
        }) {
            true => self.back(cx),
            false => self.replace(parent, cx),
        }
    }

    /// Opens settings on the destination last shown there, or on General the first time.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        let destination = self
            .last_settings
            .clone()
            .unwrap_or_else(|| Destination::Settings(crate::SettingsTab::General.lead()));
        self.go(destination, cx);
    }

    pub fn forward(&mut self, cx: &mut Context<Self>) {
        if !self.can_go_forward() {
            return;
        }
        self.at += 1;
        self.arrive(cx);
    }

    fn arrive(&mut self, cx: &mut Context<Self>) {
        if self.current().in_settings() {
            self.last_settings = Some(self.current());
        }
        cx.emit(NavigationEvent::Moved(self.current()));
        cx.notify();
    }
}
