use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Entity, Global, SharedString};

const LINGER: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    Song(SharedString),
    Album(SharedString),
    Artist(SharedString),
    Playlist(SharedString),
}

#[derive(Clone)]
pub struct Toast {
    pub id: usize,
    pub outcome: Outcome,
    pub key: SharedString,
    pub name: Option<SharedString>,
    /// A line of its own below the text, such as the error behind a failure. Not translated.
    pub detail: Option<SharedString>,
    pub target: Option<Target>,
    held: bool,
    rest: Instant,
}

pub struct Toasts {
    shown: Vec<Toast>,
    next: usize,
}

struct Installed(Entity<Toasts>);

impl Global for Installed {}

impl Toasts {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let toasts = cx.new(|_| Self {
                shown: Vec::new(),
                next: 0,
            });
            cx.set_global(Installed(toasts));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn show(outcome: Outcome, key: impl Into<SharedString>, cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| {
            this.push(outcome, key.into(), None, None, None, cx)
        });
    }

    pub fn about(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        cx: &mut App,
    ) {
        let toasts = Self::entity(cx);
        let name = Some(name.into());
        toasts.update(cx, |this, cx| {
            this.push(outcome, key.into(), name, None, None, cx)
        });
    }

    pub fn linked(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        target: Option<Target>,
        cx: &mut App,
    ) {
        let toasts = Self::entity(cx);
        let name = Some(name.into());
        toasts.update(cx, |this, cx| {
            this.push(outcome, key.into(), name, None, target, cx)
        });
    }

    /// Shows a toast with `detail` on a line below the text, and `name`, when given, as `$name`.
    pub fn with_detail(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: Option<SharedString>,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) {
        let toasts = Self::entity(cx);
        let detail = Some(detail.into());
        toasts.update(cx, |this, cx| {
            this.push(outcome, key.into(), name, detail, None, cx)
        });
    }

    pub fn shown(&self) -> &[Toast] {
        &self.shown
    }

    pub fn dismiss(&mut self, id: usize, cx: &mut Context<Self>) {
        self.shown.retain(|toast| toast.id != id);
        cx.notify();
    }

    pub fn hold(&mut self, id: usize, held: bool, cx: &mut Context<Self>) {
        let Some(toast) = self.shown.iter_mut().find(|toast| toast.id == id) else {
            return;
        };
        if toast.held == held {
            return;
        }
        toast.held = held;
        if !held {
            toast.rest = Instant::now();
        }
        cx.notify();
    }

    fn push(
        &mut self,
        outcome: Outcome,
        key: SharedString,
        name: Option<SharedString>,
        detail: Option<SharedString>,
        target: Option<Target>,
        cx: &mut Context<Self>,
    ) {
        let showing = self.shown.iter().any(|toast| {
            toast.outcome == outcome
                && toast.key == key
                && toast.name == name
                && toast.detail == detail
        });
        if showing {
            return;
        }

        let id = self.next;
        self.next += 1;
        self.shown.push(Toast {
            id,
            outcome,
            key,
            name,
            detail,
            target,
            held: false,
            rest: Instant::now(),
        });
        cx.notify();

        cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(wait)) = this.read_with(cx, |this, _| this.left(id)) else {
                    return;
                };
                if wait.is_zero() {
                    this.update(cx, |this, cx| this.dismiss(id, cx)).ok();
                    return;
                }
                cx.background_executor().timer(wait).await;
            }
        })
        .detach();
    }

    fn left(&self, id: usize) -> Option<Duration> {
        let toast = self.shown.iter().find(|toast| toast.id == id)?;
        match toast.held {
            true => Some(LINGER),
            false => Some(LINGER.saturating_sub(toast.rest.elapsed())),
        }
    }
}
