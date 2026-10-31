use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Entity, Global, SharedString, Task};

const LINGER: Duration = Duration::from_secs(4);

type Run = Rc<dyn Fn(&mut App)>;

/// What a toast reports, which picks its icon. `Loading` stays up until it is settled or
/// dismissed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A plain message with no icon.
    Plain,
    Done,
    Info,
    Warning,
    Failed,
    Loading,
}

/// A page a toast leads to, which the views turn into a destination.
#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    Song(SharedString),
    Album(SharedString),
    Artist(SharedString),
    Playlist(SharedString),
    /// The Playback tab of settings, where the Widevine module is managed.
    PlaybackSettings,
}

/// A button on a toast. Pressing it runs the handler, opens the target or the link if there is
/// one, and then dismisses the toast.
#[derive(Clone)]
pub struct ToastAction {
    /// The Fluent key of the button's label.
    pub key: SharedString,
    /// The page the button opens after the handler has run.
    pub target: Option<Target>,
    /// The web page the button opens in the browser after the handler has run.
    pub url: Option<SharedString>,
    run: Run,
}

impl ToastAction {
    pub fn new(key: impl Into<SharedString>, run: impl Fn(&mut App) + 'static) -> Self {
        Self {
            key: key.into(),
            target: None,
            url: None,
            run: Rc::new(run),
        }
    }

    pub fn run(&self, cx: &mut App) {
        (self.run)(cx)
    }
}

/// One toast, built with the methods below and shown with [`Toasts::post`].
#[derive(Clone)]
pub struct Toast {
    pub id: usize,
    pub outcome: Outcome,
    pub key: SharedString,
    pub name: Option<SharedString>,
    /// How many things the message is about, passed as `$count` so Fluent can pick a plural.
    pub count: Option<usize>,
    /// A line of its own below the text, such as the error behind a failure. Not translated.
    pub detail: Option<SharedString>,
    /// The Fluent key of a line below the text that explains it. `detail` takes the line when
    /// both are set.
    pub note: Option<SharedString>,
    pub target: Option<Target>,
    /// The emphasised button, such as Undo.
    pub action: Option<ToastAction>,
    /// The quiet button beside the action, such as Cancel.
    pub cancel: Option<ToastAction>,
    /// How far the work a loading toast waits on has got, from 0 to 1, which shows a filling
    /// ring in place of the spinner. `None` while there is no telling.
    pub progress: Option<f32>,
    /// Whether the message keeps to one line, cut short with an ellipsis, rather than wrapping.
    pub single_line: bool,
    /// Runs when the listener dismisses the toast by its cross or a swipe. Closing it from code
    /// or letting it expire runs nothing.
    dismissed: Option<Run>,
    /// How long the toast stays while nobody holds it, or `None` to stay until dismissed.
    linger: Option<Duration>,
    /// When the clock last started running.
    rest: Instant,
    /// Time the clock already ran before it was last paused.
    spent: Duration,
}

impl Toast {
    pub fn new(outcome: Outcome, key: impl Into<SharedString>) -> Self {
        Self {
            id: 0,
            outcome,
            key: key.into(),
            name: None,
            count: None,
            detail: None,
            note: None,
            target: None,
            action: None,
            cancel: None,
            progress: None,
            dismissed: None,
            single_line: false,
            linger: match outcome {
                Outcome::Loading => None,
                _ => Some(LINGER),
            },
            rest: Instant::now(),
            spent: Duration::ZERO,
        }
    }

