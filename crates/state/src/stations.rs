use std::rc::Rc;

use gpui::{Context, Entity, Task};
use music::Station;

use crate::{Io, Session, SessionEvent, join};

/// The internet radio stations of the signed-in server, for the Radio page. Nothing is fetched
/// until the page calls `load`, and a sign-in fetches them again only after that.
pub struct Stations {
    stations: Rc<Vec<Station>>,
    /// Whether the page has asked for the stations, so a sign-in knows to fetch them.
    wanted: bool,
    /// Whether the stations of the current session have arrived, so `load` asks only once.
    loaded: bool,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
}

impl Stations {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => {
                this.clear(cx);
                if this.wanted {
                    this.load(cx);
                }
            }
            SessionEvent::SignedOut => this.clear(cx),
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            stations: Rc::new(Vec::new()),
            wanted: false,
            loaded: false,
            loading: false,
            error: None,
            session,
            io,
            task: None,
        }
    }

    pub fn stations(&self) -> Rc<Vec<Station>> {
        self.stations.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Fetches the stations unless they are loaded or loading. Before a sign-in it only marks
    /// them wanted, and the sign-in fetches them.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.wanted = true;
        if self.loading || self.loaded {
            return;
        }
        self.fetch(cx);
    }

    /// Asks the server for its stations again, whatever was loaded before.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.wanted = true;
        self.fetch(cx);
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            return;
        };

        self.loading = true;
        self.error = None;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move { client.stations().await })).await;

            this.update(cx, |this, cx| {
                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(stations) => {
                        this.stations = Rc::new(stations);
                        this.loaded = true;
                    }
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Forgets the stations of the session that ended.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.stations = Rc::new(Vec::new());
        self.loaded = false;
        self.loading = false;
        self.error = None;
        cx.notify();
    }
}
