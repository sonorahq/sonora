//! How state reaches the async world and the UI thread without a UI framework.
//!
//! Every model is a plain struct inside [`Store`](crate::Store). Work that waits (the network, a
//! timer, a blocking read) runs on tokio as a [`Job`]; when it has an answer it hands the UI
//! thread a closure through its [`Link`], which runs with the whole store and then lets the
//! store settle the [`Event`]s that closure raised. Models never hold handles to each other: a
//! method that needs another model takes it as an argument, and a change other models react to
//! is an [`Event`] that [`Store::settle`](crate::Store::settle) routes, in one place.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Change, Event, Io, Store, Topic};

/// Work for the UI thread, carrying whatever a job found.
pub type Apply = Box<dyn FnOnce(&mut Store) + Send>;

type Deliver = dyn Fn(Apply) + Send + Sync;

/// Hands closures to the UI thread from any thread. The host (the window) decides how: Sonora
/// queues them on Rune's event loop, where each runs between frames with the store.
#[derive(Clone)]
pub struct Link {
    deliver: Arc<Deliver>,
    /// Cleared when the job this link belongs to is dropped, so an answer that was already on
    /// its way when the job was cancelled changes nothing.
    alive: Option<Arc<AtomicBool>>,
}

impl Link {
    pub fn new(deliver: impl Fn(Apply) + Send + Sync + 'static) -> Self {
        Self {
            deliver: Arc::new(deliver),
            alive: None,
        }
    }

    /// A link that drops everything, for tests and for a store with no window.
    pub fn detached() -> Self {
        Self::new(|_| {})
    }

    /// Runs `apply` on the UI thread with the store, unless the job that holds this link was
    /// cancelled first.
    pub fn post(&self, apply: impl FnOnce(&mut Store) + Send + 'static) {
        let alive = self.alive.clone();
        (self.deliver)(Box::new(move |store| {
            if alive.is_some_and(|alive| !alive.load(Ordering::Acquire)) {
                return;
            }
            apply(store);
        }));
    }

    /// Runs `read` on the UI thread and waits for its answer. The window draws afterwards only
    /// if `read` raised a change. `None` when the job was cancelled
    /// or the window is gone before it ran.
    pub async fn read<R: Send + 'static>(
        &self,
        read: impl FnOnce(&mut Store) -> R + Send + 'static,
    ) -> Option<R> {
        let (answer, answered) = tokio::sync::oneshot::channel();
        // A read asks; it doesn't change what the window shows unless the closure raises a
        // topic or an event, or refreshes, so it doesn't run the view by itself. A background
        // worker that polls (the PO token minter, a heartbeat) costs no frame.
        self.post(move |store| {
            store.cx.hush();
            answer.send(read(store)).ok();
        });
        answered.await.ok()
    }

    fn for_job(&self, alive: Arc<AtomicBool>) -> Self {
        Self {
            deliver: self.deliver.clone(),
            alive: Some(alive),
        }
    }
}

/// A piece of work running on tokio. Dropping it cancels the work and anything it already
/// posted but that hasn't run yet; [`Job::detach`] lets it run to the end.
#[must_use = "a job is cancelled as soon as it is dropped"]
pub struct Job {
    abort: Option<tokio::task::AbortHandle>,
    alive: Arc<AtomicBool>,
}

impl Job {
    /// Lets the work run to the end with no one holding it.
    pub fn detach(mut self) {
        self.abort = None;
    }

    pub fn is_finished(&self) -> bool {
        self.abort.as_ref().is_none_or(|abort| abort.is_finished())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let Some(abort) = self.abort.take() else {
            return;
        };
        self.alive.store(false, Ordering::Release);
        abort.abort();
    }
}

/// What every model method that starts work or tells others about a change is handed: the
/// tokio runtime, the way back to the UI thread and the queue of events the store settles.
pub struct Cx {
    pub io: Io,
    link: Link,
    changes: Vec<Change>,
    /// Whether anything changed that the window shows outside a commit the host already makes,
    /// so a poll knows to draw.
    dirty: bool,
}

impl Cx {
    pub fn new(io: Io, link: Link) -> Self {
        Self {
            io,
            link,
            changes: Vec::new(),
            dirty: true,
        }
    }

    pub fn link(&self) -> Link {
        self.link.clone()
    }

    /// Starts `work` on tokio. It gets a link that stops delivering once the job is dropped.
    pub fn spawn<F>(&self, work: impl FnOnce(Link) -> F) -> Job
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let alive = Arc::new(AtomicBool::new(true));
        let future = work(self.link.for_job(alive.clone()));
        let handle = self.io.spawn(future);
        Job {
            abort: Some(handle.abort_handle()),
            alive,
        }
    }

    /// Runs `work` on tokio and hands its answer to `apply` on the UI thread.
    pub fn load<T: Send + 'static>(
        &self,
        work: impl Future<Output = anyhow::Result<T>> + Send + 'static,
        apply: impl FnOnce(&mut Store, anyhow::Result<T>) + Send + 'static,
    ) -> Job {
        self.spawn(move |link| async move {
            let answer = work.await;
            link.post(move |store| apply(store, answer));
        })
    }

    /// Runs `apply` on the UI thread after `wait`.
    pub fn after(
        &self,
        wait: std::time::Duration,
        apply: impl FnOnce(&mut Store) + Send + 'static,
    ) -> Job {
        self.spawn(move |link| async move {
            tokio::time::sleep(wait).await;
            link.post(apply);
        })
    }

    /// Queues an event for the store to route once the current change is done.
    pub fn emit(&mut self, event: Event) {
        self.changes.push(Change::Event(event));
        self.dirty = true;
    }

    /// Says that a model others follow changed. Raising it again before the store settles
    /// reaches the followers once.
    pub fn changed(&mut self, topic: Topic) {
        self.dirty = true;
        let pending = self
            .changes
            .iter()
            .any(|change| matches!(change, Change::Topic(raised) if *raised == topic));
        if !pending {
            self.changes.push(Change::Topic(topic));
        }
    }

    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    /// Asks the window to draw, for a change made outside a handler or a posted closure.
    pub fn refresh(&mut self) {
        self.dirty = true;
    }

    /// Says that the change being applied shows nothing on screen, such as a position report
    /// the window already follows through its live values, so the window skips running its
    /// view for it. A topic or event raised afterwards still asks for a draw.
    pub fn hush(&mut self) {
        self.dirty = false;
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}
