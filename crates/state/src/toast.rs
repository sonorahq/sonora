use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use i18n::SharedString;

use crate::{Change, Cx, Event, Job, SessionEvent, Store};

const LINGER: Duration = Duration::from_secs(4);

/// The id the next toast gets. Ids are handed out as a toast is built rather than when it lands
/// on the stack, so a model that only raises an event still knows which toast it raised.
static NEXT: AtomicUsize = AtomicUsize::new(0);

type Run = Arc<dyn Fn(&mut Store) + Send + Sync>;

/// What a toast reports, which picks its icon. `Loading` stays up until it is settled or
/// dismissed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Song(SharedString),
    Album(SharedString),
    Artist(SharedString),
    Playlist(SharedString),
    /// The Playback tab of settings, where the Widevine module is managed.
    PlaybackSettings,
    /// The Providers tab of settings, where a provider is signed in again.
    ProviderSettings,
}

/// Which of a toast's two buttons was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastButton {
    /// The emphasised one, such as Undo.
    Action,
    /// The quiet one beside it, such as Cancel.
    Cancel,
}

/// A change to a toast already raised, for a model that holds only a [`Cx`]. The stack applies
/// it once the change that raised it settles.
#[derive(Clone, Debug)]
pub enum ToastUpdate {
    /// See [`Toasts::settle`].
    Settle(usize, Box<Toast>),
    /// See [`Toasts::progress`].
    Progress(usize, f32),
    /// See [`Toasts::close`].
    Close(usize),
}

/// A handler a toast runs on the UI thread with the whole store. It has to cope with
/// everything that may have changed since the toast went up.
#[derive(Clone)]
struct Handler(Run);

impl fmt::Debug for Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handler")
    }
}

/// A button on a toast. Pressing it runs the handler, opens the target or the link if there is
/// one, and then dismisses the toast.
#[derive(Clone, Debug)]
pub struct ToastAction {
    /// The Fluent key of the button's label.
    pub key: SharedString,
    /// The page the button opens after the handler has run.
    pub target: Option<Target>,
    /// The web page the button opens in the browser after the handler has run.
    pub url: Option<SharedString>,
    run: Handler,
}

impl ToastAction {
    pub fn new(
        key: impl Into<SharedString>,
        run: impl Fn(&mut Store) + Send + Sync + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            target: None,
            url: None,
            run: Handler(Arc::new(run)),
        }
    }

    pub fn run(&self, store: &mut Store) {
        (self.run.0)(store)
    }
}

/// One toast, built with the methods below and shown with [`Toasts::post`].
#[derive(Clone, Debug)]
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
    dismissed: Option<Handler>,
    /// How long the toast stays while nobody holds it, or `None` to stay until dismissed.
    linger: Option<Duration>,
    /// When the clock last started running.
    rest: Instant,
    /// Time the clock already ran before it was last paused.
    spent: Duration,
    /// The ids of repeats that were folded into this toast rather than shown, so whoever raised
    /// one of them still reaches this one by its id.
    also: Vec<usize>,
}

impl Toast {
    pub fn new(outcome: Outcome, key: impl Into<SharedString>) -> Self {
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
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
            also: Vec::new(),
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

    /// Adds a second line. It goes through [`music::redact`] first, since the detail is often an
    /// error that names the url it failed on, and people share screenshots of toasts.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        let detail = detail.into();
        self.detail = Some(match music::redact(&detail) {
            Cow::Borrowed(_) => detail,
            Cow::Owned(clean) => clean.into(),
        });
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
        run: impl Fn(&mut Store) + Send + Sync + 'static,
    ) -> Self {
        self.action = Some(ToastAction::new(key, run));
        self
    }

