use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use gpui::{Context, Entity, EventEmitter, Task};
use i18n::t;
use music::{
    MusicApi, MusicProvider, PlaybackFactory, PromptSink, ProviderSession, Shape, SignIn,
    SignInFailure, SignInProblem, SignInPrompt, UserProfile,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::Shelf;
use crate::catalog::CatalogSource;
use crate::settings::AppSettings;
use crate::{Io, join};

const HEARTBEAT: Duration = Duration::from_secs(30);
/// How often the sign-in window is asked whether the user is through.
const WINDOW_POLL: Duration = Duration::from_millis(300);
const BACKOFF: [Duration; 5] = [
    Duration::ZERO,
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(300),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub problem: Option<SignInProblem>,
    pub summary: String,
    pub detail: Option<String>,
}

impl Failure {
    fn new(error: &Error) -> Self {
        let problem = error
            .downcast_ref::<SignInFailure>()
            .map(|failure| failure.0);
        let detail = error
            .chain()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        Self {
            problem,
            summary: error.to_string(),
            detail: (!detail.is_empty()).then(|| detail.join(": ")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    SignedOut,
    Restoring,
    Authorizing(Option<SignInPrompt>),
    SignedIn(UserProfile),
    Failed(Failure),
}

pub enum SessionEvent {
    SignedIn,
    SignedOut,
    Reconnected,
    LocalChanged,
}

#[derive(Clone, Copy)]
enum SecretInput {
    Browser,
    Manual,
}

pub struct ProviderInfo {
    pub slug: &'static str,
    pub name: &'static str,
    pub options: Vec<SignIn>,
    pub web_sign_in: bool,
    pub web_sign_in_label: Option<&'static str>,
    pub stored: bool,
    pub active: bool,
    pub pending: bool,
    pub error: Option<Failure>,
}

pub struct Session {
    state: SessionState,
    providers: Vec<Arc<dyn MusicProvider>>,
    active: Option<usize>,
    awaiting: Option<usize>,
    resume: Option<(usize, UserProfile)>,
    error: Option<(usize, Failure)>,
    settings: Entity<AppSettings>,
    client: Option<Arc<dyn MusicApi>>,
    catalog: Option<Arc<CatalogSource>>,
    playback: Option<Arc<dyn PlaybackFactory>>,
    shape: Shape,
    authenticated: bool,
    playcounts: bool,
    io: Io,
    task: Option<Task<()>>,
    prompt_task: Option<Task<()>>,
    input: Option<UnboundedSender<String>>,
    /// The browser window a `SignInPrompt::Secret` opened, while it is up.
    window: Option<webview::Login>,
    window_task: Option<Task<()>>,
    local_provider: Arc<dyn MusicProvider>,
    local_folders: Vec<PathBuf>,
    local_client: Option<Arc<dyn MusicApi>>,
    local_catalog: Option<Arc<CatalogSource>>,
    local_playback: Option<Arc<dyn PlaybackFactory>>,
    local_task: Option<Task<()>>,
    watch: Option<Task<()>>,
    reconnect: Option<Task<()>>,
    reconnecting: bool,
    attempt: usize,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn new(
        providers: Vec<Arc<dyn MusicProvider>>,
        local_provider: Arc<dyn MusicProvider>,
        settings: Entity<AppSettings>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let remembered = settings.read(cx).provider().to_string();
        let local_folders = settings.read(cx).local_folders().to_vec();
        let active = providers
            .iter()
            .position(|provider| provider.slug() == remembered);
        let mut session = Self {
            state: SessionState::SignedOut,
            providers,
            active,
            awaiting: None,
            resume: None,
            error: None,
            settings,
            client: None,
            catalog: None,
            playback: None,
            shape: Shape::Saved,
            authenticated: false,
            playcounts: false,
            io,
            task: None,
            prompt_task: None,
            input: None,
            window: None,
            window_task: None,
            local_provider,
            local_folders,
            local_client: None,
            local_catalog: None,
            local_playback: None,
            local_task: None,
            watch: None,
            reconnect: None,
            reconnecting: false,
            attempt: 0,
        };
        session.restore_local(cx);
        session
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn client(&self) -> Option<Arc<dyn MusicApi>> {
        self.client.clone()
    }

    pub fn playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        self.playback.clone()
    }

    pub fn local_client(&self) -> Option<Arc<dyn MusicApi>> {
        self.local_client.clone()
    }

    /// The client serving a shelf, if that shelf has a provider right now.
    pub fn client_of(&self, shelf: Shelf) -> Option<Arc<dyn MusicApi>> {
        match shelf {
            Shelf::Streaming => self.client.clone(),
            Shelf::Local => self.local_client.clone(),
        }
    }

    /// What a shelf's library is made of. The local shelf is always a catalog.
    pub fn shape_of(&self, shelf: Shelf) -> Shape {
        match shelf {
            Shelf::Streaming => self.shape,
            Shelf::Local => Shape::Catalog,
        }
    }

    pub(crate) fn catalog(&self, id: &str) -> Option<Arc<CatalogSource>> {
        match music::is_local_id(id) {
            true => self.local_catalog.clone(),
            false => self.catalog.clone(),
        }
    }

    pub fn local_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        self.local_playback.clone()
    }

    pub fn local_paths(&self) -> Vec<String> {
        self.local_folders
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    pub fn providers(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers.iter().enumerate().map(|(index, provider)| {
            let browser = provider.web_sign_in();
            // Asked in this order because answering `supported` costs a library load on
            // Linux, and only a provider that signs in with cookies is worth it.
            let web_sign_in = browser.is_some() && webview::supported();
            ProviderInfo {
                slug: provider.slug(),
                name: provider.name(),
                options: provider.sign_in_options(),
                web_sign_in,
                web_sign_in_label: web_sign_in
                    .then(|| browser.and_then(|sign_in| sign_in.label))
                    .flatten(),
                stored: provider.stored(),
                active: self.active == Some(index),
                pending: self.awaiting == Some(index),
                error: match &self.error {
                    Some((failed, failure)) if *failed == index => Some(failure.clone()),
                    _ => None,
                },
            }
        })
    }

    pub fn connected(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers().filter(|info| info.stored)
    }

    pub fn forget(&mut self, slug: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        if self.active == Some(index) {
            return self.sign_out(cx);
        }
        self.providers[index].sign_out();
        cx.notify();
    }

    pub fn switch(&mut self, slug: &str, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        if self.active == Some(index) && matches!(self.state, SessionState::SignedIn(_)) {
            return;
        }
        self.release(cx);
        self.active = Some(index);
        self.restore(cx);
    }

    pub fn provider_name(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.name())
    }

    pub fn provider_slug(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.slug())
    }

    pub fn local_slug(&self) -> &'static str {
        self.local_provider.slug()
    }

    pub fn active_slugs(&self) -> Vec<&'static str> {
        let mut slugs = Vec::new();
        if self.client.is_some() {
            slugs.extend(self.provider_slug());
        }
        if self.local_client.is_some() {
            slugs.push(self.local_slug());
        }
        slugs
    }

    pub fn slug_for(&self, id: &str) -> Option<&'static str> {
        self.provider_for(id).map(|provider| provider.slug())
    }

    /// The provider an id belongs to, whichever shelf it sits on.
    pub(crate) fn provider_for(&self, id: &str) -> Option<&dyn MusicProvider> {
        match music::is_local_id(id) {
            true => Some(self.local_provider.as_ref()),
            false => self.active.map(|index| self.providers[index].as_ref()),
        }
    }

    pub fn authenticated(&self) -> bool {
        self.authenticated
    }

    pub fn playcounts(&self) -> bool {
        self.playcounts
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.state,
            SessionState::Restoring | SessionState::Authorizing(_)
        )
    }

    pub fn restore(&mut self, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        let Some(active) = self.active else {
            self.state = SessionState::SignedOut;
            cx.notify();
            cx.emit(SessionEvent::SignedOut);
            return;
        };
        self.state = SessionState::Restoring;
        cx.notify();

        let provider = self.providers[active].clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let restored = join(io.spawn(async move { provider.restore().await })).await;

            this.update(cx, |this, cx| match restored {
                Ok(Some(session)) => this.signed_in(session, active, cx),
                Ok(None) => {
                    this.state = SessionState::SignedOut;
                    cx.notify();
                    cx.emit(SessionEvent::SignedOut);
                }
                Err(error) => this.failed(&error, cx),
            })
            .ok();
        }));
    }

    pub fn sign_in(&mut self, slug: &str, method: SignIn, cx: &mut Context<Self>) {
        self.start_sign_in(slug, method, SecretInput::Browser, cx);
    }

    pub fn sign_in_with_cookies(&mut self, slug: &str, cx: &mut Context<Self>) {
        self.start_sign_in(slug, SignIn::Secret, SecretInput::Manual, cx);
    }

    fn start_sign_in(
        &mut self,
        slug: &str,
        method: SignIn,
        secret_input: SecretInput,
        cx: &mut Context<Self>,
    ) {
        if self.is_pending() {
            return;
        }
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        self.resume = match &self.state {
            SessionState::SignedIn(profile) => self.active.map(|active| (active, profile.clone())),
            _ => None,
        };
        self.error = None;
        self.awaiting = Some(index);
        self.state = SessionState::Authorizing(None);
        cx.notify();

        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        self.input = Some(input_tx);
        let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::unbounded_channel::<SignInPrompt>();
        self.prompt_task = Some(cx.spawn(async move |this, cx| {
            while let Some(prompt) = prompt_rx.recv().await {
                this.update(cx, |this, cx| {
                    if matches!(this.state, SessionState::Authorizing(_)) {
                        let secret = matches!(prompt, SignInPrompt::Secret);
                        this.state = SessionState::Authorizing(Some(prompt));
                        cx.notify();
                        if secret && matches!(secret_input, SecretInput::Browser) {
                            this.open_window(cx);
                        }
                    }
                })
                .ok();
            }
        }));
        let prompt: PromptSink = Arc::new(move |prompt| {
            prompt_tx.send(prompt).ok();
        });

        let provider = self.providers[index].clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let authorized =
                join(io.spawn(async move { provider.sign_in(method, prompt, input_rx).await }))
                    .await;

            this.update(cx, |this, cx| {
                this.prompt_task = None;
                this.input = None;
                this.window = None;
                match authorized {
                    Ok(session) => this.signed_in(session, index, cx),
                    Err(error) => this.failed(&error, cx),
                }
            })
            .ok();
        }));
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, SessionState::Authorizing(_)) {
            return;
        }
        if let Some(index) = self.awaiting {
            let provider = self.providers[index].clone();
            self.io.spawn(async move { provider.abandon() });
        }
        self.task = None;
        self.prompt_task = None;
        self.input = None;
        self.window = None;
        self.window_task = None;
        self.awaiting = None;
        self.error = None;
        if let Some((index, profile)) = self.resume.take() {
            self.active = Some(index);
            self.state = SessionState::SignedIn(profile);
            cx.notify();
            return;
        }
        self.state = SessionState::SignedOut;
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    /// Opens the browser window for the secret prompt now showing. The window answers the prompt
    /// itself once the user is through; closing it cancels the sign-in.
    fn open_window(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.state,
            SessionState::Authorizing(Some(SignInPrompt::Secret))
        ) || self.window.is_some()
        {
            return;
        }
        let Some(index) = self.awaiting else {
            return;
        };
        let provider = &self.providers[index];
        let Some(sign_in) = provider.web_sign_in() else {
            return;
        };
        let target = webview::Target {
            url: sign_in.url.to_string(),
            landing: sign_in.landing.to_string(),
            domain: sign_in.domain.to_string(),
            proof: sign_in.proof.iter().map(ToString::to_string).collect(),
            title: t!("login-window-title", provider = provider.name()).to_string(),
        };
        match webview::Login::open(target) {
            Ok(login) => self.window = Some(login),
            Err(error) => {
                log::warn!("session: cannot open the sign-in window: {error:#}");
                // Dropping the provider's task closes its prompt channel, which ends the prompt
                // task on its own; this runs inside that task, so it must not drop it here.
                self.task = None;
                self.input = None;
                return self.failed(&error, cx);
            }
        }
        self.window_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WINDOW_POLL).await;
                let open = this.update(cx, |this, cx| {
                    let Some(login) = this.window.as_mut() else {
                        return false;
                    };
                    match login.poll() {
                        webview::Poll::Pending => true,
                        webview::Poll::Closed => {
                            this.window = None;
                            this.cancel_sign_in(cx);
                            false
                        }
                        webview::Poll::Cookies(header) => {
                            this.window = None;
                            this.submit_input(header, cx);
                            false
                        }
                    }
                });
                if !open.unwrap_or(false) {
                    break;
                }
            }
        }));
    }

    pub fn submit_input(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(input) = &self.input {
            self.window = None;
            input.send(text).ok();
            if let SessionState::Authorizing(Some(
                SignInPrompt::Secret | SignInPrompt::Accounts(_),
            )) = &self.state
            {
                self.state = SessionState::Authorizing(None);
                cx.notify();
            }
        }
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active {
            self.providers[active].sign_out();
        }
        self.release(cx);
        if let Some(index) = self.remaining() {
            self.active = Some(index);
            self.restore(cx);
        }
    }

    fn remaining(&self) -> Option<usize> {
        self.active
            .filter(|index| self.providers[*index].stored())
            .or_else(|| self.providers.iter().position(|provider| provider.stored()))
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.watch = None;
        self.reconnect = None;
        self.reconnecting = false;
        self.attempt = 0;
        self.prompt_task = None;
        self.input = None;
        self.window = None;
        self.window_task = None;
        self.awaiting = None;
        self.resume = None;
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.playcounts = false;
        self.state = SessionState::SignedOut;
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    fn signed_in(&mut self, session: ProviderSession, index: usize, cx: &mut Context<Self>) {
        let replaced = self
            .resume
            .take()
            .is_some_and(|(held, profile)| held != index || profile.id != session.profile.id);
        if replaced {
            self.drop_previous_session(cx);
        }
        self.active = Some(index);
        self.awaiting = None;
        self.error = None;
        let slug = self.providers[index].slug();
        self.settings.update(cx, |settings, cx| {
            settings.set_provider(slug, cx);
        });
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.playcounts = session.playcounts;
        self.state = SessionState::SignedIn(session.profile);
        self.attempt = 0;
        self.start_heartbeat(cx);
        cx.notify();
        cx.emit(SessionEvent::SignedIn);
    }

    fn drop_previous_session(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.playcounts = false;
        self.watch = None;
        self.reconnect = None;
        self.reconnecting = false;
        self.attempt = 0;
        cx.emit(SessionEvent::SignedOut);
    }

    fn start_heartbeat(&mut self, cx: &mut Context<Self>) {
        self.watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(HEARTBEAT).await;
                if this
                    .update(cx, |this, cx| this.reconnect_if_stale(cx))
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    pub fn reconnect_if_stale(&mut self, cx: &mut Context<Self>) -> bool {
        if self.reconnecting {
            return true;
        }
        if !matches!(self.state, SessionState::SignedIn(_)) {
            return false;
        }
        let Some(client) = &self.client else {
            return false;
        };
        if client.alive() {
            self.attempt = 0;
            return false;
        }
        let Some(active) = self.active else {
            return false;
        };
        let wait = BACKOFF[self.attempt.min(BACKOFF.len() - 1)];
        self.attempt += 1;
        self.reconnecting = true;
        log::warn!(
            "session: the {} session went stale, reconnecting in {}s",
            self.providers[active].name(),
            wait.as_secs()
        );
        let provider = self.providers[active].clone();
        let io = self.io.clone();
        self.reconnect = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let restored = join(io.spawn(async move { provider.restore().await })).await;
            this.update(cx, |this, cx| {
                this.reconnecting = false;
                match restored {
                    Ok(Some(session)) => this.reconnected(session, cx),
                    Ok(None) => log::warn!("session: nothing stored to reconnect with"),
                    Err(error) => log::warn!("session: cannot reconnect: {error:#}"),
                }
            })
            .ok();
        }));
        true
    }

    fn reconnected(&mut self, session: ProviderSession, cx: &mut Context<Self>) {
        self.attempt = 0;
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.playcounts = session.playcounts;
        log::debug!("session: reconnected");
        cx.notify();
        cx.emit(SessionEvent::Reconnected);
    }

    fn failed(&mut self, error: &Error, cx: &mut Context<Self>) {
        let failure = Failure::new(error);
        if let Some(failed) = self.awaiting.or(self.active) {
            self.error = Some((failed, failure.clone()));
        }
        self.awaiting = None;
        if let Some((index, profile)) = self.resume.take() {
            self.active = Some(index);
            self.state = SessionState::SignedIn(profile);
            cx.notify();
            return;
        }
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.state = SessionState::Failed(failure);
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    fn restore_local(&mut self, cx: &mut Context<Self>) {
        if self.local_folders.is_empty() {
            return;
        }
        self.rescan_local(cx);
    }

    /// Adds a folder to the local library, then rescans every configured folder together so
    /// artists and albums that span more than one root merge into one, seamlessly.
    pub fn add_local_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.add_local_folders(vec![path], cx);
    }

    /// Same as [`Session::add_local_folder`], for a batch picked in one native dialog.
    pub fn add_local_folders(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut folders = self.local_folders.clone();
        for path in paths {
            if folders.iter().any(|existing| overlaps(existing, &path)) {
                log::warn!(
                    "session: {} overlaps an already-added local folder",
                    path.display()
                );
                continue;
            }
            folders.push(path);
        }
        if folders.len() == self.local_folders.len() {
            return;
        }
        self.set_local_folders(folders, cx);
    }

    pub fn remove_local_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut folders = self.local_folders.clone();
        let before = folders.len();
        folders.retain(|existing| existing != path);
        if folders.len() == before {
            return;
        }
        self.set_local_folders(folders, cx);
    }

    /// Rescans every configured local folder without changing the list, e.g. after files
    /// changed on disk or a tag was edited.
    pub fn rescan_local(&mut self, cx: &mut Context<Self>) {
        self.set_local_folders(self.local_folders.clone(), cx);
    }

    fn set_local_folders(&mut self, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        if folders.is_empty() {
            self.local_provider.sign_out();
            self.local_folders = Vec::new();
            self.settings.update(cx, |settings, cx| {
                settings.set_local_folders(Vec::new(), cx)
            });
            self.local_client = None;
            self.local_catalog = None;
            self.local_playback = None;
            self.local_task = None;
            cx.notify();
            cx.emit(SessionEvent::LocalChanged);
            return;
        }

        let provider = self.local_provider.clone();
        let chosen = folders.clone();
        let io = self.io.clone();
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let prompt: PromptSink = Arc::new(|_| {});
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let signed_in = join(
                io.spawn(async move { provider.sign_in(SignIn::Path(folders), prompt, rx).await }),
            )
            .await;

            this.update(cx, |this, cx| match signed_in {
                Ok(session) => {
                    this.local_folders = chosen.clone();
                    this.settings
                        .update(cx, |settings, cx| settings.set_local_folders(chosen, cx));
                    this.local_signed_in(session, cx);
                }
                Err(error) => {
                    log::warn!("session: cannot update local music folders: {error:#}");
                }
            })
            .ok();
        }));
    }

    /// Brings up the local engine even when no folder has ever been configured, so a
    /// file-association open can play a track without the user having visited Settings first.
    /// A no-op once a local client already exists or one is already being brought up; a
    /// configured library goes through the normal [`Session::rescan_local`] path instead.
    pub fn ensure_local_ready(&mut self, cx: &mut Context<Self>) {
        if self.local_client.is_some() || self.local_task.is_some() {
            return;
        }
        if !self.local_folders.is_empty() {
            return self.rescan_local(cx);
        }

        let provider = self.local_provider.clone();
        let io = self.io.clone();
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let prompt: PromptSink = Arc::new(|_| {});
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let signed_in = join(io.spawn(async move {
                provider.sign_in(SignIn::Path(Vec::new()), prompt, rx).await
            }))
            .await;

            this.update(cx, |this, cx| match signed_in {
                Ok(session) => this.local_signed_in(session, cx),
                Err(error) => {
                    log::warn!("session: cannot initialize local playback: {error:#}");
                }
            })
            .ok();
        }));
    }

    fn local_signed_in(&mut self, session: ProviderSession, cx: &mut Context<Self>) {
        self.local_catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.local_client = Some(session.api);
        self.local_playback = Some(session.playback);
        cx.notify();
        cx.emit(SessionEvent::LocalChanged);
    }
}

/// Whether `a` and `b` are the same directory, or one contains the other — either way, scanning
/// both would double-count the tracks they share.
fn overlaps(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b || a.starts_with(&b) || b.starts_with(&a)
}
