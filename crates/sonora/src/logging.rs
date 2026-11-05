use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use env_logger::{Env, Logger, Target};
use log::{Log, Metadata, Record};

const CONSOLE: &str = "warn,symphonia=error,lofty=error,discord_rich_presence=error";
const DISK: &str = "warn,symphonia=error,lofty=error,discord_rich_presence=error,sonora=debug,ui=debug,music=debug,ytmusic=debug,state::potoken=debug";
const FILTER: &str = "SONORA_LOG";
const PREVIOUS: &str = "sonora.log.1";
const LIMIT: u64 = 16 * 1024 * 1024;
/// How many runs the log keeps, counting the one that is starting.
const SESSIONS: usize = 5;
/// The first line of every run, which is how a later start finds where each run began.
const STARTED: &str = "logging: sonora";

/// Logs to stderr only, for a process that is not the app, such as the Widevine host.
pub fn console() {
    env_logger::Builder::from_env(Env::default().default_filter_or(CONSOLE))
        .format_timestamp(None)
        .init();
}

pub fn init() {
    let console = env_logger::Builder::from_env(Env::default().default_filter_or(CONSOLE))
        .format_timestamp(None)
        .format_module_path(false)
        .build();

    let disk = open().map(|file| {
        env_logger::Builder::from_env(Env::new().filter_or(FILTER, DISK))
            .target(Target::Pipe(Box::new(file)))
            .build()
    });

    let (level, logger): (log::LevelFilter, Box<dyn Log>) = match disk {
        None => (console.filter(), Box::new(console)),
        Some(disk) => (
            console.filter().max(disk.filter()),
            Box::new(Fan { console, disk }),
        ),
    };

    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(level);
    }
    catch_panics();

    log::debug!("{STARTED} {} started", state::VERSION);
}

/// Writes a panic to the log before the default hook prints it. A Sonora started from a
/// desktop entry or a tray has no terminal to lose it to, so this is the only place a crash
/// leaves a trace. `RUST_BACKTRACE` still decides whether there is a backtrace to write.
fn catch_panics() {
    let default = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let backtrace = std::backtrace::Backtrace::capture();
        match backtrace.status() {
            std::backtrace::BacktraceStatus::Captured => {
                log::error!("panic: {panic} on the {name} thread\n{backtrace}")
            }
            _ => log::error!("panic: {panic} on the {name} thread"),
        }
        log::logger().flush();
        default(panic);
    }));
}

/// The log file under the size limit: a write that would carry it past `LIMIT` rotates it
/// first and lands in a fresh file, so a flood of lines churns through the two files instead
/// of filling the disk.
struct Capped {
    path: PathBuf,
    file: File,
    written: u64,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let overflowing = self.written + buf.len() as u64 > LIMIT;
        if overflowing && self.written > 0 {
            rotate(&self.path);
            if let Some(file) = append(&self.path) {
                self.file = file;
                self.written = 0;
            }
        }
        let count = self.file.write(buf)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct Fan {
    console: Logger,
    disk: Logger,
}

impl Log for Fan {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.console.enabled(metadata) || self.disk.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.console.log(record);
        self.disk.log(record);
    }

    fn flush(&self) {
        self.console.flush();
        self.disk.flush();
    }
}

fn open() -> Option<Capped> {
    let path = state::log_file()?;
    fs::create_dir_all(path.parent()?).ok()?;
    trim(&path);

    let outgrown = fs::metadata(&path).is_ok_and(|file| file.len() > LIMIT);
    if outgrown {
        rotate(&path);
    }

    let file = append(&path)?;
    let written = file.metadata().map(|file| file.len()).unwrap_or(0);
    Some(Capped {
        path,
        file,
        written,
    })
}

/// Cuts the log down to its last `SESSIONS - 1` runs, so the run that is starting makes
/// `SESSIONS`. Once the log holds that many, everything in `PREVIOUS` is older, so it goes too.
fn trim(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let marker = format!("] {STARTED} ");
    let starts: Vec<usize> = text
        .match_indices(&marker)
        .map(|(at, _)| text[..at].rfind('\n').map_or(0, |line| line + 1))
        .collect();
    let Some(&keep) = starts
        .len()
        .checked_sub(SESSIONS - 1)
        .and_then(|first| starts.get(first))
    else {
        return;
    };
    if let Some(folder) = path.parent() {
        let _ = fs::remove_file(folder.join(PREVIOUS));
    }
    if keep > 0 {
        let _ = fs::write(path, &text[keep..]);
    }
}

fn append(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Moves the file to `PREVIOUS`, dropping what was there, so however much is written the
/// two files together never exceed twice the limit.
fn rotate(path: &Path) {
    let Some(folder) = path.parent() else { return };
    let _ = fs::rename(path, folder.join(PREVIOUS));
}
