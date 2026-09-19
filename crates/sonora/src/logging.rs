use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use env_logger::{Env, Logger, Target};
use log::{Log, Metadata, Record};

const CONSOLE: &str = "warn,symphonia=error,lofty=error,discord_rich_presence=error";
const DISK: &str = "warn,symphonia=error,lofty=error,discord_rich_presence=error,sonora=debug,ui=debug,music=debug,ytmusic=debug";
const FILTER: &str = "SONORA_LOG";
const PREVIOUS: &str = "sonora.log.1";

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

    log::debug!("logging: sonora {} started", env!("CARGO_PKG_VERSION"));
}

/// The log file under the size limit: a write that would carry it past `state::log_limit()`
/// rotates it first and lands in a fresh file, so a flood of lines churns through the two
/// files instead of filling the disk. The limit is read on every write, since Settings can
/// change it while running.
struct Capped {
    path: PathBuf,
    file: File,
    written: u64,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let overflowing = self.written + buf.len() as u64 > state::log_limit();
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

    let outgrown = fs::metadata(&path).is_ok_and(|file| file.len() > state::log_limit());
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

fn append(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Moves the file to `PREVIOUS`, dropping what was there, so however much is written the
/// two files together never exceed twice the limit.
fn rotate(path: &Path) {
    let Some(folder) = path.parent() else { return };
    let _ = fs::rename(path, folder.join(PREVIOUS));
}
