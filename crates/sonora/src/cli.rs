use clap::error::ErrorKind;
use clap::{Arg, ArgAction, Parser};

/// Sonora's command line. Everything on it is optional, since a launcher usually starts Sonora
/// with nothing or with the files and links to open.
#[derive(Parser)]
#[command(
    name = "sonora",
    version = state::VERSION,
    about = "A native music streaming client",
    disable_version_flag = true,
    arg = Arg::new("version")
        .short('v')
        .long("version")
        .action(ArgAction::Version)
        .help("Print the version"),
)]
pub struct Cli {
    /// Files to play or links to open
    #[arg(value_name = "FILE_OR_LINK")]
    pub opened: Vec<String>,
}

impl Cli {
    /// Reads the command line. The help and the version print and exit. Any other mistake, such
    /// as a flag a launcher added that Sonora does not know, leaves the flags out and keeps the
    /// rest as things to open, so Sonora still starts.
    pub fn read() -> Self {
        match Self::try_parse() {
            Ok(cli) => cli,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
            {
                attach_console();
                error.exit()
            }
            Err(_) => Self {
                opened: std::env::args()
                    .skip(1)
                    .filter(|arg| !arg.starts_with('-'))
                    .collect(),
            },
        }
    }
}

/// A Windows release build has no console of its own, so it borrows the one of the terminal that
/// started it before printing. Without one there is nowhere to print, which is not an error.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: AttachConsole takes no pointers and only reports whether a console was attached.
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
