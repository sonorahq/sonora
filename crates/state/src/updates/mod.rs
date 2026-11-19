mod build;
mod install;
mod staged;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{Context, Entity, SharedString, Subscription, Task};
use i18n::t;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{AppSettings, Io, join};
use install::Install;
use staged::Staged;

pub use staged::start_staged;

const RELEASES: &str = "https://api.github.com/repos/sonorahq/sonora/releases";
const SUMS: &str = "SHA256SUMS";
/// The tag of the one nightly prerelease. The older `nightly` release is immutable and frozen, so
/// its tag can never be reused.
const NIGHTLY_TAG: &str = "nightly-build";
/// What the nightly workflow puts before the version label in the nightly release's title.
const NIGHTLY_TITLE: &str = "Nightly ";
/// How many leading hex digits of a commit hash the version row shows, as `git` abbreviates it.
const SHORT_COMMIT: usize = 7;
/// How often a running Sonora asks GitHub again, so a copy left open in the tray for days still
/// hears about a release.
const POLL: Duration = Duration::from_secs(6 * 60 * 60);
/// Held while an update is prepared or cleared away.
static STAGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sonorahq/sonora)"
);

/// The builds Sonora updates to. Stable follows tagged releases, and Nightly follows the build
/// of main the nightly workflow publishes every night.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::Nightly];

    /// The channel the running build came from.
    pub fn running() -> Self {
        build::Build::current().channel
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }

    /// Reads a stored id. An empty or unknown one means the channel of the running build.
    pub fn from_id(id: &str) -> Self {
        match id {
            "stable" => Self::Stable,
            "nightly" => Self::Nightly,
            _ => Self::running(),
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Stable => t!("update-channel-stable"),
            Self::Nightly => t!("update-channel-nightly"),
        }
    }

    fn url(self) -> String {
        match self {
            Self::Stable => format!("{RELEASES}/latest"),
            Self::Nightly => format!("{RELEASES}/tags/{NIGHTLY_TAG}"),
        }
    }
}

/// The package's displayed version, falling back to the compiled version for portable copies.
pub fn version() -> &'static str {
    &build::Build::current().version
}

/// The shortened hash of the commit a nightly was built from, or `None` for a stable release
/// and for a build that does not know its commit.
pub fn commit() -> Option<&'static str> {
    let build = build::Build::current();
    if build.channel != Channel::Nightly {
        return None;
    }
    let commit = build.commit.as_deref()?;
    Some(commit.get(..SHORT_COMMIT).unwrap_or(commit))
}

/// Whether this copy of Sonora installs its own updates, which is when a channel can be chosen.
/// A copy a package manager installed, or a portable one, does not.
pub fn updates_itself() -> bool {
    Install::current().updates_itself()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page: String,
    /// The asset this install updates from, when the release carries one.
    package: Option<Asset>,
    sums: Option<String>,
}

