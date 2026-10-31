use std::io::{self, BufRead as _, BufReader, ErrorKind, Read, Write};
use std::thread;
use std::time::Duration;

use cli::commands::{Request, Watch};
use cli::wire::{self, Failure, Reply};
use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{ListenerOptions, Name, Stream};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

const SOCKET: &str = match cfg!(debug_assertions) {
    true => "sonora-dev.sock",
    false => "sonora.sock",
};
/// How long the running Sonora waits for a connection to say what it wants, so a stalled peer
/// never holds a thread for good.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a command line waits for its answer. A provider switch signs in and a playlist to
/// queue is fetched before the answer, which can take a while on a slow network.
const ANSWER: Duration = Duration::from_secs(60);

/// A command from the command line and where its answer goes.
pub enum Asked {
    /// A command answered once.
    Once(Request, oneshot::Sender<Reply>),
    /// `sonora watch`, answered again on every change until the command line hangs up and the
    /// receiver goes with it.
    Watch(Watch, UnboundedSender<Reply>),
}

pub enum Instance {
    First,
    Running,
    Failed,
}

/// `args` is every non-flag command-line argument of this launch: a lone `spotify:`/web link, or
/// one or more file paths from an "Open With" selection. Later launches hand their items to
/// `sender` and their commands to `commands`.
pub fn claim(
    args: &[String],
    sender: UnboundedSender<Vec<String>>,
    commands: UnboundedSender<Asked>,
) -> Instance {
    let name = match name() {
        Ok(name) => name,
        Err(error) => {
            log::warn!("single: cannot name the instance socket: {error:#}");
            return Instance::Failed;
        }
    };

    let listener = match options(name.clone()).create_sync() {
        Ok(listener) => listener,

        Err(error) => {
            // Windows reports an occupied pipe with an error other than
            // AddrInUse, so any failure may mean another Sonora owns the socket.
            if hand_over(name.clone(), args) {
                return Instance::Running;
            }

            // Only a filesystem socket can leave a stale AddrInUse behind.
            if error.kind() != ErrorKind::AddrInUse {
                log::error!("single: cannot own the instance socket: {error:#}");
                return Instance::Failed;
            }

            log::warn!("single: socket is occupied but unreachable; attempting recovery");

            match options(name.clone())
                .try_overwrite(true)
                .max_spin_time(Duration::ZERO)
                .create_sync()
            {
                Ok(listener) => listener,
                Err(error) => {
                    log::error!("single: cannot recover instance socket: {error:#}");
                    return Instance::Failed;
                }
            }
        }
    };

    thread::spawn(move || {
        for connection in listener.incoming() {
            if sender.is_closed() {
                break;
            }
            let Ok(connection) = connection else {
                continue;
            };
            let (sender, commands) = (sender.clone(), commands.clone());
            thread::spawn(move || serve(connection, &sender, &commands));
        }
    });

    Instance::First
}

/// The running Sonora's instance socket, for a command line that talks to it. One whose answer
/// `streams` waits for each reply as long as it takes, since a watch is quiet until something
/// changes.
pub fn connect(streams: bool) -> io::Result<Stream> {
    let stream = Stream::connect(name()?)?;
    stream.set_recv_timeout(match streams {
        true => None,
        false => Some(ANSWER),
    })?;
    Ok(stream)
}

/// Answers one connection. A line of JSON is a command and gets a reply; anything else is
/// the items of a launch handed over, read until the other end closes.
fn serve(
    connection: Stream,
    sender: &UnboundedSender<Vec<String>>,
    commands: &UnboundedSender<Asked>,
) {
    if let Err(error) = connection.set_recv_timeout(Some(PATIENCE)) {
        log::warn!("single: cannot bound the read: {error:#}");
    }
    let mut reader = BufReader::new(&connection);
    let command = match reader.fill_buf() {
        Ok(buffer) => buffer.first() == Some(&b'{'),
        Err(_) => return,
    };
    if command {
        let reply = match wire::receive::<Request>(&mut reader) {
            Ok(Request::Watch(watch)) => return follow(&connection, watch, commands),
            Ok(request) => {
                let (reply, answered) = oneshot::channel();
                if commands.send(Asked::Once(request, reply)).is_err() {
                    return;
                }
                match answered.blocking_recv() {
                    Ok(reply) => reply,
                    Err(_) => return,
                }
            }
            Err(wire::Error::Version(_)) => Reply::Failed(Failure::Version(wire::VERSION)),
            Err(error) => {
                log::warn!("single: cannot read a command: {error}");
                Reply::Failed(Failure::Malformed)
            }
        };
        if let Err(error) = wire::send(&mut &connection, &reply) {
            log::warn!("single: cannot answer a command: {error}");
        }
        return;
    }

    let mut payload = String::new();
    if reader.read_to_string(&mut payload).is_err() {
        return;
    }
    let items: Vec<String> = payload
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    sender.send(items).ok();
}