    /// Fills the message's `$name` and shows it in bold.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn note(mut self, key: impl Into<SharedString>) -> Self {
        self.note = Some(key.into());
        self
    }

    /// Makes the bold name a link to the target's page.
    pub fn target(mut self, target: Option<Target>) -> Self {
        self.target = target;
        self
    }

    pub fn action(
        mut self,
        key: impl Into<SharedString>,
        run: impl Fn(&mut App) + 'static,
    ) -> Self {
        self.action = Some(ToastAction::new(key, run));
        self
    }

    pub fn cancel(
        mut self,
        key: impl Into<SharedString>,
        run: impl Fn(&mut App) + 'static,
    ) -> Self {
        self.cancel = Some(ToastAction::new(key, run));
        self
    }

    /// Adds a button that opens the target's page.
    pub fn open(mut self, key: impl Into<SharedString>, target: Target) -> Self {
        let mut action = ToastAction::new(key, |_| {});
        action.target = Some(target);
        self.action = Some(action);
        self
    }

    /// Adds a button that opens `url` in the browser.
    pub fn visit(mut self, key: impl Into<SharedString>, url: impl Into<SharedString>) -> Self {
        let mut action = ToastAction::new(key, |_| {});
        action.url = Some(url.into());
        self.action = Some(action);
        self
    }

    /// Adds an Undo button. `run` puts back what the toast reports was removed, and has to cope
    /// with everything that may have changed since, since the toast stays up for seconds.
    pub fn undo(self, run: impl Fn(&mut App) + 'static) -> Self {
        self.action("toast-undo", run)
    }

    /// Shows how far the work has got, from 0 to 1, as a ring that fills in place of the spinner.
    pub fn progress(mut self, done: f32) -> Self {
        self.progress = Some(done);
        self
    }

    /// Keeps the message to one line, for a toast that names something whose name may run long.
    pub fn single_line(mut self) -> Self {
        self.single_line = true;
        self
    }

    /// Runs `run` when the listener dismisses the toast, which makes the cross and a swipe
    /// undo what the toast reports, such as a download under way.
    pub fn on_dismiss(mut self, run: impl Fn(&mut App) + 'static) -> Self {
        self.dismissed = Some(Rc::new(run));
        self
    }

    /// Keeps the toast up until the user or the code dismisses it.
    pub fn lasting(mut self) -> Self {
        self.linger = None;
        self
    }

    /// How long a running clock still has to go, or `None` for a toast that never expires.
    fn left(&self) -> Option<Duration> {
        let ran = self.spent + self.rest.elapsed();
        self.linger.map(|linger| linger.saturating_sub(ran))
    }

    /// Whether the toast can act on or lead into the account it was shown for. A loading toast
    /// counts because whatever settles it may add an Undo.
    fn bound(&self) -> bool {
        self.action.is_some()
            || self.cancel.is_some()
            || self.target.is_some()
            || self.outcome == Outcome::Loading
    }

    fn same(&self, other: &Self) -> bool {
        self.outcome == other.outcome
            && self.key == other.key
            && self.name == other.name
            && self.count == other.count
            && self.detail == other.detail
            && self.note == other.note
    }
}

/// The toasts on screen, oldest first. Every toast's clock stops while the stack is paused,
/// which it is while the pointer is over it.
pub struct Toasts {
    shown: Vec<Toast>,
    next: usize,
    paused: bool,
    clocks: HashMap<usize, Task<()>>,
}

struct Installed(Entity<Toasts>);

impl Global for Installed {}

impl Toasts {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let toasts = cx.new(|_| Self {
                shown: Vec::new(),
                next: 0,
                paused: false,
                clocks: HashMap::new(),
            });
            cx.set_global(Installed(toasts));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn show(outcome: Outcome, key: impl Into<SharedString>, cx: &mut App) {
        Self::post(Toast::new(outcome, key), cx);
    }

    pub fn about(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        cx: &mut App,
    ) {
        Self::post(Toast::new(outcome, key).name(name), cx);
    }

    pub fn linked(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        target: Option<Target>,
        cx: &mut App,
    ) {
        Self::post(Toast::new(outcome, key).name(name).target(target), cx);
    }

    /// Shows a toast with `detail` on a line below the text, and `name`, when given, as `$name`.
    pub fn with_detail(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: Option<SharedString>,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) {
        let mut toast = Toast::new(outcome, key).detail(detail);
        toast.name = name;
        Self::post(toast, cx);
    }

