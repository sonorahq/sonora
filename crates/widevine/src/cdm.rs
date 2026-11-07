//! The system CDM, hosted in a helper process the way a browser runs its CDM in a utility
//! process.
//!
//! The module never maps into the app. [`Cdm::open`] starts the Sonora executable again as the
//! host in [`crate::host`] and talks to it over its stdin and stdout, and every [`Cdm`] is a
//! lease on that one process. There is one host at a time, every call to it is serialized, and
//! its session accumulates content keys rather than replacing them, so more than one track can
//! be licensed at once. That is what makes preloading the next track possible.
//!
//! The host stays up while any lease is held and for a grace period after the last one goes, so
//! a track change does not restart it. Then its stdin is closed and it exits, taking the module's
//! memory with it. A host that takes too long over one call is killed, so a wedged module costs
//! one failed track rather than a hung player.

#[cfg(feature = "cdm")]
mod host {
    use std::io::{self, BufReader, BufWriter, Read as _, Write as _};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError, Weak};
    use std::time::{Duration, Instant};

    use anyhow::{Context as _, Result, anyhow, bail};

    use crate::wire;

    /// How long the host outlives its last lease. A track change or a preload lands well
    /// inside it, and the next protected track after a longer pause pays one start.
    const GRACE: Duration = Duration::from_secs(30);

    /// How long the host gets to open the module or to answer one request. A decrypt takes a
    /// couple of milliseconds and a license well under a second, so a host past this is wedged.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// How often the watcher looks at the host's deadline and its leases.
    const TICK: Duration = Duration::from_secs(1);

    /// How long a closing host gets to exit on its own before it is killed.
    const EXIT_WAIT: Duration = Duration::from_secs(2);

    /// Room for a whole request, so a sample leaves in one write.
    const PIPE_BUFFER: usize = 64 * 1024;

    /// Keeps the console window away from the host on Windows.
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The running host and how many leases hold it. A host that died stays here until the
    /// next [`Cdm::open`] notices and replaces it.
    struct Slot {
        link: Option<Arc<Link>>,
        leases: usize,
        /// When the last lease went, while none is held.
        idle: Option<Instant>,
        /// Set while one caller starts a host outside the lock. Others wait on [`STARTED`].
        starting: bool,
    }

    static SLOT: Mutex<Slot> = Mutex::new(Slot {
        link: None,
        leases: 0,
        idle: None,
        starting: false,
    });

    /// Signalled when a start ends, either way.
    static STARTED: Condvar = Condvar::new();

    /// A lease on the CDM host. Clones share the host, every call takes the same lock in the
    /// order it arrives, and the host stays up while any lease is alive.
    ///
    /// A lease on a host that has since died keeps failing. The next [`Cdm::open`] starts a
    /// fresh host, whose session has none of the old keys.
    pub struct Cdm {
        link: Arc<Link>,
    }

    /// One running host process.
    struct Link {
        io: Mutex<Pipe>,
        /// The process, locked only for a moment to look at it or kill it, never across a
        /// call. Taken when the link goes, to be reaped.
        child: Mutex<Option<Child>>,
        /// Set once the pipe fails, after which nothing is sent and the host is replaced.
        broken: AtomicBool,
        /// When the request in flight has to be answered by, in milliseconds since `born`, or
        /// zero while nothing is in flight.
        deadline: AtomicU64,
        born: Instant,
        pid: u32,
    }

    /// Both directions of the host's pipe.
    struct Pipe {
        /// Taken when the host is closed, since its end of stdin closing is what makes it exit.
        to: Option<BufWriter<ChildStdin>>,
        from: BufReader<ChildStdout>,
        /// Where a failure message or a dropped answer is read, kept between calls.
        scratch: Vec<u8>,
    }

    /// Locks the slot. Its fields are only ever written together under the lock, so a panic
    /// elsewhere leaves nothing half-done and poisoning is ignored.
    fn held() -> MutexGuard<'static, Slot> {
        SLOT.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Cdm {
        /// Starts the host for the CDM [`crate::find`] settles on, or hands back a lease on the
        /// one already running. The start happens outside the slot lock, so leases elsewhere
        /// come and go while the module loads.
        pub fn open() -> Result<Self> {
            let mut slot = held();
            loop {
                if let Some(link) = slot.link.as_ref().filter(|link| link.alive()) {
                    let link = link.clone();
                    slot.leases += 1;
                    slot.idle = None;
                    return Ok(Self { link });
                }
                if !slot.starting {
                    break;
                }
                slot = STARTED.wait(slot).unwrap_or_else(PoisonError::into_inner);
            }
            if let Some(stale) = slot.link.take() {
                log::warn!(
                    "widevine: cdm host {} is gone, starting a fresh one",
                    stale.pid
                );
                stale.abandon();
            }
            slot.leases = 0;
            slot.idle = None;
            slot.starting = true;
            drop(slot);

            let started = start();
            let mut slot = held();
            slot.starting = false;
            STARTED.notify_all();
            let link = started?;
            slot.link = Some(link.clone());
            slot.leases = 1;
            Ok(Self { link })
        }

        /// The license challenge for `init`, a `pssh` box. What comes back goes to the
        /// provider's license endpoint untouched.
        pub fn challenge(&self, init: &[u8]) -> Result<Vec<u8>> {
            self.link
                .call(wire::CHALLENGE, init.len(), |to| to.write_all(init), take)
                .context("cannot build a challenge")
        }

        /// Hands the license back to the CDM, which loads the content keys it carries. An
        /// earlier track's keys stay loaded beside them.
        pub fn accept(&self, license: &[u8]) -> Result<()> {
            self.link
                .call(
                    wire::UPDATE,
                    license.len(),
                    |to| to.write_all(license),
                    drain,
                )
                .context("the widevine cdm refused the license")
        }

        /// Decrypts one CENC sample in place. `subs` is empty when the whole sample is
        /// encrypted. On failure the sample is left as it was or partly overwritten, and either
        /// way must not be served.
        pub fn decrypt(
            &self,
            sample: &mut [u8],
            key_id: &[u8],
            iv: &[u8],
            subs: &[(u32, u32)],
        ) -> Result<()> {
            anyhow::ensure!(
                matches!(iv.len(), 8 | 16),
                "a CENC IV must be 8 or 16 bytes"
            );
            let len = wire::decrypt_len(iv, key_id, subs, sample);
            let fitted = self
                .link
                .send(wire::DECRYPT, len, |to| {
                    wire::put_decrypt(to, iv, key_id, subs, sample)
                })
                .and_then(|mut pipe| {
                    self.link
                        .receive(&mut pipe, |pipe, len| match len == sample.len() {
                            true => pipe.from.read_exact(sample).map(|()| true),
                            false => drain(pipe, len).map(|()| false),
                        })
                })
                .context("cannot decrypt a sample")?;
            match fitted {
                true => Ok(()),
                false => bail!("the widevine cdm returned a sample of the wrong length"),
            }
        }
    }

    impl Clone for Cdm {
        fn clone(&self) -> Self {
            let mut slot = held();
            if slot
                .link
                .as_ref()
                .is_some_and(|link| Arc::ptr_eq(link, &self.link))
            {
                slot.leases += 1;
            }
            Self {
                link: self.link.clone(),
            }
        }
    }

    impl Drop for Cdm {
        /// Gives the lease back, and starts the grace period when it was the last one.
        fn drop(&mut self) {
            let mut slot = held();
            if !slot
                .link
                .as_ref()
                .is_some_and(|link| Arc::ptr_eq(link, &self.link))
            {
                return;
            }
            slot.leases = slot.leases.saturating_sub(1);
            if slot.leases == 0 {
                slot.idle = Some(Instant::now());
            }
        }
    }

    /// Finds the module and starts a host for it, with a watcher beside it.
    fn start() -> Result<Arc<Link>> {
        let Some(found) = crate::find() else {
            bail!("no widevine module was found, so there is no cdm to open");
        };
        log::info!(
            "widevine: opening the {:?} cdm at {}",
            found.origin,
            found.path.display()
        );
        let link = Link::start(&found.path)
            .with_context(|| format!("cannot open the widevine cdm at {}", found.path.display()))?;
        log::info!("widevine: cdm host {} is up", link.pid);
        Ok(link)
    }

    impl Link {
        /// Starts the host for the module at `module` and waits for it to say the module
        /// opened. A host that fails to open it exits, and its reason becomes the error here.
        fn start(module: &Path) -> Result<Arc<Self>> {
            let mut command = Command::new(executable()?);
            #[cfg(unix)]
            if let Ok(named) = std::env::current_exe() {
                use std::os::unix::process::CommandExt as _;
                command.arg0(named);
            }
            command
                .arg(crate::HOST_ARG)
                .arg(module)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            let mut child = command.spawn().context("cannot start the cdm host")?;
            let (Some(to), Some(from)) = (child.stdin.take(), child.stdout.take()) else {
                bail!("cannot reach the cdm host over its pipes");
            };
            let link = Arc::new(Self {
                io: Mutex::new(Pipe {
                    to: Some(BufWriter::with_capacity(PIPE_BUFFER, to)),
                    from: BufReader::with_capacity(PIPE_BUFFER, from),
                    scratch: Vec::new(),
                }),
                pid: child.id(),
                child: Mutex::new(Some(child)),
                broken: AtomicBool::new(false),
                deadline: AtomicU64::new(0),
                born: Instant::now(),
            });
            let watched = Arc::downgrade(&link);
            std::thread::Builder::new()
                .name("widevine-host".into())
                .spawn(move || watch(watched))
                .context("cannot watch the cdm host")?;

            let mut pipe = link
                .io
                .lock()
                .map_err(|_| anyhow!("the cdm host is poisoned"))?;
            link.arm();
            let hello = pipe.answer(take);
            link.disarm();
            let version = hello
                .context("cannot hear from the cdm host")?
                .map_err(|message| anyhow!(message))?;
            if version != wire::VERSION.as_bytes() {
                bail!(
                    "the cdm host is version {}, not {}, so sonora was replaced while running",
                    String::from_utf8_lossy(&version),
                    wire::VERSION
                );
            }
            drop(pipe);
            Ok(link)
        }

        /// Whether the host can still be asked anything. A host that exited between tracks is
        /// caught here rather than by the next track's challenge. One busy with a call counts as
        /// alive, which is safe because a call that overruns gets the host killed.
        fn alive(&self) -> bool {
            if self.broken.load(Ordering::Acquire) {
                return false;
            }
            match self.child.try_lock() {
                Ok(mut child) => child
                    .as_mut()
                    .is_some_and(|child| matches!(child.try_wait(), Ok(None))),
                Err(TryLockError::WouldBlock) => true,
                Err(TryLockError::Poisoned(_)) => false,
            }
        }

        /// Lets go of a host that has been replaced while stale leases still hold it. Its stdin
        /// closes so a live one exits, and a dead one is reaped rather than left a zombie until
        /// the last stale lease goes. The pipe is skipped when a call holds it.
        fn abandon(&self) {
            self.broken.store(true, Ordering::Release);
            if let Ok(mut pipe) = self.io.try_lock() {
                drop(pipe.to.take());
            }
            if let Ok(mut child) = self.child.try_lock()
                && let Some(child) = child.as_mut()
            {
                child.try_wait().ok();
            }
        }

        /// Milliseconds since the link was made, the unit of [`Link::deadline`].
        fn now(&self) -> u64 {
            self.born.elapsed().as_millis() as u64 + 1
        }

        /// Gives the call about to block [`PATIENCE`] to finish.
        fn arm(&self) {
            let by = self.now() + PATIENCE.as_millis() as u64;
            self.deadline.store(by, Ordering::Release);
        }

        /// Clears the deadline once the call has its answer or has failed.
        fn disarm(&self) {
            self.deadline.store(0, Ordering::Release);
        }

        /// Kills the host when the call in flight has overrun its deadline. The blocked read
        /// then sees the pipe close, fails, and retires the link.
        fn enforce(&self) {
            let by = self.deadline.load(Ordering::Acquire);
            if by == 0 || self.now() <= by {
                return;
            }
            self.disarm();
            self.broken.store(true, Ordering::Release);
            log::warn!(
                "widevine: cdm host {} did not answer within {}s, killing it",
                self.pid,
                PATIENCE.as_secs()
            );
            if let Ok(mut child) = self.child.lock()
                && let Some(child) = child.as_mut()
            {
                child.kill().ok();
            }
        }

        /// Sends one request and reads its answer, for a request whose payload and answer
        /// do not share a buffer.
        fn call<T>(
            &self,
            op: u8,
            len: usize,
            body: impl FnOnce(&mut BufWriter<ChildStdin>) -> io::Result<()>,
            read: impl FnOnce(&mut Pipe, usize) -> io::Result<T>,
        ) -> Result<T> {
            let mut pipe = self.send(op, len, body)?;
            self.receive(&mut pipe, read)
        }

        /// Sends one request, where `body` writes exactly `len` payload bytes, and hands back
        /// the pipe still locked so its answer is the next thing read. The deadline runs from
        /// here until [`Link::receive`] has the answer.
        fn send(
            &self,
            op: u8,
            len: usize,
            body: impl FnOnce(&mut BufWriter<ChildStdin>) -> io::Result<()>,
        ) -> Result<MutexGuard<'_, Pipe>> {
            let mut pipe = self
                .io
                .lock()
                .map_err(|_| anyhow!("the cdm host is poisoned"))?;
            if self.broken.load(Ordering::Acquire) {
                bail!("the cdm host {} has stopped", self.pid);
            }
            self.arm();
            match pipe.request(op, len, body) {
                Ok(()) => Ok(pipe),
                Err(error) => {
                    self.disarm();
                    Err(self.retire(error))
                }
            }
        }

        /// Reads the answer to the request just sent, where `read` takes exactly the answer's
        /// length off the pipe. A failure the module reports leaves the host usable.
        fn receive<T>(
            &self,
            pipe: &mut Pipe,
            read: impl FnOnce(&mut Pipe, usize) -> io::Result<T>,
        ) -> Result<T> {
            let answered = pipe.answer(read);
            self.disarm();
            match answered {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(message)) => Err(anyhow!(message)),
                Err(error) => Err(self.retire(error)),
            }
        }

        /// Marks the host unusable after its pipe failed, so the next [`Cdm::open`] replaces
        /// it, and says so in the log.
        fn retire(&self, error: io::Error) -> anyhow::Error {
            self.broken.store(true, Ordering::Release);
            log::warn!(
                "widevine: cdm host {} stopped answering, the next track starts a fresh one: {error}",
                self.pid
            );
            anyhow!(error).context("cannot reach the cdm host")
        }
    }

    impl Drop for Link {
        /// Closes stdin so the host exits, and reaps it on a thread of its own, so whoever
        /// drops the last lease never waits on it.
        fn drop(&mut self) {
            if let Ok(pipe) = self.io.get_mut() {
                drop(pipe.to.take());
            }
            let Some(mut child) = self.child.get_mut().ok().and_then(Option::take) else {
                return;
            };
            if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                return;
            }
            let reaper = std::thread::Builder::new()
                .name("widevine-reap".into())
                .spawn(move || reap(child));
            if let Err(error) = reaper {
                log::warn!("widevine: cannot reap the cdm host: {error}");
            }
        }
    }

    impl Pipe {
        /// Writes one request and flushes it.
        fn request(
            &mut self,
            op: u8,
            len: usize,
            body: impl FnOnce(&mut BufWriter<ChildStdin>) -> io::Result<()>,
        ) -> io::Result<()> {
            let to = self
                .to
                .as_mut()
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
            wire::put_header(to, op, len)?;
            body(to)?;
            to.flush()
        }

        /// Reads one answer: the payload through `read` when it succeeded, or the message the
        /// host failed with.
        fn answer<T>(
            &mut self,
            read: impl FnOnce(&mut Pipe, usize) -> io::Result<T>,
        ) -> io::Result<Result<T, String>> {
            let (status, len) = wire::take_header(&mut self.from)?;
            match status {
                wire::OK => read(self, len).map(Ok),
                wire::FAILED => {
                    self.scratch.resize(len, 0);
                    self.from.read_exact(&mut self.scratch)?;
                    Ok(Err(String::from_utf8_lossy(&self.scratch).into_owned()))
                }
                status => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the cdm host answered with status {status}"),
                )),
            }
        }
    }

    /// The file to start the host from. On Linux that is the running image itself, which
    /// still opens after a package upgrade has replaced the file on disk.
    #[cfg(target_os = "linux")]
    fn executable() -> Result<PathBuf> {
        Ok(PathBuf::from("/proc/self/exe"))
    }

    /// The file to start the host from. Elsewhere the path is all there is, and a build
    /// replaced under the app is caught by the version in the host's first answer.
    #[cfg(not(target_os = "linux"))]
    fn executable() -> Result<PathBuf> {
        std::env::current_exe().context("cannot find the sonora executable")
    }

    /// Waits for a host whose stdin has closed to exit, and kills it if it does not.
    fn reap(mut child: Child) {
        let began = Instant::now();
        while began.elapsed() < EXIT_WAIT {
            match child.try_wait() {
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Ok(Some(_)) | Err(_) => return,
            }
        }
        log::warn!("widevine: cdm host {} did not exit, killing it", child.id());
        child.kill().ok();
        child.wait().ok();
    }

    /// Takes an answer's payload as a fresh buffer.
    fn take(pipe: &mut Pipe, len: usize) -> io::Result<Vec<u8>> {
        let mut out = vec![0; len];
        pipe.from.read_exact(&mut out)?;
        Ok(out)
    }

    /// Reads past an answer's payload.
    fn drain(pipe: &mut Pipe, len: usize) -> io::Result<()> {
        pipe.scratch.resize(len, 0);
        pipe.from.read_exact(&mut pipe.scratch)
    }

    /// Watches one host for as long as it exists: kills it when a call overruns, and closes it
    /// once it has been the current host without a lease for [`GRACE`].
    fn watch(watched: Weak<Link>) {
        loop {
            std::thread::sleep(TICK);
            let Some(link) = watched.upgrade() else {
                return;
            };
            link.enforce();
            let mut slot = held();
            let current = slot
                .link
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &link));
            let idle = slot.leases == 0 && slot.idle.is_some_and(|since| since.elapsed() >= GRACE);
            if current && idle {
                slot.link = None;
                slot.idle = None;
                drop(slot);
                log::info!("widevine: closing idle cdm host {}", link.pid);
                return;
            }
        }
    }
}

#[cfg(not(feature = "cdm"))]
mod host {
    use anyhow::{Result, bail};

    /// A handle to a CDM this build has no host for.
    #[derive(Clone)]
    pub struct Cdm;

    impl Cdm {
        pub fn open() -> Result<Self> {
            bail!("this build carries no widevine host")
        }

        pub fn challenge(&self, _init: &[u8]) -> Result<Vec<u8>> {
            bail!("this build carries no widevine host")
        }

        pub fn accept(&self, _license: &[u8]) -> Result<()> {
            bail!("this build carries no widevine host")
        }

        pub fn decrypt(
            &self,
            _sample: &mut [u8],
            _key_id: &[u8],
            _iv: &[u8],
            _subs: &[(u32, u32)],
        ) -> Result<()> {
            bail!("this build carries no widevine host")
        }
    }
}

pub use host::Cdm;