/// Keeps a `sonora watch` connection open, writing each reply the app sends until the command
/// line hangs up. A write that fails ends it, which drops the receiver, so the app stops
/// sending to it.
fn follow(connection: &Stream, watch: Watch, commands: &UnboundedSender<Asked>) {
    let (sink, mut replies) = mpsc::unbounded_channel();
    if commands.send(Asked::Watch(watch, sink)).is_err() {
        return;
    }
    while let Some(reply) = replies.blocking_recv() {
        if let Err(error) = wire::send(&mut &*connection, &reply) {
            log::debug!("single: a watch ended: {error}");
            return;
        }
    }
}

fn hand_over(name: Name<'_>, args: &[String]) -> bool {
    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        Err(error) => {
            log::warn!("single: cannot reach the running instance: {error:#}");
            return false;
        }
    };
    stream.write_all(args.join("\n").as_bytes()).is_ok()
}

/// The instance socket as a file in a folder only this user can enter, so another account can
/// neither take the name first nor connect to it.
#[cfg(unix)]
fn name() -> io::Result<Name<'static>> {
    use interprocess::local_socket::{GenericFilePath, ToFsName as _};

    private_dir()?.join(SOCKET).to_fs_name::<GenericFilePath>()
}

/// The instance socket as a named pipe, which `reachable` closes to other accounts.
#[cfg(windows)]
fn name() -> io::Result<Name<'static>> {
    use interprocess::local_socket::{GenericNamespaced, ToNsName as _};

    SOCKET.to_ns_name::<GenericNamespaced>()
}

/// The user's runtime folder, which the session manager creates readable by this user alone. A
/// Flatpak sandbox gets a fresh one per launch, so there the app's own subfolder, shared by every
/// launch, is used instead. Without a runtime folder the socket goes in the user's cache folder.
#[cfg(all(unix, not(target_os = "macos")))]
fn private_dir() -> io::Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;
    use std::path::PathBuf;

    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let dir = match (runtime, std::env::var_os("FLATPAK_ID")) {
        (Some(runtime), Some(app)) => runtime.join("app").join(app),
        (Some(runtime), None) => return Ok(runtime),
        (None, _) => dirs::cache_dir()
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no runtime or cache folder"))?
            .join("sonora"),
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    Ok(dir)
}

/// The per-user temporary folder macOS keeps under `/var/folders`, which only this user can
/// enter. It is asked of the system rather than read from `TMPDIR`, so a launch from a shell
/// that changed the variable still finds the same socket.
#[cfg(target_os = "macos")]
fn private_dir() -> io::Result<std::path::PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: confstr writes at most `buffer.len()` bytes into the buffer it is given.
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if length == 0 || length > buffer.len() {
        return Err(io::Error::last_os_error());
    }
    // the length counts the terminating NUL
    buffer.truncate(length - 1);
    Ok(OsString::from_vec(buffer).into())
}

/// The listener for the instance socket, open to every Sonora this user launches.
fn options(name: Name<'_>) -> ListenerOptions<'_> {
    let options = ListenerOptions::new().name(name);
    #[cfg(windows)]
    let options = match reachable() {
        Some(descriptor) => {
            use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
            options.security_descriptor(descriptor)
        }
        None => options,
    };
    options
}

/// Access to the pipe for this user's account alone, elevated or not. Windows owns an elevated
/// process's objects as Administrators, so neither the default descriptor nor one granting the
/// owner lets the same user's taskbar launch reach a Sonora the updater relaunched elevated, and
/// that launch would exit without a window. `None` keeps the default, and is logged.
#[cfg(windows)]
fn reachable() -> Option<interprocess::os::windows::security_descriptor::SecurityDescriptor> {
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;

    let described = user_sid()
        .and_then(|sid| {
            widestring::U16CString::from_str(format!("D:(A;;GA;;;{sid})"))
                .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
        })
        .and_then(|descriptor| SecurityDescriptor::deserialize(&descriptor));
    match described {
        Ok(descriptor) => Some(descriptor),
        Err(error) => {
            log::warn!("single: cannot describe the socket's access: {error:#}");
            None
        }
    }
}

/// The SID of the account this process runs as, in its `S-1-5-…` text form.
#[cfg(windows)]
fn user_sid() -> io::Result<String> {
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the pseudo handle of the current process needs no closing, and `token` is a live
    // out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut length = 0u32;
    // SAFETY: a null buffer of length zero only asks for the size the answer needs.
    unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut length) };
    // u64 cells keep the buffer aligned for the pointer at the start of TOKEN_USER
    let mut buffer = vec![0u64; (length as usize).div_ceil(size_of::<u64>())];
    // SAFETY: the buffer holds at least `length` bytes, and the token is open until closed below.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: the token was opened above and is not used again.
    unsafe { CloseHandle(token) };
    if read == 0 {
        return Err(error);
    }

    // SAFETY: GetTokenInformation filled the buffer with a TOKEN_USER whose SID points into it.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text = ptr::null_mut();
    // SAFETY: `sid` is valid while `buffer` lives, and `text` is a live out pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ConvertSidToStringSidW returned a NUL-terminated string that LocalFree releases.
    let sid = unsafe { widestring::U16CStr::from_ptr_str(text) }.to_string_lossy();
    unsafe { LocalFree(text.cast()) };
    Ok(sid)
}
