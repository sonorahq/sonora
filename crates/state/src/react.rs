//! Where every change reaches the models that follow it. Each model that follows another has a
//! `react_*` method on [`Store`] in its own file; this is the one list of them, in the order
//! they run: the order the models are built in, so a model hears a change after the models it
//! depends on have.

use crate::{Change, Store};

impl Store {
    pub(crate) fn react(&mut self, change: &Change) {
        self.react_network(change);
        self.react_session(change);
        self.react_updates(change);
        self.react_toasts(change);
        self.react_library(change);
        self.react_queue(change);
        self.react_equalizer(change);
        self.react_playback(change);
        self.react_downloads(change);
        self.react_history(change);
        self.react_scan(change);
        self.react_scrobbling(change);
        self.react_lyrics(change);
        self.react_cover(change);
        self.react_drm(change);
        self.react_requirements(change);
        self.react_usage(change);
        self.react_pins(change);
        self.react_signal(change);
        self.react_wake(change);
        self.react_devices(change);
        self.react_listening(change);
        self.react_links(change);
        self.react_control(change);
        self.react_discord(change);
        self.react_remote(change);
        self.react_home(change);
        self.react_genre(change);
        self.react_detail(change);
        self.react_artist(change);
        self.react_song(change);
        self.react_profile(change);
        self.react_search(change);
    }
}