impl Release {
    fn installable(&self) -> bool {
        updates_itself() && self.package.is_some() && self.sums.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Quiet,
    Offered(Release),
    Fetching,
    /// The release is downloaded and prepared, and goes in the next time Sonora starts.
    Ready(Release),
    Failed,
}

pub struct Updates {
    state: UpdateState,
    settings: Entity<AppSettings>,
    http: reqwest::Client,
    io: Io,
    task: Option<Task<()>>,
    /// The channel chosen in settings when Sonora last looked, so a different choice looks again.
    channel: Channel,
    /// Whether updates install themselves, as settings had it when Sonora last looked.
    automatic: bool,
    /// The version whose card the listener closed, which later looks leave quiet.
    dismissed: Option<String>,
    _settings: Subscription,
    _poll: Task<()>,
}

impl Updates {
    pub fn new(settings: Entity<AppSettings>, io: Io, cx: &mut Context<Self>) -> Self {
        let (channel, automatic) = wishes(&settings, cx);
        let watch = cx.observe(&settings, |this: &mut Self, settings, cx| {
            let (channel, automatic) = wishes(&settings, cx);
            if channel == this.channel && automatic == this.automatic {
                return;
            }
            let moved = channel != this.channel;
            this.channel = channel;
            this.automatic = automatic;
            this.task = None;
            this.state = UpdateState::Quiet;
            cx.notify();
            if moved || !automatic {
                forget(&this.io);
            }
            this.look(moved, cx);
        });
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |this, cx| this.look(false, cx)).is_err() {
                    break;
                }
            }
        });
        let mut updates = Self {
            state: UpdateState::Quiet,
            settings,
            http: reqwest::Client::new(),
            io,
            task: None,
            channel,
            automatic,
            dismissed: None,
            _settings: watch,
            _poll: poll,
        };
        updates.look(false, cx);
        updates
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn offered(&self) -> Option<&Release> {
        match &self.state {
            UpdateState::Offered(release) => Some(release),
            _ => None,
        }
    }

    pub fn installable(&self) -> bool {
        self.offered().is_some_and(Release::installable)
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if let UpdateState::Offered(release) | UpdateState::Ready(release) = &self.state {
            self.dismissed = Some(release.version.clone());
        }
        self.task = None;
        self.state = UpdateState::Quiet;
        cx.notify();
    }

    /// Downloads the offered build, checks it against the release's checksums and puts it in
    /// place, then quits so the new build can start. A failure leaves the running build as it
    /// was and shows the failure instead of the offer.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.offered().cloned() else {
            return;
        };
        let (Some(package), Some(sums)) = (release.package, release.sums) else {
            return;
        };
        self.state = UpdateState::Fetching;
        cx.notify();

        let http = self.http.clone();
        let io = self.io.clone();
        let version = release.version;
        self.task = Some(cx.spawn(async move |this, cx| {
            let fetched =
                join(io.spawn(async move { download(&http, &package, &sums, &version).await }))
                    .await;
            let applied = match fetched {
                Ok(path) => join(io.spawn_blocking(move || Install::current().apply(&path))).await,
                Err(error) => Err(error),
            };
            this.update(cx, |this, cx| this.finish(applied, cx)).ok();
        }));
    }

    /// Puts the update that is ready in place now instead of on the next start, then quits so
    /// the new build can start.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready(release) = &self.state else {
            return;
        };
        if self.task.is_some() {
            return;
        }
        let version = release.version.clone();
        let channel = self.channel;
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let begun = join(io.spawn_blocking(move || {
                let staged = Staged::read()
                    .filter(|staged| staged.holds(&version, channel))
                    .context("the update is no longer staged")?;
                staged.begin()
            }))
            .await;
            this.update(cx, |this, cx| this.finish(begun, cx)).ok();
        }));
    }

    /// Quits once an update is in place, or shows the failure and keeps the running build.
    fn finish(&mut self, applied: Result<()>, cx: &mut Context<Self>) {
        self.task = None;
        match applied {
            Ok(()) => cx.quit(),
            Err(error) => {
                log::warn!("updates: cannot install the new version: {error:#}");
                self.state = UpdateState::Failed;
                cx.notify();
            }
        }
    }

    /// Asks GitHub for the newest build on the chosen channel. `asked` marks a look the user
    /// started by switching channels, which runs even with update checks turned off. A copy a
    /// package manager installed follows its own build's channel, and a nightly one never looks.
    /// When updates install themselves, a release found is downloaded and prepared in the
    /// background and only shown once it is ready. A look while another runs, or while an
    /// update waits for the next start, does nothing.
    fn look(&mut self, asked: bool, cx: &mut Context<Self>) {
        let channel = match updates_itself() {
            true => self.channel,
            false if Channel::running() == Channel::Stable => Channel::Stable,
            false => return,
        };
        if !asked && !self.settings.read(cx).check_updates() {
            return;
        }
        if self.task.is_some()
            || matches!(self.state, UpdateState::Fetching | UpdateState::Ready(_))
        {
            return;
        }
        let automatic = self.automatic;
        let http = self.http.clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let lookup = http.clone();
            let found = join(io.spawn(async move { latest(&lookup, channel).await })).await;
            let release = match found {
                Ok(Some(release)) => release,
                Ok(None) => {
                    this.update(cx, |this, _| this.task = None).ok();
                    return;
                }
                Err(error) => {
                    log::warn!("updates: cannot ask github: {error:#}");
                    this.update(cx, |this, _| this.task = None).ok();
                    return;
                }
            };
            if !automatic || !release.installable() {
                this.update(cx, |this, cx| this.offer(release, cx)).ok();
                return;
            }
            let staging = release.clone();
            let staged = join(io.spawn(async move { stage(&http, &staging, channel).await })).await;
            this.update(cx, |this, cx| match staged {
                Ok(staged) if this.channel == channel && this.automatic => {
                    // The manifest is written here rather than on tokio, so a look cancelled by
                    // a change in settings never leaves an update behind to install.
                    if let Err(error) = staged.write() {
                        log::warn!("updates: cannot stage {}: {error:#}", release.version);
                        this.offer(release, cx);
                        return;
                    }
                    log::info!("updates: {} is ready for the next start", staged.version);
                    this.task = None;
                    if this.dismissed.as_deref() != Some(release.version.as_str()) {
                        this.state = UpdateState::Ready(release);
                        cx.notify();
                    }
                }
                // The change in settings that brought this about already cleared the update.
                Ok(_) => this.task = None,
                Err(error) => {
                    log::warn!("updates: cannot stage {}: {error:#}", release.version);
                    this.offer(release, cx);
                }
            })
            .ok();
        }));
    }

    /// Shows the card for `release` unless the listener already closed it once.
    fn offer(&mut self, release: Release, cx: &mut Context<Self>) {
        self.task = None;
        if self.dismissed.as_deref() == Some(release.version.as_str()) {
            return;
        }
        self.state = UpdateState::Offered(release);
        cx.notify();
    }
}

