use std::io::{self, BufReader, Read, Write};
use std::process::ExitCode;

use crate::args;
use crate::commands::{Format, Kind, Local, Repeat, Request};
use crate::wire::{self, Failure, Reply, Value};

/// The running Sonora refused the command, or the exchange broke.
const FAILED: u8 = 1;
/// No Sonora runs to send the command to. clap already takes 2 for usage errors.
const NOT_RUNNING: u8 = 3;
/// The headers of `sonora providers` and `sonora search`.
const PROVIDERS: [&str; 2] = ["provider", "status"];
const HITS: [&str; 5] = ["type", "title", "by", "length", "link"];

/// Carries out a command that needs no running Sonora.
pub fn local(local: Local) -> ExitCode {
    match local {
        Local::Completions(shell) => args::completions(shell),
    }
    ExitCode::SUCCESS
}

/// Sends `request` over `stream`, prints the reply in `format` and gives the code to exit with.
pub fn run<S: Read + Write>(stream: &mut S, request: &Request, format: Format) -> ExitCode {
    if request.streams() {
        return follow(stream, request);
    }
    // TODO: the messages here come from Fluent once render lands
    match wire::exchange(stream, request) {
        Ok(Reply::Done) => ExitCode::SUCCESS,
        Ok(Reply::Value(value)) => {
            let printed = match format {
                Format::Text => show(&value),
                Format::Json => json(&value),
            };
            if !printed.is_empty() {
                println!("{printed}");
            }
            ExitCode::SUCCESS
        }
        Ok(Reply::Failed(failure)) => {
            eprintln!("sonora: {}", describe(&failure));
            ExitCode::from(FAILED)
        }
        Err(wire::Error::Closed) => {
            eprintln!("sonora: the running Sonora did not answer; it may be older, restart it");
            ExitCode::from(FAILED)
        }
        Err(error) => {
            eprintln!("sonora: {error}");
            ExitCode::from(FAILED)
        }
    }
}

/// Sends a request answered by a stream, `sonora watch`, and prints each reply as one line of
/// JSON as it arrives, whatever the format, since a line per change is what a script reads.
/// It ends when Sonora closes the stream, by quitting, or at the first line written after
/// nothing reads them any more, as when they go into `head`.
fn follow<S: Read + Write>(stream: &mut S, request: &Request) -> ExitCode {
    if let Err(error) = wire::send(stream, request) {
        eprintln!("sonora: {error}");
        return ExitCode::from(FAILED);
    }
    let mut reader = BufReader::new(stream);
    let mut out = io::stdout().lock();
    loop {
        match wire::receive::<Reply>(&mut reader) {
            Ok(Reply::Value(value)) => {
                let written = writeln!(out, "{}", json(&value)).and_then(|()| out.flush());
                if written.is_err() {
                    return ExitCode::SUCCESS;
                }
            }
            Ok(Reply::Done) => {}
            Ok(Reply::Failed(failure)) => {
                eprintln!("sonora: {}", describe(&failure));
                return ExitCode::from(FAILED);
            }
            Err(wire::Error::Closed) => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("sonora: {error}");
                return ExitCode::from(FAILED);
            }
        }
    }
}

/// Says that no Sonora runs, for a command that has nowhere to go.
pub fn not_running() -> ExitCode {
    eprintln!("sonora: Sonora is not running");
    ExitCode::from(NOT_RUNNING)
}

/// A queried value for a person: the track as JSON, a list as a table, anything else as a
/// bare word.
fn show(value: &Value) -> String {
    match value {
        Value::Current(current) => serde_json::to_string_pretty(current).unwrap_or_default(),
        Value::Volume(level) => level.to_string(),
        Value::Shuffle(on) => match on {
            true => "on".to_owned(),
            false => "off".to_owned(),
        },
        Value::Repeat(repeat) => match repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
        .to_owned(),
        Value::Providers(providers) => table(
            PROVIDERS,
            providers.iter().map(|provider| {
                let status = match (provider.current, provider.stored) {
                    (true, _) => "current",
                    (false, true) => "connected",
                    (false, false) => "not connected",
                };
                [provider.slug.clone(), status.to_owned()]
            }),
        ),
        Value::Provider(slug) => slug.clone().unwrap_or_default(),
        Value::Hits(hits) if hits.is_empty() => String::new(),
        Value::Hits(hits) => table(
            HITS,
            hits.iter().map(|hit| {
                let kind = match hit.kind {
                    Kind::Song => "song",
                    Kind::Album => "album",
                    Kind::Artist => "artist",
                    Kind::Playlist => "playlist",
                };
                [
                    kind.to_owned(),
                    hit.title.clone(),
                    hit.by.clone(),
                    hit.duration.map(clock).unwrap_or_default(),
                    hit.link.clone(),
                ]
            }),
        ),
        Value::Session(None) => String::new(),
        Value::Session(Some(session)) => format!(
            "{} on {}{}\n{}",
            session.code,
            session.provider,
            match session.hosting {
                true => ", hosting",
                false => "",
            },
            session.members.join(", ")
        ),
    }
}

/// A queried value for a script, as one line of JSON without the envelope around it.
fn json(value: &Value) -> String {
    let printed = match value {
        Value::Current(current) => serde_json::to_string(current),
        Value::Volume(level) => serde_json::to_string(level),
        Value::Shuffle(on) => serde_json::to_string(on),
        Value::Repeat(repeat) => serde_json::to_string(repeat),
        Value::Providers(providers) => serde_json::to_string(providers),
        Value::Provider(slug) => serde_json::to_string(slug),
        Value::Hits(hits) => serde_json::to_string(hits),
        Value::Session(session) => serde_json::to_string(session),
    };
    printed.unwrap_or_default()
}

/// Rows under a header, every column but the last padded to its widest cell so the next one
/// lines up. The last is left as it is, so a line ends without trailing spaces.
fn table<const N: usize>(header: [&str; N], rows: impl Iterator<Item = [String; N]>) -> String {
    let rows: Vec<[String; N]> = std::iter::once(header.map(str::to_owned))
        .chain(rows)
        .collect();
    let mut widths = [0; N];
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (index, cell) in row.iter().enumerate() {
                match index + 1 == N {
                    true => line.push_str(cell),
                    false => line.push_str(&format!("{cell:<width$}  ", width = widths[index])),
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whole seconds as `m:ss`, or `h:mm:ss` past an hour.
fn clock(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match hours {
        0 => format!("{minutes}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

fn describe(failure: &Failure) -> String {
    match failure {
        Failure::Version(v) => format!("the running Sonora speaks protocol {v}; restart it"),
        Failure::Malformed => {
            "the running Sonora could not read the command; if it is older, restart it".to_owned()
        }
        Failure::Unsupported => "this command is not supported yet".to_owned(),
        Failure::SignedOut => "no provider is signed in".to_owned(),
        Failure::OtherProvider { link, current } => {
            format!("the link is for {link}, but {current} is the current provider")
        }
        Failure::Unqueueable(item) => format!("'{item}' cannot be queued"),
        Failure::Unreadable(path) => format!("cannot read '{path}'"),
        Failure::Unfetched(link) => format!("cannot fetch '{link}'"),
        Failure::UnknownProvider(slug) => {
            format!("no provider is called '{slug}'; see `sonora providers`")
        }
        Failure::NoAccount(name) => format!("{name} has no account to switch to; sign in first"),
        Failure::SignInFailed(name) => format!("{name} did not sign in"),
        Failure::Busy => "a sign-in is already on its way; try again in a moment".to_owned(),
        Failure::NothingFound(query) => format!("nothing found for '{query}'"),
    }
}
