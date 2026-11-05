use std::env;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use discord_rich_presence::DiscordIpc;
use discord_rich_presence::error::Error;

/// Environment variables Discord reads, in order, to pick the folder for its socket.
const ROOT_VARIABLES: [&str; 4] = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"];
/// Where Discord puts its socket when none of the variables is set. Every user can write there,
/// so it is searched only in that case.
const FALLBACK_ROOT: &str = "/tmp";
/// Folders under the root where a sandboxed Discord build or client keeps its socket. The empty
/// one is a native install, Canary and PTB included.
const SOCKET_DIRS: [&str; 9] = [
    "",
    "app/com.discordapp.Discord",
    "app/com.discordapp.DiscordCanary",
    "app/dev.vencord.Vesktop",
    ".flatpak/com.discordapp.Discord/xdg-run",
    ".flatpak/com.discordapp.DiscordCanary/xdg-run",
    ".flatpak/dev.vencord.Vesktop/xdg-run",
    "snap.discord",
    "snap.discord-canary",
];
/// Discord numbers its socket from 0 and takes the first free slot, so a second running client
/// such as Canary next to stable lands on 1.
const SOCKET_SLOTS: u8 = 10;
/// The largest frame accepted from the socket. Discord answers in a few kilobytes, and the
/// crate's own reader would allocate whatever length the header claims.
const MAX_FRAME_BYTES: u32 = 1 << 20;

/// A connection to Discord's IPC socket on Unix. It replaces the crate's client, which stops at
/// the first socket file it sees even when that file is stale, does not know the Flatpak Canary
/// path, and trusts whoever answers.
pub(super) struct Socket {
    client_id: String,
    stream: Option<UnixStream>,
}

impl Socket {
    pub(super) fn new(client_id: &str) -> Self {
        Self {
            client_id: client_id.to_owned(),
            stream: None,
        }
    }
}

impl DiscordIpc for Socket {
    fn get_client_id(&self) -> &str {
        &self.client_id
    }

    /// Connects to the first candidate socket that accepts and is served by our own user. A file
    /// left by a client that has since quit is skipped, and so is one another user planted.
    fn connect_ipc(&mut self) -> Result<(), Error> {
        let stream = candidates()
            .filter_map(|path| UnixStream::connect(path).ok())
            .find(is_ours)
            .ok_or(Error::IPCNotFound)?;
        self.stream = Some(stream);
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or(Error::NotConnected)?;
        stream.write_all(data).map_err(Error::WriteError)
    }

    /// Reads one frame like the crate's own `recv`, but refuses a length over `MAX_FRAME_BYTES`
    /// before allocating for it and drops the connection.
    fn recv(&mut self) -> Result<(u32, serde_json::Value), Error> {
        let mut header = [0; 8];
        self.read(&mut header)?;
        let [o0, o1, o2, o3, l0, l1, l2, l3] = header;
        let op = u32::from_le_bytes([o0, o1, o2, o3]);
        let length = u32::from_le_bytes([l0, l1, l2, l3]);
        if length > MAX_FRAME_BYTES {
            self.stream = None;
            return Err(Error::ReadError(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the socket sent a frame of {length} bytes"),
            )));
        }

        let mut data = vec![0; length as usize];
        self.read(&mut data)?;
        let answer = serde_json::from_slice(&data).map_err(|_| Error::JsonParseResponse)?;
        Ok((op, answer))
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or(Error::NotConnected)?;
        stream.read_exact(buffer).map_err(Error::ReadError)
    }

    fn close(&mut self) -> Result<(), Error> {
        self.stream = None;
        Ok(())
    }
}

/// Every path Discord's socket may have, most likely first. The shared `/tmp` is a candidate
/// only when no variable names a folder, which is also the only case where Discord uses it.
fn candidates() -> impl Iterator<Item = PathBuf> {
    let mut roots: Vec<PathBuf> = ROOT_VARIABLES
        .into_iter()
        .filter_map(env::var_os)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    if roots.is_empty() {
        roots.push(PathBuf::from(FALLBACK_ROOT));
    }
    roots
        .into_iter()
        .flat_map(|root| SOCKET_DIRS.map(|dir| root.join(dir)))
        .flat_map(|dir| (0..SOCKET_SLOTS).map(move |slot| dir.join(format!("discord-ipc-{slot}"))))
}

/// Whether the process on the other end of the socket runs as our user. The kernel reports the
/// connected peer, so the socket file cannot be swapped between the check and the connection.
fn is_ours(stream: &UnixStream) -> bool {
    // SAFETY: getuid always succeeds and touches no memory.
    let uid = unsafe { libc::getuid() };
    peer_uid(stream) == Some(uid)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Option<libc::uid_t> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the pointer and length describe a live `ucred`, which SO_PEERCRED fills.
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &mut length,
        )
    };
    (status == 0).then_some(credentials.uid)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_uid(stream: &UnixStream) -> Option<libc::uid_t> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: both pointers are live locals of the types getpeereid writes.
    let status = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (status == 0).then_some(uid)
}