/// The channel settings ask for and whether updates install themselves, which needs update
/// checks on, automatic installs on and a copy that updates itself.
fn wishes(settings: &Entity<AppSettings>, cx: &gpui::App) -> (Channel, bool) {
    let settings = settings.read(cx);
    let automatic = updates_itself() && settings.check_updates() && settings.auto_update();
    (settings.update_channel(), automatic)
}

#[derive(Deserialize)]
struct Published {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The newest build on `channel`, or `None` when the running build should stay.
async fn latest(http: &reqwest::Client, channel: Channel) -> Result<Option<Release>> {
    let published: Published = http
        .get(channel.url())
        .header("User-Agent", AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("cannot reach github")?
        .error_for_status()
        .context("github refused the release request")?
        .json()
        .await
        .context("cannot read the github release")?;

    let version = match channel {
        Channel::Stable => published.tag_name.trim_start_matches('v').to_owned(),
        Channel::Nightly => published
            .name
            .as_deref()
            .and_then(|name| name.strip_prefix(NIGHTLY_TITLE))
            .context("the nightly release names no version")?
            .to_owned(),
    };
    if !wanted(channel, &version) {
        return Ok(None);
    }
    let install = Install::current();
    let package = published
        .assets
        .iter()
        .find(|asset| install.wants(&asset.name))
        .cloned();
    let sums = published
        .assets
        .iter()
        .find(|asset| asset.name == SUMS)
        .map(|asset| asset.browser_download_url.clone());

    Ok(Some(Release {
        version,
        page: published.html_url,
        package,
        sums,
    }))
}

/// Deletes the staged update on tokio once no preparation is running.
fn forget(io: &Io) {
    io.spawn(async {
        let _staging = STAGING.lock().await;
        tokio::task::spawn_blocking(Staged::clear).await.ok();
    });
}

/// Downloads `release` and prepares it beside the running copy, unless an earlier run already
/// did. The result is not written down yet, which is left to the caller. A version an earlier
/// launch put in place without it starting is refused, so the card offers it instead.
/// One stage runs at a time, and a clear waits for it, since a look a change in settings
/// abandoned keeps running on tokio and prepares into the same place beside the running copy.
async fn stage(http: &reqwest::Client, release: &Release, channel: Channel) -> Result<Staged> {
    let _staging = STAGING.lock().await;
    let earlier = tokio::task::spawn_blocking(Staged::read).await?;
    if let Some(earlier) =
        earlier.filter(|earlier| earlier.version == release.version && earlier.channel == channel)
    {
        if earlier.tried {
            anyhow::bail!("an earlier launch could not start it");
        }
        if earlier
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.exists())
        {
            return Ok(earlier);
        }
    }
    tokio::task::spawn_blocking(Staged::clear).await?;

    let package = release
        .package
        .as_ref()
        .context("the release has no package")?;
    let sums = release
        .sums
        .as_deref()
        .context("the release has no checksums")?;
    let path = download(http, package, sums, &release.version).await?;
    let prepared = tokio::task::spawn_blocking(move || Install::current().prepare(&path)).await??;
    Ok(Staged {
        version: release.version.clone(),
        channel,
        prepared: Some(prepared),
        tried: false,
    })
}

/// Streams the asset into the update folder while hashing it, and keeps it only when the hash
/// matches the one the release lists.
async fn download(
    http: &reqwest::Client,
    package: &Asset,
    sums: &str,
    version: &str,
) -> Result<PathBuf> {
    let listed = http
        .get(sums)
        .header("User-Agent", AGENT)
        .send()
        .await
        .context("cannot reach the checksums")?
        .error_for_status()
        .context("the checksums are missing")?
        .text()
        .await
        .context("cannot read the checksums")?;
    let expected = listed
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim() == package.name)
        .map(|(sum, _)| sum.trim().to_ascii_lowercase())
        .context("the release lists no checksum for the update")?;