    /// Shows a toast and returns its id, which [`Self::settle`] and [`Self::close`] take. Every
    /// call shows a new toast, except a failure or warning that says the same thing as one
    /// already up. Those can fire again on their own, so the one up restarts its clock instead
    /// and its id is returned.
    pub fn post(toast: Toast, cx: &mut App) -> usize {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| this.push(toast, cx))
    }

    /// Replaces what toast `id` says and restarts its clock, the way a loading toast turns into
    /// its result. Nothing happens when the toast is already gone.
    pub fn settle(id: usize, toast: Toast, cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| {
            let Some(at) = this.shown.iter().position(|shown| shown.id == id) else {
                return;
            };
            this.shown[at] = Toast {
                id,
                rest: Instant::now(),
                spent: Duration::ZERO,
                ..toast
            };
            this.arm(id, cx);
            cx.notify();
        });
    }

    /// Moves the progress toast `id` shows to `done`, from 0 to 1, leaving its clock and the rest
    /// of what it says alone. Nothing happens when the toast is already gone.
    pub fn progress(id: usize, done: f32, cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| {
            let Some(toast) = this.shown.iter_mut().find(|shown| shown.id == id) else {
                return;
            };
            if toast.progress != Some(done) {
                toast.progress = Some(done);
                cx.notify();
            }
        });
    }

    /// Dismisses toast `id` for the listener, by its cross or a swipe, then runs what the toast
    /// set with [`Toast::on_dismiss`]. It runs outside the stack's update, so it may post toasts.
    pub fn discard(id: usize, cx: &mut App) {
        let toasts = Self::entity(cx);
        let run = toasts
            .read(cx)
            .shown
            .iter()
            .find(|shown| shown.id == id)
            .and_then(|shown| shown.dismissed.clone());
        toasts.update(cx, |this, cx| this.dismiss(id, cx));
        if let Some(run) = run {
            run(cx);
        }
    }

    /// Dismisses toast `id` from outside the stack.
    pub fn close(id: usize, cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| this.dismiss(id, cx));
    }

    /// Dismisses every toast.
    pub fn clear(cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| {
            this.shown.clear();
            this.clocks.clear();
            cx.notify();
        });
    }

    /// Dismisses every toast bound to the account being signed out of. Plain messages stay, so
    /// one that explains the sign-out itself is still read.
    pub fn clear_account(cx: &mut App) {
        let toasts = Self::entity(cx);
        toasts.update(cx, |this, cx| {
            let clocks = &mut this.clocks;
            this.shown.retain(|toast| {
                let bound = toast.bound();
                if bound {
                    clocks.remove(&toast.id);
                }
                !bound
            });
            cx.notify();
        });
    }

    pub fn shown(&self) -> &[Toast] {
        &self.shown
    }

    pub fn dismiss(&mut self, id: usize, cx: &mut Context<Self>) {
        self.shown.retain(|toast| toast.id != id);
        self.clocks.remove(&id);
        cx.notify();
    }

    /// Stops or restarts every toast's clock. A restarted clock runs only the time it had left.
    pub fn pause(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        let now = Instant::now();
        for toast in &mut self.shown {
            match paused {
                true => toast.spent += now.duration_since(toast.rest),
                false => toast.rest = now,
            }
        }
        match paused {
            true => self.clocks.clear(),
            false => {
                let ids: Vec<usize> = self.shown.iter().map(|toast| toast.id).collect();
                for id in ids {
                    self.arm(id, cx);
                }
            }
        }
    }

    fn push(&mut self, toast: Toast, cx: &mut Context<Self>) -> usize {
        let repeats = matches!(toast.outcome, Outcome::Failed | Outcome::Warning);
        if let Some(shown) = self
            .shown
            .iter_mut()
            .find(|shown| repeats && shown.same(&toast))
        {
            let id = shown.id;
            shown.rest = Instant::now();
            shown.spent = Duration::ZERO;
            self.arm(id, cx);
            return id;
        }

        let id = self.next;
        self.next += 1;
        self.shown.push(Toast {
            id,
            rest: Instant::now(),
            spent: Duration::ZERO,
            ..toast
        });
        self.arm(id, cx);
        cx.notify();
        id
    }

    /// Starts the clock that dismisses toast `id` once the time it has left runs out. A paused
    /// stack and a toast that never expires get no clock.
    fn arm(&mut self, id: usize, cx: &mut Context<Self>) {
        self.clocks.remove(&id);
        if self.paused {
            return;
        }
        let Some(toast) = self.shown.iter().find(|toast| toast.id == id) else {
            return;
        };
        let Some(wait) = toast.left() else {
            return;
        };
        let clock = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.dismiss(id, cx)).ok();
        });
        self.clocks.insert(id, clock);
    }
}