    pub fn cancel(
        mut self,
        key: impl Into<SharedString>,
        run: impl Fn(&mut Store) + Send + Sync + 'static,
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
    pub fn undo(self, run: impl Fn(&mut Store) + Send + Sync + 'static) -> Self {
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
    pub fn on_dismiss(mut self, run: impl Fn(&mut Store) + Send + Sync + 'static) -> Self {
        self.dismissed = Some(Handler(Arc::new(run)));
        self
    }

    /// Keeps the toast up until the user or the code dismisses it.
    pub fn lasting(mut self) -> Self {
        self.linger = None;
        self
    }

    /// Whether `id` names this toast, as its own id or as one of a repeat folded into it.
    fn answers(&self, id: usize) -> bool {
        self.id == id || self.also.contains(&id)
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
///
/// A model raises a toast with [`Toasts::post`] and its siblings, which only need a [`Cx`]: the
/// toast travels as [`Event::Toast`] and lands on the stack once the change settles. Views and
/// `Store` methods reach the stack directly at `store.toasts`.
pub struct Toasts {
    shown: Vec<Toast>,
    paused: bool,
    clocks: HashMap<usize, Job>,
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            shown: Vec::new(),
            paused: false,
            clocks: HashMap::new(),
        }
    }

    pub fn show(outcome: Outcome, key: impl Into<SharedString>, cx: &mut Cx) {
        Self::post(Toast::new(outcome, key), cx);
    }

    pub fn about(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        cx: &mut Cx,
    ) {
        Self::post(Toast::new(outcome, key).name(name), cx);
    }

    pub fn linked(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: impl Into<SharedString>,
        target: Option<Target>,
        cx: &mut Cx,
    ) {
        Self::post(Toast::new(outcome, key).name(name).target(target), cx);
    }

    /// Shows a toast with `detail` on a line below the text, and `name`, when given, as `$name`.
    pub fn with_detail(
        outcome: Outcome,
        key: impl Into<SharedString>,
        name: Option<SharedString>,
        detail: impl Into<SharedString>,
        cx: &mut Cx,
    ) {
        let mut toast = Toast::new(outcome, key).detail(detail);
        toast.name = name;
        Self::post(toast, cx);
    }

    /// Shows a toast and returns its id, which [`Self::settle`] and [`Self::close`] take. Every
    /// call shows a new toast, except a failure or warning that says the same thing as one
    /// already up. Those can fire again on their own, so the one up restarts its clock instead
    /// and answers to this id too. The toast lands once the current change settles, so
    /// [`Self::showing`] does not see it before then.
    pub fn post(toast: Toast, cx: &mut Cx) -> usize {
        let id = toast.id;
        cx.emit(Event::Toast(toast));
        id
    }

    /// Replaces what toast `id` says and restarts its clock, the way a loading toast turns into
    /// its result. Nothing happens when the toast is already gone.
    pub fn settle(id: usize, toast: Toast, cx: &mut Cx) {
        cx.emit(Event::ToastUpdate(ToastUpdate::Settle(id, Box::new(toast))));
    }

    /// Moves the progress toast `id` shows to `done`, from 0 to 1, leaving its clock and the rest
    /// of what it says alone. Nothing happens when the toast is already gone.
    pub fn progress(id: usize, done: f32, cx: &mut Cx) {
        cx.emit(Event::ToastUpdate(ToastUpdate::Progress(id, done)));
    }

    /// Dismisses toast `id` from outside the stack.
    pub fn close(id: usize, cx: &mut Cx) {
        cx.emit(Event::ToastUpdate(ToastUpdate::Close(id)));
    }

    pub fn shown(&self) -> &[Toast] {
        &self.shown
    }

    /// Whether toast `id` is still on screen.
    pub fn showing(&self, id: usize) -> bool {
        self.shown.iter().any(|toast| toast.answers(id))
    }

    /// Puts `toast` on the stack now and returns the id it is reached by. A failure or warning
    /// that says the same thing as one already up restarts that one's clock instead.
    pub fn push(&mut self, toast: Toast, cx: &Cx) -> usize {
        let repeats = matches!(toast.outcome, Outcome::Failed | Outcome::Warning);
        if let Some(shown) = self
            .shown
            .iter_mut()
            .find(|shown| repeats && shown.same(&toast))
        {
            let id = shown.id;
            shown.also.push(toast.id);
            shown.rest = Instant::now();
            shown.spent = Duration::ZERO;
            self.arm(id, cx);
            return id;
        }

        let id = toast.id;
        self.shown.push(Toast {
            rest: Instant::now(),
            spent: Duration::ZERO,
            ..toast
        });
        self.arm(id, cx);
        id
    }

    /// [`Self::settle`] applied now.
    pub fn replace(&mut self, id: usize, toast: Toast, cx: &Cx) {
        let Some(at) = self.shown.iter().position(|shown| shown.answers(id)) else {
            return;
        };
        let held = &self.shown[at];
        let (id, mut also) = (held.id, held.also.clone());
        also.push(toast.id);
        self.shown[at] = Toast {
            id,
            also,
            rest: Instant::now(),
            spent: Duration::ZERO,
            ..toast
        };
        self.arm(id, cx);
    }

    /// [`Self::progress`] applied now.
    pub fn set_progress(&mut self, id: usize, done: f32) {
        let Some(toast) = self.shown.iter_mut().find(|shown| shown.answers(id)) else {
            return;
        };
        toast.progress = Some(done);
    }

    pub fn dismiss(&mut self, id: usize) {
        let Some(at) = self.shown.iter().position(|toast| toast.answers(id)) else {
            return;
        };
        let toast = self.shown.remove(at);
        self.clocks.remove(&toast.id);
    }

    /// Dismisses every toast.
    pub fn clear(&mut self) {
        self.shown.clear();
        self.clocks.clear();
    }

    /// Dismisses every toast bound to the account being signed out of. Plain messages stay, so
    /// one that explains the sign-out itself is still read.
    pub fn clear_account(&mut self) {
        let clocks = &mut self.clocks;
        self.shown.retain(|toast| {
            let bound = toast.bound();
            if bound {
                clocks.remove(&toast.id);
            }
            !bound
        });
    }

    /// Stops or restarts every toast's clock. A restarted clock runs only the time it had left.
    pub fn pause(&mut self, paused: bool, cx: &Cx) {
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

    /// Starts the clock that dismisses toast `id` once the time it has left runs out. A paused
    /// stack and a toast that never expires get no clock.
    fn arm(&mut self, id: usize, cx: &Cx) {
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
        let clock = cx.after(wait, move |store| store.toasts.dismiss(id));
        self.clocks.insert(id, clock);
    }
}

impl Store {
    /// Dismisses toast `id` for the listener, by its cross or a swipe, then runs what the toast
    /// set with [`Toast::on_dismiss`]. It runs after the stack let go of the toast, so it may
    /// post toasts.
    pub fn discard_toast(&mut self, id: usize) {
        let run = self
            .toasts
            .shown
            .iter()
            .find(|shown| shown.answers(id))
            .and_then(|shown| shown.dismissed.clone());
        self.toasts.dismiss(id);
        if let Some(run) = run {
            (run.0)(self);
        }
    }

    /// Presses one of toast `id`'s buttons: runs its handler and dismisses the toast. Answers the
    /// button pressed, whose `target` the view then navigates to and whose `url` it opens, or
    /// `None` when the toast or the button is gone.
    pub fn press_toast(&mut self, id: usize, button: ToastButton) -> Option<ToastAction> {
        let toast = self.toasts.shown.iter().find(|shown| shown.answers(id))?;
        let action = match button {
            ToastButton::Action => toast.action.clone(),
            ToastButton::Cancel => toast.cancel.clone(),
        }?;
        action.run(self);
        self.toasts.dismiss(id);
        Some(action)
    }

    pub(crate) fn react_toasts(&mut self, change: &Change) {
        let Store { toasts, cx, .. } = self;
        match change {
            Change::Event(Event::Toast(toast)) => {
                toasts.push(toast.clone(), cx);
            }
            Change::Event(Event::ToastUpdate(update)) => match update {
                ToastUpdate::Settle(id, toast) => toasts.replace(*id, (**toast).clone(), cx),
                ToastUpdate::Progress(id, done) => toasts.set_progress(*id, *done),
                ToastUpdate::Close(id) => toasts.dismiss(*id),
            },
            // A toast can carry an Undo that acts on the account it was shown for, so no such
            // toast outlives it. A plain one stays, since the sign-out can be what it explains.
            Change::Event(Event::Session(SessionEvent::SignedOut)) => toasts.clear_account(),
            _ => {}
        }
    }
}