    let mut response = http
        .get(&package.browser_download_url)
        .header("User-Agent", AGENT)
        .send()
        .await
        .context("cannot reach the update")?
        .error_for_status()
        .context("the update is missing")?;

    let folder = staged::folder().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&folder).context("cannot make the update folder")?;
    let path = folder.join(format!("sonora-{version}-{}", package.name));
    let mut file = std::fs::File::create(&path).context("cannot keep the update")?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await.context("cannot read the update")? {
        hasher.update(&chunk);
        file.write_all(&chunk).context("cannot keep the update")?;
    }
    drop(file);

    let checksum: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if checksum != expected {
        std::fs::remove_file(&path).ok();
        anyhow::bail!("the update does not match its checksum");
    }
    Ok(path)
}

/// Whether `offered` on `channel` should replace the running build. Within the running build's
/// own channel only a newer build counts. From the other channel any build does, since choosing
/// that channel asks to move to it.
fn wanted(channel: Channel, offered: &str) -> bool {
    if channel != Channel::running() {
        return true;
    }
    match (ordered(offered), ordered(version())) {
        (Some(offered), Some(running)) => offered > running,
        _ => false,
    }
}

/// Splits a version into its release numbers and, for a nightly label such as
/// `0.44.2-nightly.20261026`, the date it was built. A stable version has 0 for the date.
fn ordered(version: &str) -> Option<(u64, u64, u64, u64)> {
    let (core, rest) = version
        .trim()
        .split_once(['-', '+'])
        .unwrap_or_else(|| (version.trim(), ""));
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let date = rest
        .strip_prefix("nightly.")
        .and_then(|date| date.parse().ok())
        .unwrap_or(0);
    Some((parts.next()??, parts.next()??, parts.next()??, date))
}
