use std::collections::HashMap;
use std::mem::DropGuard;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use gpui::{Context, Entity, EventEmitter, Task};
use i18n::t;
use music::link::Linked;
use music::{
    Capabilities, Catalog, Libraries, Library, MusicApi, MusicProvider, PlaybackFactory,
    PromptSink, ProviderSession, ScanReport, Shape, SignIn, SignInFailure, SignInProblem,
    SignInPrompt, Track, UserProfile,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use crate::catalog::CatalogSource;
use crate::settings::AppSettings;
use crate::{Io, Network, Outcome, Scan, Shelf, Target, Toast, Toasts, join};

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
    /// The provider the account belongs to, the same for every account of one provider.
    pub kind: &'static str,
    pub name: &'static str,
    /// What tells the account apart from the provider's other accounts, for a provider that
    /// keeps one account per server: the server's own name or the username, never its address.
    pub label: Option<String>,
    /// The account's place among the provider's accounts that share its `label`, from 2 on, so
    /// a list can still tell them apart. The first of them has none.
    pub number: Option<usize>,
    /// Whether the provider has more than one account stored, so a list has to tell them apart
    /// by `label`.
    pub several: bool,
    /// Whether this is the empty slot that adds another account to a provider that already has
    /// one.
    pub spare: bool,
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
    settings: Entity<AppSettings>,
    client: Option<Arc<dyn MusicApi>>,
    catalog: Option<Arc<CatalogSource>>,
    playback: Option<Arc<dyn PlaybackFactory>>,
    shape: Shape,
    authenticated: bool,
    capabilities: Capabilities,
    io: Io,
    task: Option<Task<()>>,
    prompt_task: Option<Task<()>>,
    input: Option<UnboundedSender<String>>,
    /// The browser window a `SignInPrompt::Secret` opened, while it is up.
    window: Option<webview::Page>,
    window_task: Option<Task<()>>,
    /// Whether this run uses only local folders, independently of a stored streaming account.
    local_only: bool,
    local_provider: Arc<dyn MusicProvider>,
    local_folders: Vec<PathBuf>,
    /// Retains folder ordering across multiple outstanding removal Undo actions.
    local_order: Vec<PathBuf>,
    local_client: Option<Arc<dyn MusicApi>>,
    /// The latest local metadata and exact move aliases, applied to already queued songs.
    local_tracks: HashMap<String, Track>,
    local_report: ScanReport,
    local_catalog: Option<Arc<CatalogSource>>,
    local_playback: Option<Arc<dyn PlaybackFactory>>,
    local_capabilities: Capabilities,
    local_task: Option<Task<()>>,
    /// Protects a newer folder selection from a scan that was already finishing when cancelled.
    local_revision: u64,
    /// The scan warning for the current folder selection, withdrawn when that selection changes.
    local_warning: Option<usize>,
    /// Whether a local scan is under way, so the UI can show its progress.
    scanning: bool,
    /// Whether the scan under way is one the user started, so its end is reported. An automatic
    /// rescan that cancels it takes the report over.
    scan_asked: bool,
    watch: Option<Task<()>>,
    reconnect: Option<Task<()>>,
    reconnecting: bool,
    attempt: usize,
    /// The server libraries the library picker lists, or why they could not be read. `None`
    /// while a read is under way or when none was asked for.
    libraries: Option<Result<Libraries, String>>,
    libraries_task: Option<Task<()>>,
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
        let providers: Vec<Arc<dyn MusicProvider>> = providers
            .into_iter()
            .flat_map(|provider| {
                let siblings = provider.siblings();
                std::iter::once(provider).chain(siblings)
            })
            .collect();
        let local_folders = settings.read(cx).local_folders().to_vec();
        // a further account signed out since leaves its slug behind, and its provider's first
        // account takes over, handing on from there like any account that cannot be opened
        let active = providers
            .iter()
            .position(|provider| provider.slug() == remembered)
            .or_else(|| {
                let kind = music::credentials::kind(&remembered);
                (kind != remembered)
                    .then(|| {
                        providers
                            .iter()
                            .position(|provider| provider.slug() == kind)
                    })
                    .flatten()
            });
        let local_playback = local_provider.playback_factory();
        let mut session = Self {
            state: SessionState::SignedOut,
            providers,
            active,
            awaiting: None,
            resume: None,
            passed: Vec::new(),
            error: None,
            settings,
            client: None,
            catalog: None,
            playback: None,
            shape: Shape::Saved,
            authenticated: false,
            capabilities: Capabilities::NONE,
            io,
            task: None,
            prompt_task: None,
            input: None,
            window: None,
            window_task: None,
            local_only: remembered == local_provider.slug(),
            local_provider,
            local_order: local_folders.clone(),
            local_folders,
            local_client: None,
            local_tracks: HashMap::new(),
            local_report: ScanReport::default(),
            local_catalog: None,
            local_playback,
            local_capabilities: Capabilities::NONE,
            local_task: None,
            local_revision: 0,
            local_warning: None,
            scanning: false,
            scan_asked: false,
            watch: None,
            reconnect: None,
            reconnecting: false,
            attempt: 0,
            libraries: None,
            libraries_task: None,
        };
        session.keep_spares();
        session.restore_local(cx);
        session
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// The signed-in account that can use a matching Soloist pairing.
    pub(crate) fn soloist_account(&self) -> Option<&str> {
        match &self.state {
            SessionState::SignedIn(profile) if self.offers_soloist() => Some(&profile.id),
            _ => None,
        }
    }

    pub fn client(&self) -> Option<Arc<dyn MusicApi>> {
        self.client.clone()
    }

    /// Whether Offline was selected as the music source. Streaming accounts remain stored.
    pub fn local_only(&self) -> bool {
        self.local_only
    }

    /// The library shown by the main Library page and searched in this run.
    pub fn primary_shelf(&self) -> Shelf {
        match self.local_only {
            true => Shelf::Local,
            false => Shelf::Streaming,
        }
    }

    /// The source for browsing and searching, without exposing a concrete provider.
    pub fn browsing_client(&self) -> Option<Arc<dyn MusicApi>> {
        self.client_of(self.primary_shelf())
    }

    /// Starts a local-only run and remembers it without opening a streaming session.
    pub fn use_local(&mut self, cx: &mut Context<Self>) {
        self.release(cx);
        self.active = None;
        self.local_only = true;
        self.settings.update(cx, |settings, cx| {
            settings.set_provider(self.local_slug(), cx);
        });
        self.state = SessionState::SignedIn(self.local_profile());
        cx.notify();
        cx.emit(SessionEvent::SignedIn);
    }

    /// The profile an offline run is signed in as.
    fn local_profile(&self) -> UserProfile {
        UserProfile {
            id: self.local_slug().to_owned(),
            display_name: t!("login-offline-title").to_string(),
            avatar: None,
        }
    }

    /// Takes a sign-in started offline back to the offline run it left. That run is still
    /// loaded, so nothing is released and the queue keeps playing.
    fn back_to_local(&mut self, cx: &mut Context<Self>) {
        self.state = SessionState::SignedIn(self.local_profile());
        cx.notify();
    }

    pub fn playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        self.playback.clone()
    }

    pub fn local_client(&self) -> Option<Arc<dyn MusicApi>> {
        self.local_client.clone()
    }

    /// Updated metadata for an indexed file, following an exact move when its old path is gone.
    pub(crate) fn refreshed_local_track(&self, id: &str) -> Option<&Track> {
        self.local_tracks.get(id).or_else(|| {
            let moved = self
                .local_report
                .moved
                .iter()
                .find(|(from, _)| from == id)?;
            self.local_tracks.get(&moved.1)
        })
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

    /// Every account slot worth showing, the accounts of one provider together and in the order
    /// they were added. A provider that keeps several accounts shows each stored one and a single
    /// empty slot to add another, and an empty slot shows anyway while it signs in or failed to.
    pub fn providers(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        let stored: Vec<bool> = self
            .providers
            .iter()
            .map(|provider| provider.stored())
            .collect();
        let mut kinds: Vec<&'static str> = Vec::new();
        for provider in &self.providers {
            let kind = music::credentials::kind(provider.slug());
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        let mut shown: Vec<(usize, bool)> = Vec::new();
        for kind in kinds {
            let slots: Vec<usize> = (0..self.providers.len())
                .filter(|&index| music::credentials::kind(self.providers[index].slug()) == kind)
                .collect();
            let accounts = slots.iter().filter(|&&index| stored[index]).count();
            let busy = |index: usize| {
                self.active == Some(index)
                    || self.awaiting == Some(index)
                    || self
                        .error
                        .as_ref()
                        .is_some_and(|(failed, _)| *failed == index)
            };
            let mut empty = false;
            for &index in &slots {
                let keep = stored[index] || busy(index);
                if keep {
                    empty |= !stored[index];
                    shown.push((index, accounts > 1));
                }
            }
            if !empty && let Some(&index) = slots.iter().find(|&&index| !stored[index]) {
                shown.push((index, accounts > 1));
            }
        }
        shown.sort_by_key(|&(index, _)| {
            let kind = music::credentials::kind(self.providers[index].slug());
            let first = self
                .providers
                .iter()
                .position(|provider| music::credentials::kind(provider.slug()) == kind);
            (first, !stored[index], index)
        });
        let labels: Vec<Option<String>> = shown
            .iter()
            .map(|&(index, _)| {
                let provider = &self.providers[index];
                provider
                    .keeps_several()
                    .then(|| provider.account_name())
                    .flatten()
            })
            .collect();
        let numbers: Vec<Option<usize>> = (0..shown.len())
            .map(|place| {
                let label = labels[place].as_ref()?;
                let kind = music::credentials::kind(self.providers[shown[place].0].slug());
                let before = (0..place)
                    .filter(|&earlier| {
                        labels[earlier].as_ref() == Some(label)
                            && music::credentials::kind(self.providers[shown[earlier].0].slug())
                                == kind
                    })
                    .count();
                (before > 0).then_some(before + 1)
            })
            .collect();
        shown.into_iter().zip(labels.into_iter().zip(numbers)).map(
            move |((index, several), (label, number))| {
                let provider = &self.providers[index];
                let kind = music::credentials::kind(provider.slug());
                let spare = !stored[index]
                    && self.providers.iter().enumerate().any(|(other, sibling)| {
                        stored[other] && music::credentials::kind(sibling.slug()) == kind
                    });
                ProviderInfo {
                    slug: provider.slug(),
                    kind,
                    name: provider.name(),
                    label,
                    number,
                    several,
                    spare,
                    options: provider.sign_in_options(),
                    // Asked in this order because answering `supported` starts a child process once
                    // on Linux, and only a provider that signs in with cookies is worth it.
                    web_sign_in: provider.web_sign_in().is_some() && webview::supported(),
                    protected: provider.protected(),
                    stored: stored[index],
                    guest: provider.stored_guest(),
                    active: self.active == Some(index),
                    pending: self.awaiting == Some(index),
                    error: match &self.error {
                        Some((failed, failure)) if *failed == index => Some(failure.clone()),
                        _ => None,
                    },
                }
            },
        )
    }

    /// Adds an empty account slot to every provider that keeps several accounts and has none
    /// left, so there is always one to sign another server in to. A slot only joins the end of
    /// the list, which keeps every index the session holds pointing at the same account.
    fn keep_spares(&mut self) {
        let mut spares = Vec::new();
        for provider in &self.providers {
            let kind = music::credentials::kind(provider.slug());
            if !provider.keeps_several() || provider.slug() != kind {
                continue;
            }
            let free = self
                .providers
                .iter()
                .any(|slot| music::credentials::kind(slot.slug()) == kind && !slot.stored());
            if !free {
                spares.extend(provider.spare());
            }
        }
        self.providers.extend(spares);
    }

    /// The providers with an account stored. A stored guest session is not an account, so it is
    /// left out even while it is the one in use.
    pub fn connected(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers().filter(|info| info.stored && !info.guest)
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

    /// Whether the current provider can play through Soloist on this computer and has an
    /// account, which is when the app offers its setup.
    pub fn offers_soloist(&self) -> bool {
        let Some(index) = self.active else {
            return false;
        };
        let provider = &self.providers[index];
        provider.soloist() && provider.stored()
    }

    /// Whether a browser window matters right now: the current provider signs in through one,
    /// or no account is stored yet and the sign-in screen offers every provider.
    pub fn wants_browser(&self) -> bool {
        if self.local_only {
            return false;
        }
        match self.active {
            Some(index) => self.providers[index].web_sign_in().is_some(),
            None => !self.providers.iter().any(|provider| provider.stored()),
        }
    }

    pub fn forget(&mut self, slug: &str, cx: &mut Context<Self>) {
        if self.local_only && slug == self.local_slug() {
            return self.sign_out(cx);
        }
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
        if self.awaiting == Some(index) {
            self.cancel_sign_in(cx);
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
        self.local_only = false;
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

    /// The scope that owns the saved play queue, including a local-only run.
    pub(crate) fn playback_slug(&self) -> Option<&'static str> {
        match self.local_only {
            true => Some(self.local_slug()),
            false => self.provider_slug(),
        }
    }

    /// Builds the active provider's reachability check for execution on tokio.
    pub fn network_probe(&self) -> Option<futures::future::BoxFuture<'static, bool>> {
        self.providers.get(self.active?)?.network_probe()
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

    /// The providers whose shelves are in use, an account kept offline included.
    pub fn active_slugs(&self) -> Vec<&'static str> {
        let mut slugs = Vec::new();
        if self.shelf_in_use(Shelf::Streaming) {
            slugs.extend(self.provider_slug());
        }
        if self.shelf_in_use(Shelf::Local) {
            slugs.push(self.local_slug());
        }
        slugs
    }

    /// Whether `shelf` belongs to an account in use, reached live or kept offline. An offline
    /// account has no client, yet its pins and what its library kept are still the listener's.
    pub fn shelf_in_use(&self, shelf: Shelf) -> bool {
        match shelf {
            Shelf::Streaming => {
                self.client.is_some() || matches!(self.state, SessionState::Offline(_))
            }
            Shelf::Local => self.local_client.is_some(),
        }
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
    /// What the signed-in provider can do. The stream levels are the playback factory's when it
    /// names any, since a provider with a second engine serves other levels through it.
    pub fn capabilities(&self) -> Capabilities {
        let qualities = self
            .playback
            .as_ref()
            .and_then(|playback| playback.qualities())
            .unwrap_or(self.capabilities.qualities);
        Capabilities {
            qualities,
            ..self.capabilities
        }
    }

    /// The same, for whichever shelf a thing belongs to. Anything that routes by id asks this
    /// one, since the two shelves can differ: local files keep favorites but seed no station.
    pub fn capabilities_of(&self, shelf: Shelf) -> Capabilities {
        match shelf {
            Shelf::Streaming => self.capabilities(),
            Shelf::Local => self.local_capabilities,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.state,
            SessionState::Restoring | SessionState::Authorizing(_)
        )
    }

    /// Tears the live session down and opens the stored sign-in again, after a change to what
    /// the stored account reads — the Subsonic library it is scoped to. The library, search,
    /// genres and home all reload on the sign-in that follows, and nothing the old client
    /// answered can arrive afterwards. A restore still under way is dropped and started over, so
    /// it cannot install a client built for the previous choice.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, SessionState::Authorizing(_)) || self.active.is_none() {
            return;
        }
        self.release(cx);
        self.restore(cx);
    }

    /// The provider whose stored account can be limited to some libraries of its server: the one
    /// in use when it can, otherwise the first such account. A guest session has no account to
    /// limit.
    fn library_provider(&self) -> Option<&Arc<dyn MusicProvider>> {
        let scoped = |provider: &&Arc<dyn MusicProvider>| {
            provider.has_libraries() && provider.stored() && !provider.stored_guest()
        };
        self.active
            .map(|index| &self.providers[index])
            .filter(scoped)
            .or_else(|| self.providers.iter().find(scoped))
    }

    /// The slug of the account the library choice applies to, `None` when no stored account
    /// can be limited.
    pub fn library_slug(&self) -> Option<&'static str> {
        self.library_provider().map(|provider| provider.slug())
    }

    /// Whether a stored account can be limited to some libraries of its server.
    pub fn offers_libraries(&self) -> bool {
        self.library_provider().is_some()
    }

    /// The libraries the stored account reads, empty for the whole server. It reads what is
    /// stored and asks the server nothing.
    pub fn chosen_libraries(&self) -> Vec<Library> {
        self.library_provider()
            .map_or_else(Vec::new, |provider| provider.chosen_libraries())
    }

    /// The libraries the last read found, or why it failed. `None` while the read is under way.
    pub fn libraries(&self) -> Option<&Result<Libraries, String>> {
        self.libraries.as_ref()
    }

    /// Asks the stored account's server for its libraries, dropping any answer already held.
    pub fn load_libraries(&mut self, cx: &mut Context<Self>) {
        self.libraries = None;
        self.libraries_task = None;
        let Some(provider) = self.library_provider().cloned() else {
            cx.notify();
            return;
        };
        let io = self.io.clone();
        self.libraries_task = Some(cx.spawn(async move |this, cx| {
            let read = join(io.spawn(async move { provider.libraries().await })).await;
            this.update(cx, |this, cx| {
                this.libraries = Some(read.map_err(|error| format!("{error:#}")));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Drops the libraries read for the picker and stops a read under way.
    pub fn forget_libraries(&mut self, cx: &mut Context<Self>) {
        self.libraries = None;
        self.libraries_task = None;
        cx.notify();
    }

    /// Keeps the stored account's library choice, the whole server when `libraries` is empty,
    /// and rebuilds the session around it when that account is the one in use. A stored account
    /// that is not in use picks the choice up when it next signs in. Fails when the choice
    /// cannot be kept.
    pub fn choose_libraries(
        &mut self,
        libraries: Vec<Library>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let Some(provider) = self.library_provider().cloned() else {
            anyhow::bail!("no stored account can be limited to a library");
        };
        provider.choose_libraries(libraries)?;
        if self.provider_slug() == Some(provider.slug()) {
            self.reload(cx);
        }
        Ok(())
    }

    /// Opens the active provider's stored sign-in. One that cannot be opened hands over to the
    /// next provider with an account stored and then to a guest session, so the sign-in page
    /// comes up only when there is nothing at all to carry on with.
    pub fn restore(&mut self, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        if self.local_only {
            return self.use_local(cx);
        }
        self.passed.clear();
        let Some(active) = self.active else {
            self.state = SessionState::SignedOut;
            cx.notify();
            cx.emit(SessionEvent::SignedOut);
            return;
        };
        self.restore_at(active, cx);
    }

    fn restore_at(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = Some(index);
        // In offline mode the account is not reached at all. Going online tries it again.
        if self.settings.read(cx).offline_mode() && self.providers[index].stored() {
            log::info!("session: offline mode is on, carrying on offline");
            return self.carry_on_offline(offline_mode(), cx);
        }
        self.state = SessionState::Restoring;
        cx.notify();

        let provider = self.providers[index].clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let restored = join(io.spawn(async move { provider.restore().await })).await;

            this.update(cx, |this, cx| match restored {
                Ok(Some(session)) => this.signed_in(session, index, cx),
                Ok(None) => this.fall_back(index, None, cx),
                // An account that is only out of reach is kept, and the app carries on offline.
                Err(error) if Failure::new(&error).offline() && this.stored_active() => {
                    this.failed(&error, cx)
                }
                Err(error) => this.fall_back(index, Some(&error), cx),
            })
            .ok();
        }));
    }

    /// Moves past a provider whose stored sign-in did not open, to the next provider with an
    /// account stored, or failing that to a guest session. The provider keeps its error for the
    /// settings page, and a toast says it could not sign back in, or that the provider invalidated
    /// the session when it refused the stored sign-in outright. With nowhere to go the
    /// session ends the way it always did, on the sign-in page.
    fn fall_back(&mut self, failed: usize, error: Option<&Error>, cx: &mut Context<Self>) {
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
                    cx.notify();
                    cx.emit(SessionEvent::SignedOut);
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
        cx.emit(SessionEvent::SignedOut);

        if let Some(index) = next {
            return self.restore_at(index, cx);
        }
        if let Some(slug) = guest {
            // A sign-in only starts from a settled state, and nothing renders in between.
            self.state = SessionState::SignedOut;
            self.start_sign_in(slug, SignIn::Anonymous, SecretInput::Browser, cx);
        }
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
                        let page = match &prompt {
                            SignInPrompt::Window(url) => Some(url.clone()),
                            _ => None,
                        };
                        // A page the sign-in finished on has done its part once anything
                        // else is asked.
                        if page.is_none() {
                            this.window = None;
                            this.window_task = None;
                        }
                        this.state = SessionState::Authorizing(Some(prompt));
                        cx.notify();
                        if secret && matches!(secret_input, SecretInput::Browser) {
                            this.open_window(cx);
                        }
                        if let Some(url) = page {
                            this.open_page(url, cx);
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
        let request = self
            .io
            .spawn(async move { provider.sign_in(method, prompt, input_rx).await });
        let abort = DropGuard::new(request.abort_handle(), |handle: AbortHandle| handle.abort());
        self.task = Some(cx.spawn(async move |this, cx| {
            let _abort = abort;
            let authorized = join(request).await;

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
            self.providers[index].abandon();
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
        if self.local_only {
            return self.back_to_local(cx);
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
            title: t!("login-choose-title", provider = provider.name()).to_string(),
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

    /// Opens a page the provider's sign-in finishes on. The window has no cookies to wait for:
    /// the provider sees the sign-in end by itself and that closes it, and closing it first
    /// cancels the sign-in. Where there is no browser window to open, the page opens in the
    /// listener's browser and its address is offered to copy.
    fn open_page(&mut self, url: String, cx: &mut Context<Self>) {
        if self.window.is_some() {
            return;
        }
        let Some(index) = self.awaiting else {
            return;
        };
        let target = webview::Target {
            url: url.clone(),
            landing: url.clone(),
            domain: url
                .split_once("://")
                .map_or(url.as_str(), |(_, rest)| rest)
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default()
                .to_owned(),
            proof: Vec::new(),
            title: t!(
                "login-window-title",
                provider = self.providers[index].name()
            )
            .to_string(),
            agent: None,
            script: None,
        };
        let opened = match webview::supported() {
            true => webview::Page::open(target),
            false => Err(anyhow::anyhow!("this system has no browser window")),
        };
        match opened {
            Ok(page) => self.window = Some(page),
            Err(error) => {
                log::warn!("session: cannot open the sign-in window: {error:#}");
                cx.open_url(&url);
                self.state = SessionState::Authorizing(Some(SignInPrompt::Url(url)));
                cx.notify();
                return;
            }
        }
        self.window_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WINDOW_POLL).await;
                let open = this.update(cx, |this, cx| {
                    let Some(page) = this.window.as_mut() else {
                        return false;
                    };
                    match page.poll() {
                        webview::Poll::Pending => true,
                        webview::Poll::Closed => {
                            this.window = None;
                            this.cancel_sign_in(cx);
                            false
                        }
                        webview::Poll::Cookies(_) => {
                            this.window = None;
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
                SignInPrompt::Secret | SignInPrompt::Accounts(..),
            )) = &self.state
            {
                self.state = SessionState::Authorizing(None);
                cx.notify();
            }
        }
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.local_only {
            self.settings
                .update(cx, |settings, cx| settings.set_provider("", cx));
            self.release(cx);
            return;
        }
        let target = self.active.or(self.awaiting);
        let pending = self.awaiting;
        if let Some(awaiting) = pending.filter(|index| Some(*index) != target) {
            self.providers[awaiting].abandon();
        }
        if let Some(index) = target {
            self.providers[index].sign_out();
        }
        self.release(cx);
        let remaining = self.providers.iter().enumerate().find(|(index, provider)| {
            Some(*index) != target && Some(*index) != pending && provider.stored()
        });
        if let Some((index, _)) = remaining {
            self.active = Some(index);
            self.restore(cx);
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
    pub fn go_offline(&mut self, cx: &mut Context<Self>) {
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
        cx.emit(SessionEvent::WentOffline);
    }

    /// Tries the stored account again once the network is back, for a run that started without
    /// one. Anything but a run held up by the network is left alone.
    pub fn restore_if_offline(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, SessionState::Offline(_)) {
            self.restore(cx);
        }
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
        self.capabilities = Capabilities::NONE;
        self.local_only = false;
        self.state = SessionState::SignedOut;
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    fn signed_in(&mut self, session: ProviderSession, index: usize, cx: &mut Context<Self>) {
        let local = self.local_only;
        self.local_only = false;
        let replaced = self
            .resume
            .take()
            .is_some_and(|(held, profile)| held != index || profile.id != session.profile.id);
        if local || replaced {
            self.drop_previous_session(cx);
        }
        self.active = Some(index);
        self.awaiting = None;
        // A provider passed over on the way here keeps its error until it signs in again.
        self.error = self.error.take().filter(|(failed, _)| *failed != index);
        self.passed.clear();
        let slug = self.providers[index].slug();
        self.keep_spares();
        self.settings.update(cx, |settings, cx| {
            settings.set_provider(slug, cx);
        });
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.capabilities = session.capabilities;
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
        self.capabilities = Capabilities::NONE;
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
                // A sign-in started meanwhile owns the session now, so a late answer about the
                // old account must not end it.
                let owned =
                    this.active == Some(active) && matches!(this.state, SessionState::SignedIn(_));
                match restored {
                    Ok(Some(session)) => this.reconnected(session, cx),
                    Ok(None) if owned && !this.stored_active() => {
                        log::warn!("session: nothing stored to reconnect with, ending the session");
                        this.release(cx);
                    }
                    Ok(None) => log::warn!("session: cannot reconnect yet, will try again"),
                    Err(error) => {
                        let failure = Failure::new(&error);
                        match owned && failure.problem == Some(SignInProblem::Credentials) {
                            true => {
                                log::warn!("session: the account was refused, ending the session");
                                this.release(cx);
                                this.state = SessionState::Failed(failure);
                                cx.notify();
                            }
                            false => log::warn!("session: cannot reconnect: {error:#}"),
                        }
                    }
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
        self.capabilities = session.capabilities;
        log::debug!("session: reconnected");
        cx.notify();
        cx.emit(SessionEvent::Reconnected);
    }

    /// Keeps the stored account without reaching it, so the app runs on what the library kept
    /// and on the downloads, which still play through an engine built from the stored sign-in
    /// alone.
    fn carry_on_offline(&mut self, failure: Failure, cx: &mut Context<Self>) {
        // A provider may need the runtime to build its offline engine.
        let _runtime = self.io.handle().enter();
        self.playback = self
            .active
            .and_then(|active| self.providers[active].offline_playback());
        self.state = SessionState::Offline(failure);
        cx.notify();
    }

    fn failed(&mut self, error: &Error, cx: &mut Context<Self>) {
        let failure = Failure::new(error);
        Network::failed(&format!("{error:#}"), cx);
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
            cx.notify();
            return;
        }
        if self.local_only {
            return self.back_to_local(cx);
        }
        if restoring && failure.offline() && self.stored_active() {
            log::warn!("session: the account could not be reached, carrying on offline");
            return self.carry_on_offline(failure, cx);
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
        self.rescan_local(false, cx);
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
            self.local_order.retain(|existing| existing != &path);
            self.local_order.push(path.clone());
            folders.push(path);
        }
        if folders.len() == self.local_folders.len() {
            return;
        }
        self.set_local_folders(folders, true, cx);
    }

    /// Drops a folder from the local library and offers to put it back.
    pub fn remove_local_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(at) = self
            .local_folders
            .iter()
            .position(|existing| existing == path)
        else {
            return;
        };
        let mut folders = self.local_folders.clone();
        let removed = folders.remove(at);
        self.set_local_folders(folders, true, cx);

        let session = cx.entity().downgrade();
        let toast = Toast::new(Outcome::Plain, "toast-folder-removed")
            .name(folder_name(&removed))
            .undo(move |cx| {
                let removed = removed.clone();
                session
                    .update(cx, |session, cx| {
                        session.restore_local_folder(removed, at, cx)
                    })
                    .ok();
            });
        Toasts::post(toast, cx);
    }

    /// Puts a folder the user just removed back where it was in the list. A folder gone from
    /// disk is reported instead, and one that overlaps a folder added since is left alone.
    fn restore_local_folder(&mut self, path: PathBuf, at: usize, cx: &mut Context<Self>) {
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
        let position = self.local_order.iter().position(|known| known == &path);
        let at = position
            .and_then(|position| {
                folders.iter().position(|folder| {
                    self.local_order
                        .iter()
                        .position(|known| known == folder)
                        .is_some_and(|rank| rank > position)
                })
            })
            .unwrap_or_else(|| match position {
                Some(_) => folders.len(),
                None => at.min(folders.len()),
            });
        folders.insert(at, path);
        self.set_local_folders(folders, true, cx);
    }

    /// Rescans every configured local folder without changing the list, e.g. after files
    /// changed on disk or a tag was edited. A `thorough` rescan is the one the user asked for:
    /// it forgets what the last scan recorded, so every folder is listed and every file stat'd
    /// again, which is the only way an edit made behind Sonora's back is noticed.
    pub fn rescan_local(&mut self, thorough: bool, cx: &mut Context<Self>) {
        self.set_local_folders(self.local_folders.clone(), thorough, cx);
    }

    /// Points the local library at `folders` and scans them. A scan already under way is
    /// cancelled first: its folders may be the ones just removed, and two scans would only
    /// fight over the same disk. An `asked` scan is one the user started, and its end or
    /// failure is reported in a toast.
    fn set_local_folders(&mut self, folders: Vec<PathBuf>, asked: bool, cx: &mut Context<Self>) {
        let ticket = music::progress::reserve();
        if let Some(warning) = self.local_warning.take() {
            Toasts::close(warning, cx);
        }
        self.local_revision = self.local_revision.wrapping_add(1);
        let revision = self.local_revision;
        self.local_task = None;
        self.local_folders = folders.clone();
        self.settings.update(cx, |settings, cx| {
            settings.set_local_folders(folders.clone(), cx)
        });
        self.scan_asked |= asked;
        if folders.is_empty() {
            self.scanning = false;
            self.scan_asked = false;
            self.local_provider.sign_out();
            self.local_folders = Vec::new();
            self.settings.update(cx, |settings, cx| {
                settings.set_local_folders(Vec::new(), cx)
            });
            self.local_client = None;
            self.local_tracks.clear();
            self.local_report = ScanReport::default();
            self.local_catalog = None;
            self.local_capabilities = Capabilities::NONE;
            self.local_task = None;
            cx.notify();
            cx.emit(SessionEvent::LocalChanged);
            return;
        }

        let provider = self.local_provider.clone();
        let io = self.io.clone();
        let thorough = self.scan_asked;
        self.scanning = true;
        cx.notify();
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let signed_in = join(io.spawn(async move {
                let session = provider.scan(folders, ticket, thorough).await?;
                let tracks = session.api.all_tracks().await?;
                let report = session.api.scan_report();
                anyhow::Ok((session, tracks, report))
            }))
            .await;

            this.update(cx, |this, cx| {
                if this.local_revision != revision {
                    return;
                }
                this.scanning = false;
                let asked = std::mem::take(&mut this.scan_asked);
                match signed_in {
                    Ok((session, tracks, report)) => {
                        let songs = tracks.len();
                        this.local_tracks = tracks
                            .into_iter()
                            .filter_map(|track| Some((track.id.clone()?, track)))
                            .collect();
                        this.local_report = report;
                        if !this.local_report.issues.is_empty() {
                            this.local_warning = Some(Toasts::post(
                                Toast::new(Outcome::Warning, "toast-scan-issues")
                                    .count(this.local_report.issues.len())
                                    .detail(this.local_report.issues.join("\n"))
                                    .open("settings-music-folders", Target::MusicFolders),
                                cx,
                            ));
                        }
                        this.local_signed_in(session, cx);
                        if asked && this.local_report.issues.is_empty() {
                            Scan::global(cx).update(cx, |scan, cx| scan.finished(songs, cx));
                        }
                    }
                    Err(error) => {
                        log::warn!("session: cannot update local music folders: {error:#}");
                        this.local_warning = Some(Toasts::post(
                            Toast::new(Outcome::Failed, "toast-scan-failed")
                                .detail(format!("{error:#}")),
                            cx,
                        ));
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    /// Whether a local scan is under way right now.
    pub fn scanning(&self) -> bool {
        self.scanning
    }

    fn local_signed_in(&mut self, session: ProviderSession, cx: &mut Context<Self>) {
        self.local_catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.local_client = Some(session.api);
        if self.local_playback.is_none() {
            self.local_playback = Some(session.playback);
        }
        self.local_capabilities = session.capabilities;
        cx.notify();
        cx.emit(SessionEvent::LocalChanged);
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
