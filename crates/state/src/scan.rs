use std::time::{Duration, Instant};

use music::progress::Progress;

use crate::{Change, Cx, Event, Job, Outcome, Session, Store, Toast, Topic};

/// How often the progress is sampled while a scan runs. The count lives in atomics the workers
/// bump, so this is only how often the screen catches up with them.
const TICK: Duration = Duration::from_millis(100);

/// Follows the local library scan for the UI: how far it has got, and how long the last one the
/// user asked for took.
///
/// The scan runs on worker threads inside `music`, so its numbers are sampled rather than pushed.
/// `Session::scanning` says when one is under way, which is what starts and stops the sampling.
pub struct Scan {
    running: bool,
    started: Option<Instant>,
    /// Set by the Rescan button, so a scan at startup reports nothing when it ends.
    asked: bool,
    done: Option<Duration>,
    /// Whether the settings page has had a chance to show `done`, which is what lets leaving the
    /// page clear it.
    seen: bool,
    /// Whether the settings page is open, which shows a scan's progress itself.
    on_settings: bool,
    poll: Option<Job>,
}

impl Scan {
    pub fn new(session: &Session, cx: &mut Cx) -> Self {
        let scanning = session.scanning();
        let mut scan = Self {
            running: false,
            started: None,
            asked: false,
            done: None,
            seen: false,
            on_settings: false,
            poll: None,
        };
        scan.follow(scanning, cx);
        scan
    }

    /// The scan running right now, if one is.
    pub fn progress(&self) -> Option<Progress> {
        self.running.then(music::progress::scanning).flatten()
    }

    /// How long the last scan the user asked for took, until the settings page has shown it and
    /// been left.
    pub fn done(&self) -> Option<Duration> {
        self.done
    }

    /// Says the next scan is one the user asked for, so its time is worth reporting.
    pub fn asked(&mut self) {
        self.asked = true;
    }

    /// Follows the settings page: arriving marks a finished scan as shown, leaving forgets it.
    /// Switching category counts as arriving, so only leaving the page clears the note.
    pub fn viewing_settings(&mut self, viewing: bool) {
        self.on_settings = viewing;
        match viewing {
            true => self.seen = self.done.is_some(),
            false if self.seen => {
                self.done = None;
                self.seen = false;
            }
            false => {}
        }
    }

    /// Reports a scan the user started that found `songs`, unless the settings page is open to
    /// show it.
    pub(crate) fn finished(&self, songs: usize, cx: &mut Cx) {
        if self.on_settings {
            return;
        }
        cx.emit(Event::Toast(
            Toast::new(Outcome::Done, "toast-library-scanned").count(songs),
        ));
    }

    /// Picks up a scan starting or ending. A scan the user asked for leaves its duration behind.
    fn follow(&mut self, scanning: bool, cx: &mut Cx) {
        if scanning == self.running {
            return;
        }
        self.running = scanning;
        match scanning {
            true => {
                self.started = Some(Instant::now());
                self.done = None;
                self.seen = false;
                self.poll = Some(cx.spawn(|link| async move {
                    loop {
                        tokio::time::sleep(TICK).await;
                        link.post(|store| store.cx.refresh());
                    }
                }));
            }
            false => {
                self.poll = None;
                let took = self.started.take().map(|started| started.elapsed());
                // A scan cut short by a folder being removed has no time worth reporting.
                if self.asked && !music::progress::interrupted() {
                    self.done = took;
                }
                self.asked = false;
            }
        }
    }
}

impl Store {
    /// Starts or stops sampling the scan as the session starts or ends one.
    pub(crate) fn react_scan(&mut self, change: &Change) {
        match change {
            Change::Topic(Topic::Session) => {
                let scanning = self.session.scanning();
                self.scan.follow(scanning, &mut self.cx);
            }
            _ => {}
        }
    }
}
