use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use i18n::t;
use music::link::Linked;
use music::{
    Capabilities, Catalog, MusicApi, MusicProvider, PlaybackFactory, PromptSink, ProviderSession,
    Shape, SignIn, SignInFailure, SignInProblem, SignInPrompt, UserProfile,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::catalog::CatalogSource;
use crate::settings::AppSettings;
use crate::{Change, Cx, Event, Job, Outcome, Shelf, Store, Target, Toast, Toasts, Topic, join};

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
        let reason = format!("{error:#}");
        let problem = error
            .downcast_ref::<SignInFailure>()
            .map(|failure| failure.0)
            .or_else(|| music::trouble::offline(&reason).then_some(SignInProblem::Network));
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

    /// Whether the sign-in failed because there was no network, rather than because the account
    /// was refused. A provider that was only unreachable is still the user's.
    pub fn offline(&self) -> bool {
        self.problem == Some(SignInProblem::Network)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    SignedOut,
    Restoring,
    Authorizing(Option<SignInPrompt>),
    SignedIn(UserProfile),
    /// The stored account could not be reached. It is still the user's, so nothing is signed
    /// out: the app stays on whatever the library kept and on the local files, and tries the
    /// account again by itself once the network is back.
    Offline(Failure),
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    SignedIn,
    SignedOut,
    Reconnected,
    /// Offline mode let go of a live account. What was loaded stays, but nothing reaches the
    /// network until the account signs in again.
    WentOffline,
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
    pub protected: bool,
    pub stored: bool,
    /// Whether what is stored is an anonymous session rather than an account.
    pub guest: bool,
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
    /// The providers a restore has already given up on, so falling back never comes round to
    /// one of them again. A sign-in that lands empties it.
    passed: Vec<usize>,
    error: Option<(usize, Failure)>,
    client: Option<Arc<dyn MusicApi>>,
    catalog: Option<Arc<CatalogSource>>,
    playback: Option<Arc<dyn PlaybackFactory>>,
    shape: Shape,
    authenticated: bool,
    capabilities: Capabilities,
    task: Option<Job>,
    prompt_task: Option<Job>,
    input: Option<UnboundedSender<String>>,
    /// The browser window a `SignInPrompt::Secret` opened, while it is up.
    window: Option<webview::Page>,
    window_task: Option<Job>,
    local_provider: Arc<dyn MusicProvider>,
    local_folders: Vec<PathBuf>,
    local_client: Option<Arc<dyn MusicApi>>,
    local_catalog: Option<Arc<CatalogSource>>,
    local_playback: Option<Arc<dyn PlaybackFactory>>,
    local_capabilities: Capabilities,
    local_task: Option<Job>,
    /// Whether a local scan is under way, so the UI can show its progress.
    scanning: bool,
    /// Whether the scan under way is one the user started, so its end is reported. An automatic
    /// rescan that cancels it takes the report over.
    scan_asked: bool,
    watch: Option<Job>,
    reconnect: Option<Job>,
    reconnecting: bool,
    attempt: usize,
}

impl Session {
    /// Picks up the provider and the local folders `settings` remembers and starts scanning those
    /// folders. Nothing is signed in until [`Store::restore_session`].
    pub fn new(
        providers: Vec<Arc<dyn MusicProvider>>,
        local_provider: Arc<dyn MusicProvider>,
        settings: &AppSettings,
        cx: &mut Cx,
    ) -> Self {
        let remembered = settings.provider().to_string();
        let local_folders = settings.local_folders().to_vec();
        let active = providers
            .iter()
            .position(|provider| provider.slug() == remembered);
        let local_playback = local_provider.playback_factory();
        let mut session = Self {
            state: SessionState::SignedOut,
            providers,
            active,
            awaiting: None,
            resume: None,
            passed: Vec::new(),
            error: None,
            client: None,
            catalog: None,
            playback: None,
            shape: Shape::Saved,
            authenticated: false,
            capabilities: Capabilities::NONE,
            task: None,
            prompt_task: None,
            input: None,
            window: None,
            window_task: None,
            local_provider,
            local_folders,
            local_client: None,
            local_catalog: None,
            local_playback,
            local_capabilities: Capabilities::NONE,
            local_task: None,
            scanning: false,
            scan_asked: false,
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

    /// What a shelf's library is made of. The local shelf is always a fixed catalog.
    pub fn shape_of(&self, shelf: Shelf) -> Shape {
        match shelf {
            Shelf::Streaming => self.shape,
            Shelf::Local => Shape::Catalog(Catalog::Fixed),
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

    /// The local provider itself, so a file can be read as a track without waiting for
    /// a scan: an explorer open may point outside every configured folder.
    pub fn local_provider(&self) -> Arc<dyn MusicProvider> {
        self.local_provider.clone()
    }

    pub fn local_paths(&self) -> Vec<String> {
        self.local_folders
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    pub fn providers(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers
            .iter()
            .enumerate()
            .map(|(index, provider)| ProviderInfo {
                slug: provider.slug(),
                name: provider.name(),
                options: provider.sign_in_options(),
                // Asked in this order because answering `supported` starts a child process once
                // on Linux, and only a provider that signs in with cookies is worth it.
                web_sign_in: provider.web_sign_in().is_some() && webview::supported(),
                protected: provider.protected(),
                stored: provider.stored(),
                guest: provider.stored_guest(),
                active: self.active == Some(index),
                pending: self.awaiting == Some(index),
                error: match &self.error {
                    Some((failed, failure)) if *failed == index => Some(failure.clone()),
                    _ => None,
                },
            })
    }

    pub fn connected(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers().filter(|info| info.stored)
    }

    /// Whether the current provider's tracks need the Widevine module and it has an account.
    /// A protected provider the user signed into but is not on right now does not count, so
    /// nothing about the module is looked for or shown until they switch to it.
    pub fn wants_drm(&self) -> bool {
        let Some(index) = self.active else {
            return false;
        };
        let provider = &self.providers[index];
        provider.protected() && provider.stored()
    }

    /// Whether a browser window matters right now: the current provider signs in through one,
    /// or no account is stored yet and the sign-in screen offers every provider.
    pub fn wants_browser(&self) -> bool {
        match self.active {
            Some(index) => self.providers[index].web_sign_in().is_some(),
            None => !self.providers.iter().any(|provider| provider.stored()),
        }
    }

    pub fn forget(&mut self, slug: &str, settings: &AppSettings, cx: &mut Cx) {
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        if self.active == Some(index) {
            return self.sign_out(settings, cx);
        }
        self.providers[index].sign_out();
        cx.changed(Topic::Session);
    }

    pub fn switch(&mut self, slug: &str, settings: &AppSettings, cx: &mut Cx) {
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
        self.restore(settings, cx);
    }

    pub fn provider_name(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.name())
    }

    pub fn provider_slug(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.slug())
    }

    /// The host to try when checking whether the network is back. It is the active provider's
    /// own, so the check never touches a service the app is not already using.
    pub fn reach(&self) -> Option<String> {
        self.providers.get(self.active?)?.reach()
    }

    /// What `link` points at, with the slug of the provider whose link it is. Every provider is
    /// asked, not only the current one.
    pub fn read_link(&self, link: &str) -> Option<(&'static str, Linked)> {
        self.providers
            .iter()
            .find_map(|provider| Some((provider.slug(), provider.link(link)?)))
    }

    /// The name of the provider `slug` and whether it has an account stored.
    pub fn account(&self, slug: &str) -> Option<(&'static str, bool)> {
        self.providers
            .iter()
            .find(|provider| provider.slug() == slug)
            .map(|provider| (provider.name(), provider.stored()))
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
    pub fn provider_for(&self, id: &str) -> Option<&dyn MusicProvider> {
        match music::is_local_id(id) {
            true => Some(self.local_provider.as_ref()),
            false => self.active.map(|index| self.providers[index].as_ref()),
        }
    }

    pub fn authenticated(&self) -> bool {
        self.authenticated
    }

    /// Whether the active session is an anonymous guest session rather than an authenticated account.
    pub fn guest(&self) -> bool {
        self.client.is_some() && !self.authenticated
    }

    /// What the live streaming provider can do beyond listing and playing. Nothing is offered
    /// while signed out, which is what the empty set means.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// The same, for whichever shelf a thing belongs to. Anything that routes by id asks this
    /// one, since the two shelves can differ: local files keep favorites but seed no station.
    pub fn capabilities_of(&self, shelf: Shelf) -> Capabilities {
        match shelf {
            Shelf::Streaming => self.capabilities,
            Shelf::Local => self.local_capabilities,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.state,
            SessionState::Restoring | SessionState::Authorizing(_)
        )
    }

    /// Opens the active provider's stored sign-in. One that cannot be opened hands over to the
    /// next provider with an account stored and then to a guest session, so the sign-in page
    /// comes up only when there is nothing at all to carry on with.
    pub fn restore(&mut self, settings: &AppSettings, cx: &mut Cx) {
        if self.is_pending() {
            return;
        }
        self.passed.clear();
        let Some(active) = self.active else {
            self.state = SessionState::SignedOut;
            cx.changed(Topic::Session);
            cx.emit(Event::Session(SessionEvent::SignedOut));
            return;
        };
        self.restore_at(active, settings, cx);
    }

    fn restore_at(&mut self, index: usize, settings: &AppSettings, cx: &mut Cx) {
        self.active = Some(index);
        // In offline mode the account is not reached at all. Going online tries it again.
        if settings.offline_mode() && self.providers[index].stored() {
            log::info!("session: offline mode is on, carrying on offline");
            return self.carry_on_offline(offline_mode(), cx);
        }
        self.state = SessionState::Restoring;
        cx.changed(Topic::Session);

        let provider = self.providers[index].clone();
        let io = cx.io.clone();
        self.task = Some(cx.spawn(|link| async move {
            let restored = join(io.spawn(async move { provider.restore().await })).await;

            link.post(move |store| {
                let Store {
                    session,
                    settings,
                    cx,
                    ..
                } = store;
                match restored {
                    Ok(Some(signed)) => session.signed_in(signed, index, settings, cx),
                    Ok(None) => session.fall_back(index, None, settings, cx),
                    // An account that is only out of reach is kept, and the app carries on
                    // offline.
                    Err(error) if Failure::new(&error).offline() && session.stored_active() => {
                        session.failed(&error, cx)
                    }
                    Err(error) => session.fall_back(index, Some(&error), settings, cx),
                }
            });
        }));
    }

    /// Moves past a provider whose stored sign-in did not open, to the next provider with an
    /// account stored, or failing that to a guest session. The provider keeps its error for the
    /// settings page, and a toast says it could not sign back in, or that the provider invalidated
    /// the session when it refused the stored sign-in outright. With nowhere to go the
    /// session ends the way it always did, on the sign-in page.
    fn fall_back(
        &mut self,
        failed: usize,
        error: Option<&Error>,
        settings: &AppSettings,
        cx: &mut Cx,
    ) {
        let provider = self.providers[failed].clone();
        self.passed.push(failed);
        let next = self
            .providers
            .iter()
            .enumerate()
            .find(|(index, provider)| provider.stored() && !self.passed.contains(index))
            .map(|(index, _)| index);
        let guest = self
            .providers
            .iter()
            .enumerate()
            .find(|(index, provider)| {
                !self.passed.contains(index)
                    && provider
                        .sign_in_options()
                        .iter()
                        .any(|option| matches!(option, SignIn::Anonymous))
            })
            .map(|(_, provider)| provider.slug());

        if next.is_none() && guest.is_none() {
            match error {
                Some(error) => self.failed(error, cx),
                None => {
                    self.state = SessionState::SignedOut;
                    cx.changed(Topic::Session);
                    cx.emit(Event::Session(SessionEvent::SignedOut));
                }
            }
            return;
        }

        match error {
            Some(error) => {
                log::warn!(
                    "session: cannot restore {}, falling back: {error:#}",
                    provider.name()
                );
                self.error = Some((failed, Failure::new(error)));
                Toasts::with_detail(
                    Outcome::Failed,
                    "toast-restore-failed",
                    Some(provider.name().into()),
                    format!("{error:#}"),
                    cx,
                );
            }
            None if provider.stored() => {
                log::warn!(
                    "session: {} refused its stored sign-in, falling back",
                    provider.name()
                );
                let toast = Toast::new(Outcome::Failed, "toast-session-invalidated")
                    .name(provider.name())
                    .open("toast-sign-in", Target::ProviderSettings)
                    .lasting();
                Toasts::post(toast, cx);
            }
            None => log::debug!(
                "session: {} has nothing stored, falling back",
                provider.name()
            ),
        }
        // The failed provider's library goes with it, as it would on the sign-in page.
        self.client = None;
        self.catalog = None;
        self.playback = None;
        cx.emit(Event::Session(SessionEvent::SignedOut));

        if let Some(index) = next {
            return self.restore_at(index, settings, cx);
        }
        if let Some(slug) = guest {
            // A sign-in only starts from a settled state, and nothing renders in between.
            self.state = SessionState::SignedOut;
            self.start_sign_in(slug, SignIn::Anonymous, SecretInput::Browser, cx);
        }
    }

    pub fn sign_in(&mut self, slug: &str, method: SignIn, cx: &mut Cx) {
        self.start_sign_in(slug, method, SecretInput::Browser, cx);
    }

    pub fn sign_in_with_cookies(&mut self, slug: &str, cx: &mut Cx) {
        self.start_sign_in(slug, SignIn::Secret, SecretInput::Manual, cx);
    }

    fn start_sign_in(
        &mut self,
        slug: &str,
        method: SignIn,
        secret_input: SecretInput,
        cx: &mut Cx,
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
        cx.changed(Topic::Session);

        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        self.input = Some(input_tx);
        let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::unbounded_channel::<SignInPrompt>();
        self.prompt_task = Some(cx.spawn(|link| async move {
            while let Some(prompt) = prompt_rx.recv().await {
                link.post(move |store| {
                    let Store { session, cx, .. } = store;
                    if matches!(session.state, SessionState::Authorizing(_)) {
                        let secret = matches!(prompt, SignInPrompt::Secret);
                        session.state = SessionState::Authorizing(Some(prompt));
                        cx.changed(Topic::Session);
                        if secret && matches!(secret_input, SecretInput::Browser) {
                            session.open_window(cx);
                        }
                    }
                });
            }
        }));
        let prompt: PromptSink = Arc::new(move |prompt| {
            prompt_tx.send(prompt).ok();
        });

        let provider = self.providers[index].clone();
        let io = cx.io.clone();
        self.task = Some(cx.spawn(|link| async move {
            let authorized =
                join(io.spawn(async move { provider.sign_in(method, prompt, input_rx).await }))
                    .await;

            link.post(move |store| {
                let Store {
                    session,
                    settings,
                    cx,
                    ..
                } = store;
                session.prompt_task = None;
                session.input = None;
                session.window = None;
                match authorized {
                    Ok(signed) => session.signed_in(signed, index, settings, cx),
                    Err(error) => session.failed(&error, cx),
                }
            });
        }));
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Cx) {
        if !matches!(self.state, SessionState::Authorizing(_)) {
            return;
        }
        if let Some(index) = self.awaiting {
            let provider = self.providers[index].clone();
            cx.io.spawn(async move { provider.abandon() });
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
            cx.changed(Topic::Session);
            return;
        }
        self.state = SessionState::SignedOut;
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::SignedOut));
    }

    /// Opens the browser window for the secret prompt now showing. The window answers the prompt
    /// itself once the user is through; closing it cancels the sign-in.
    fn open_window(&mut self, cx: &mut Cx) {
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
            agent: sign_in.agent.map(str::to_owned),
            script: None,
        };
        match webview::Page::open(target) {
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
        self.window_task = Some(cx.spawn(|link| async move {
            loop {
                tokio::time::sleep(WINDOW_POLL).await;
                let open = link
                    .read(|store| {
                        let Store { session, cx, .. } = store;
                        let Some(login) = session.window.as_mut() else {
                            return false;
                        };
                        match login.poll() {
                            webview::Poll::Pending => true,
                            webview::Poll::Closed => {
                                session.window = None;
                                session.cancel_sign_in(cx);
                                false
                            }
                            webview::Poll::Cookies(header) => {
                                session.window = None;
                                session.submit_input(header, cx);
                                false
                            }
                        }
                    })
                    .await;
                if !open.unwrap_or(false) {
                    break;
                }
            }
        }));
    }

    pub fn submit_input(&mut self, text: String, cx: &mut Cx) {
        if let Some(input) = &self.input {
            self.window = None;
            input.send(text).ok();
            if let SessionState::Authorizing(Some(
                SignInPrompt::Secret | SignInPrompt::Accounts(_),
            )) = &self.state
            {
                self.state = SessionState::Authorizing(None);
                cx.changed(Topic::Session);
            }
        }
    }

    pub fn sign_out(&mut self, settings: &AppSettings, cx: &mut Cx) {
        if let Some(active) = self.active {
            self.providers[active].sign_out();
        }
        self.release(cx);
        if let Some(index) = self.remaining() {
            self.active = Some(index);
            self.restore(settings, cx);
        }
    }

    /// Whether the active provider has an account stored, which is what tells a failed restore
    /// from a user who signed out.
    fn stored_active(&self) -> bool {
        self.active
            .is_some_and(|index| self.providers[index].stored())
    }

    /// Lets go of the live account when offline mode turns on, leaving the session where a launch
    /// in offline mode would put it. A sign-in in progress and an account with nothing stored
    /// are left alone.
    pub fn go_offline(&mut self, cx: &mut Cx) {
        let live = matches!(
            self.state,
            SessionState::SignedIn(_) | SessionState::Restoring
        );
        if !live || !self.stored_active() {
            return;
        }
        log::info!("session: offline mode is on, letting go of the account");
        self.task = None;
        self.watch = None;
        self.reconnect = None;
        self.reconnecting = false;
        self.attempt = 0;
        self.client = None;
        self.catalog = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.capabilities = Capabilities::NONE;
        self.carry_on_offline(offline_mode(), cx);
        cx.emit(Event::Session(SessionEvent::WentOffline));
    }

    /// Tries the stored account again once the network is back, for a run that started without
    /// one. Anything but a run held up by the network is left alone.
    pub fn restore_if_offline(&mut self, settings: &AppSettings, cx: &mut Cx) {
        if matches!(self.state, SessionState::Offline(_)) {
            self.restore(settings, cx);
        }
    }

    fn remaining(&self) -> Option<usize> {
        self.active
            .filter(|index| self.providers[*index].stored())
            .or_else(|| self.providers.iter().position(|provider| provider.stored()))
    }

    fn release(&mut self, cx: &mut Cx) {
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
        self.capabilities = Capabilities::NONE;
        self.state = SessionState::SignedOut;
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::SignedOut));
    }

    fn signed_in(
        &mut self,
        session: ProviderSession,
        index: usize,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
        let replaced = self
            .resume
            .take()
            .is_some_and(|(held, profile)| held != index || profile.id != session.profile.id);
        if replaced {
            self.drop_previous_session(cx);
        }
        self.active = Some(index);
        self.awaiting = None;
        // A provider passed over on the way here keeps its error until it signs in again.
        self.error = self.error.take().filter(|(failed, _)| *failed != index);
        self.passed.clear();
        let slug = self.providers[index].slug();
        settings.set_provider(slug, cx);
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.capabilities = session.capabilities;
        self.state = SessionState::SignedIn(session.profile);
        self.attempt = 0;
        self.start_heartbeat(cx);
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::SignedIn));
    }

    fn drop_previous_session(&mut self, cx: &mut Cx) {
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.capabilities = Capabilities::NONE;
        self.watch = None;
        self.reconnect = None;
        self.reconnecting = false;
        self.attempt = 0;
        cx.emit(Event::Session(SessionEvent::SignedOut));
    }

    fn start_heartbeat(&mut self, cx: &mut Cx) {
        self.watch = Some(cx.spawn(|link| async move {
            loop {
                tokio::time::sleep(HEARTBEAT).await;
                let checked = link
                    .read(|store| store.session.reconnect_if_stale(&mut store.cx))
                    .await;
                if checked.is_none() {
                    return;
                }
            }
        }));
    }

    pub fn reconnect_if_stale(&mut self, cx: &mut Cx) -> bool {
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
        let io = cx.io.clone();
        self.reconnect = Some(cx.spawn(|link| async move {
            tokio::time::sleep(wait).await;
            let restored = join(io.spawn(async move { provider.restore().await })).await;
            link.post(move |store| {
                let Store { session, cx, .. } = store;
                session.reconnecting = false;
                match restored {
                    Ok(Some(signed)) => session.reconnected(signed, cx),
                    Ok(None) => log::warn!("session: nothing stored to reconnect with"),
                    Err(error) => log::warn!("session: cannot reconnect: {error:#}"),
                }
            });
        }));
        true
    }

    fn reconnected(&mut self, session: ProviderSession, cx: &mut Cx) {
        self.attempt = 0;
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.capabilities = session.capabilities;
        log::debug!("session: reconnected");
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::Reconnected));
    }

    /// Keeps the stored account without reaching it, so the app runs on what the library kept
    /// and on the downloads, which still play through an engine built from the stored sign-in
    /// alone.
    fn carry_on_offline(&mut self, failure: Failure, cx: &mut Cx) {
        // A provider may need the runtime to build its offline engine.
        let runtime = cx.io.handle();
        let _runtime = runtime.enter();
        self.playback = self
            .active
            .and_then(|active| self.providers[active].offline_playback());
        self.state = SessionState::Offline(failure);
        cx.changed(Topic::Session);
    }

    fn failed(&mut self, error: &Error, cx: &mut Cx) {
        let failure = Failure::new(error);
        cx.emit(Event::NetworkFailed(format!("{error:#}")));
        if let Some(failed) = self.awaiting.or(self.active) {
            self.error = Some((failed, failure.clone()));
        }
        // Only a restore may end up offline. A sign-in the user is watching says what went
        // wrong on the page they started it from.
        let restoring = self.awaiting.is_none();
        self.awaiting = None;
        if let Some((index, profile)) = self.resume.take() {
            self.active = Some(index);
            self.state = SessionState::SignedIn(profile);
            cx.changed(Topic::Session);
            return;
        }
        if restoring && failure.offline() && self.stored_active() {
            log::warn!("session: the account could not be reached, carrying on offline");
            return self.carry_on_offline(failure, cx);
        }
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.state = SessionState::Failed(failure);
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::SignedOut));
    }

    fn restore_local(&mut self, cx: &mut Cx) {
        if self.local_folders.is_empty() {
            return;
        }
        self.scan_local(self.local_folders.clone(), false, cx);
    }

    /// Adds a folder to the local library, then rescans every configured folder together so
    /// artists and albums that span more than one root merge into one, seamlessly.
    pub fn add_local_folder(&mut self, path: PathBuf, settings: &mut AppSettings, cx: &mut Cx) {
        self.add_local_folders(vec![path], settings, cx);
    }

    /// Same as [`Session::add_local_folder`], for a batch picked in one native dialog.
    pub fn add_local_folders(
        &mut self,
        paths: Vec<PathBuf>,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
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
        self.set_local_folders(folders, true, settings, cx);
    }

    /// Drops a folder from the local library and offers to put it back.
    pub fn remove_local_folder(&mut self, path: &Path, settings: &mut AppSettings, cx: &mut Cx) {
        let Some(at) = self
            .local_folders
            .iter()
            .position(|existing| existing == path)
        else {
            return;
        };
        let mut folders = self.local_folders.clone();
        let removed = folders.remove(at);
        self.set_local_folders(folders, true, settings, cx);

        let toast = Toast::new(Outcome::Plain, "toast-folder-removed")
            .name(folder_name(&removed))
            .undo(move |store| store.restore_local_folder(removed.clone(), at));
        Toasts::post(toast, cx);
    }

    /// Puts a folder the user just removed back where it was in the list. A folder gone from
    /// disk is reported instead, and one that overlaps a folder added since is left alone.
    fn restore_local_folder(
        &mut self,
        path: PathBuf,
        at: usize,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
        if !path.is_dir() {
            Toasts::about(
                Outcome::Warning,
                "toast-folder-missing",
                folder_name(&path),
                cx,
            );
            return;
        }
        if self
            .local_folders
            .iter()
            .any(|existing| overlaps(existing, &path))
        {
            return;
        }
        let mut folders = self.local_folders.clone();
        folders.insert(at.min(folders.len()), path);
        self.set_local_folders(folders, true, settings, cx);
    }

    /// Rescans every configured local folder without changing the list, e.g. after files
    /// changed on disk or a tag was edited. A `thorough` rescan is the one the user asked for:
    /// it forgets what the last scan recorded, so every folder is listed and every file stat'd
    /// again, which is the only way an edit made behind Sonora's back is noticed.
    pub fn rescan_local(&mut self, thorough: bool, settings: &mut AppSettings, cx: &mut Cx) {
        if thorough {
            self.local_provider.forget_scan();
        }
        self.set_local_folders(self.local_folders.clone(), thorough, settings, cx);
    }

    /// Points the local library at `folders` and scans them. A scan already under way is
    /// cancelled first: its folders may be the ones just removed, and two scans would only
    /// fight over the same disk. An `asked` scan is one the user started, and its end or
    /// failure is reported in a toast.
    fn set_local_folders(
        &mut self,
        folders: Vec<PathBuf>,
        asked: bool,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
        if !folders.is_empty() {
            return self.scan_local(folders, asked, cx);
        }
        music::progress::cancel();
        self.scanning = false;
        self.scan_asked = false;
        self.local_provider.sign_out();
        self.local_folders = Vec::new();
        settings.set_local_folders(Vec::new(), cx);
        self.local_client = None;
        self.local_catalog = None;
        self.local_capabilities = Capabilities::NONE;
        self.local_task = None;
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::LocalChanged));
    }

    /// The scan half of [`Self::set_local_folders`], for a list that is not empty. The list is
    /// only kept, in the session and in settings, once the scan has gone through.
    fn scan_local(&mut self, folders: Vec<PathBuf>, asked: bool, cx: &mut Cx) {
        music::progress::cancel();
        self.scan_asked |= asked;
        let provider = self.local_provider.clone();
        let chosen = folders.clone();
        let io = cx.io.clone();
        let counting = self.scan_asked;
        self.scanning = true;
        cx.changed(Topic::Session);
        self.local_task = Some(cx.spawn(|link| async move {
            let prompt: PromptSink = Arc::new(|_| {});
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let signed_in = join(io.spawn(async move {
                let session = provider.sign_in(SignIn::Path(folders), prompt, rx).await?;
                let songs = match counting {
                    true => session
                        .api
                        .all_tracks()
                        .await
                        .map_or(0, |tracks| tracks.len()),
                    false => 0,
                };
                anyhow::Ok((session, songs))
            }))
            .await;

            link.post(move |store| {
                let Store {
                    session,
                    settings,
                    scan,
                    cx,
                    ..
                } = store;
                session.scanning = false;
                let asked = std::mem::take(&mut session.scan_asked);
                match signed_in {
                    Ok((signed, songs)) => {
                        session.local_folders = chosen.clone();
                        settings.set_local_folders(chosen, cx);
                        session.local_signed_in(signed, cx);
                        if asked {
                            scan.finished(songs, cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("session: cannot update local music folders: {error:#}");
                        if asked {
                            Toasts::with_detail(
                                Outcome::Failed,
                                "toast-scan-failed",
                                None,
                                format!("{error:#}"),
                                cx,
                            );
                        }
                        cx.changed(Topic::Session);
                    }
                }
            });
        }));
    }

    /// Whether a local scan is under way right now.
    pub fn scanning(&self) -> bool {
        self.scanning
    }

    fn local_signed_in(&mut self, session: ProviderSession, cx: &mut Cx) {
        self.local_catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.local_client = Some(session.api);
        if self.local_playback.is_none() {
            self.local_playback = Some(session.playback);
        }
        self.local_capabilities = session.capabilities;
        cx.changed(Topic::Session);
        cx.emit(Event::Session(SessionEvent::LocalChanged));
    }
}

impl Store {
    /// Opens the active provider's stored sign-in. One that cannot be opened hands over to the
    /// next provider with an account stored and then to a guest session, so the sign-in page
    /// comes up only when there is nothing at all to carry on with.
    pub fn restore_session(&mut self) {
        self.session.restore(&self.settings, &mut self.cx);
    }

    /// Makes the provider `slug` the current one and opens its stored sign-in.
    pub fn switch_provider(&mut self, slug: &str) {
        self.session.switch(slug, &self.settings, &mut self.cx);
    }

    /// Signs out of the current provider and carries on with the next one that has an account
    /// stored, if any.
    pub fn sign_out(&mut self) {
        self.session.sign_out(&self.settings, &mut self.cx);
    }

    /// Forgets the account stored for `slug`, signing out of it first when it is the current one.
    pub fn forget_provider(&mut self, slug: &str) {
        self.session.forget(slug, &self.settings, &mut self.cx);
    }

    /// See [`Session::add_local_folder`].
    pub fn add_local_folder(&mut self, path: PathBuf) {
        self.add_local_folders(vec![path]);
    }

    /// Same as [`Store::add_local_folder`], for a batch picked in one native dialog.
    pub fn add_local_folders(&mut self, paths: Vec<PathBuf>) {
        let Store {
            session,
            settings,
            cx,
            ..
        } = self;
        session.add_local_folders(paths, settings, cx);
    }

    /// Drops a folder from the local library and offers to put it back.
    pub fn remove_local_folder(&mut self, path: &Path) {
        let Store {
            session,
            settings,
            cx,
            ..
        } = self;
        session.remove_local_folder(path, settings, cx);
    }

    /// Puts back a folder the user just removed, for the Undo on the toast that reported it.
    fn restore_local_folder(&mut self, path: PathBuf, at: usize) {
        let Store {
            session,
            settings,
            cx,
            ..
        } = self;
        session.restore_local_folder(path, at, settings, cx);
    }

    /// See [`Session::rescan_local`].
    pub fn rescan_local(&mut self, thorough: bool) {
        let Store {
            session,
            settings,
            cx,
            ..
        } = self;
        session.rescan_local(thorough, settings, cx);
    }

    pub(crate) fn react_session(&mut self, change: &Change) {
        // A run that started without a network never signed out, so the account it kept is tried
        // again the moment there is one.
        if let Change::Event(Event::Reconnected) = change {
            self.session
                .restore_if_offline(&self.settings, &mut self.cx);
        }
    }
}

/// The failure a session kept offline on purpose reports, which reads as a lost network.
fn offline_mode() -> Failure {
    Failure {
        problem: Some(SignInProblem::Network),
        summary: "offline mode is on".to_owned(),
        detail: None,
    }
}

/// The folder's own name for a toast, or the whole path when it has none, such as `/`.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Whether `a` and `b` are the same directory, or one contains the other — either way, scanning
/// both would double-count the tracks they share.
fn overlaps(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b || a.starts_with(&b) || b.starts_with(&a)
}
