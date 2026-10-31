# cli

Sonora's command line: control a running Sonora from a terminal or a script. 
It is the same `sonora` binary the app starts from, so there is nothing else to install.

```sh
sonora toggle
sonora current | jq .track.title
sonora search crescent shaped depression | fzf | awk '{print $NF}' | xargs sonora play-next
```

## Commands

| Command | What it does |
|---|---|
| `sonora` | Start Sonora, or bring the running one to the front |
| `sonora <file\|link>...` | Play files, or open a provider's link, as a file manager's "Open With" does |
| `sonora play` / `pause` / `toggle` | Resume, pause, or switch between the two |
| `sonora play <file\|link>...` | Play now; the rest of the queue carries on after |
| `sonora play -s <query> [-t TYPE]` | Find the best hit of a search and play it now |
| `sonora next` / `prev` | Skip to the next track, or go back to the previous one |
| `sonora seek <0-100\|TIME\|±STEP>` | Jump to a percent of the current track, a time such as `1:30`, or a step such as `+10` |
| `sonora current` | Print what plays now as JSON, or `null` when nothing is loaded |
| `sonora watch [--position]` | Print what plays again on every change, one line of JSON each, until stopped |
| `sonora volume [N\|+N\|-N]` | Print the volume, set it from 0 to 100, or step it |
| `sonora shuffle [on\|off\|toggle]` | Print shuffle, or switch it |
| `sonora repeat [off\|all\|one]` | Print repeat, or set it |
| `sonora add <file\|link>...` | Queue after the tracks queued by hand |
| `sonora play-next <file\|link>...` | Queue right after the current track |
| `sonora add -s <query> [-t TYPE]` | Queue the best hit of a search, the one `search` lists first |
| `sonora search <query> [-t TYPE] [-n N]` | Search the current provider |
| `sonora providers` | List the providers and which one is current |
| `sonora provider [slug]` | Print the current provider, or switch to one that has an account |
| `sonora completions <shell>` | Print a completion script for bash, zsh, fish, elvish or PowerShell |
| `sonora -v` | Print the version |

`TYPE` is `song`, `album`, `artist` or `playlist`; `-s` takes a song unless told otherwise. `sonora <command> --help` describes each one.

Playback commands go to the device that has playback, so while Sonora steers another device over
Spotify Connect, that device is the one that pauses or skips.

### Queueing

`play`, `add` and `play-next` take files and links together, and play or queue them in the order
given. `play` puts them right after the current track and skips to them, as `play-next` and then
`next` would, so the queue carries on once they end:

- A relative path is taken from the folder the command runs in. A file that does not exist is
  refused before anything is sent.
- A link has to belong to the current provider: a track, an album, a playlist or an artist, whose
  top tracks are queued. A link of another provider is refused with both names, switch first with
  `sonora provider`.
- Everything is fetched before anything is queued. When one item fails, nothing is queued and the
  command says which item it was.

### Seeking

`seek` takes a bare number as a percent, as it always has, a time as `1:30` or `1:02:03`, and a
step from where the track plays as `+10`, `-10` or `-1:30`, in seconds. A step stays inside the
track, so `-10` near the start goes back to it. Bound to keys, as in Hyprland:

```
bind = SUPER, right, exec, sonora seek +10
bind = SUPER, left, exec, sonora seek -10
```

### Searching

The words of a query need no quotes, `sonora search one more time` works as it is. Quote them
when they hold characters the shell reads first, such as `'`, `?`, `&` or `!`:

```sh
sonora play -s "What's Up?"
```

`search` ranks the provider's answers against your library the way the search page does. Each row
ends in the link `add` and `play-next` take:

```
type    title          by         length  link
song    One More Time  Daft Punk  5:20    https://open.spotify.com/track/…
album   Discovery      Daft Punk          https://open.spotify.com/album/…
```

Hits without a link to act on, such as an artist no provider id names, are left out.

### Output

Text is for reading: a bare word for a single value, a table with a header for a list. `current`
is JSON either way.

`--json` prints any answer as one line of JSON for a script, without an envelope:

```sh
sonora volume --json          # 80
sonora providers --json       # [{"slug":"spotify","stored":true,"current":true},…]
sonora search daft punk --json | jq -r '.[0].link'
```

`current` answers:

```json
{
  "state": "playing",
  "track": {
    "id": "…",
    "title": "One More Time",
    "artists": ["Daft Punk"],
    "album": "Discovery",
    "cover": "https://…",
    "duration": 320
  },
  "position": 75,
  "volume": 80,
  "shuffle": false,
  "repeat": "off",
  "provider": "spotify",
  "liked": true,
  "origin": { "kind": "album", "id": "…", "name": "Discovery" }
}
```

`state` is `playing`, `paused`, `loading` or `idle`. Times are whole seconds, the volume a percent.
`provider` is the slug `sonora providers` lists, `local` for a file. `origin` is what the track was
queued from, `null` when it was queued on its own; its `kind` is `album`, `playlist`, `artist`,
`radio`, `liked` or `local`.

### Watching

`watch` prints what plays as `current` does, then again on every change of the track, the state,
the volume, shuffle or repeat, one line of JSON each, for a status bar to follow without asking
every second. `--position` also prints a line every second while a track plays, for a clock.

```sh
sonora watch | jq --unbuffered -r '.track.title // ""'
sonora watch --position > nowplaying.json    # read by a stream overlay
```

A line is `null` while nothing is loaded. It runs until stopped, and ends with code 0 when Sonora
quits. With nothing reading its lines any more, as after `| head -1`, it ends at its next line, as
`tail -f` does, so within a second with `--position` and at the next change without.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Done |
| 1 | Sonora refused the command, or did not answer |
| 2 | The command line is wrong; nothing was sent |
| 3 | Sonora is not running |

A command never starts Sonora: with none running it exits with 3.

### Completions

```sh
sonora completions bash > ~/.local/share/bash-completion/completions/sonora
sonora completions zsh > "${fpath[1]}/_sonora"
sonora completions fish > ~/.config/fish/completions/sonora.fish
```

## How it works

A command starts a second, short-lived `sonora` process. It reads the command line, sends the
command to the running Sonora over the instance socket, prints the answer and exits. It never
opens a window, a log file or a runtime.

```
sonora next                                     running Sonora
───────────                                     ──────────────
cli::parse  → CommandType::Remote               single::serve reads one line
single::connect, wire::exchange   ── request ─▶ state::Control::answer
                                                  → Playback, Queue, Session, Library
print, exit with its code         ◀── reply ───
```

The socket is the one that keeps Sonora to a single instance. It lives where only your account
can reach it:

| Platform | Socket |
|---|---|
| Linux | `$XDG_RUNTIME_DIR/sonora.sock`, or `$XDG_RUNTIME_DIR/app/<id>/` in Flatpak |
| macOS | the per-user temporary folder |
| Windows | the named pipe `sonora.sock`, open to your account only |

A debug build uses `sonora-dev.sock`, so it never talks to an installed Sonora.

A Flatpak Sonora is reached from inside its sandbox, so run commands through it:
`flatpak run <id> next`.

### Protocol

One line of JSON each way: the request, then the reply. A watch is the one exception: its reply
comes again on every change, on the same connection, until the command line hangs up.

```
→ {"v":1,"body":{"playback":"next"}}
→ {"v":1,"body":{"playback":{"volume":{"by":5}}}}
→ {"v":1,"body":{"queue":{"add":[{"search":{"text":"one more time","kind":"song"}}]}}}
→ {"v":1,"body":{"watch":{"position":false}}}
← {"v":1,"body":"done"}
← {"v":1,"body":{"value":{"volume":80}}}
← {"v":1,"body":{"failed":{"no-account":"Deezer"}}}
```

`v` is `wire::VERSION`. A line of another version is refused before its body is read, so an older
or newer Sonora never misreads a command. Bump it on any change an older Sonora would misread.
The types of `body` are `commands::Request` and `wire::Reply`; they are the contract, so a field
is added, never renamed or removed, without a bump.

A line that does not start with `{` is the older hand-off of files and links, one per line, which
a file manager's launch still uses.

## Layout

| File | What it holds |
|---|---|
| `args.rs` | The clap command line, kept apart from the protocol types and turned into a `CommandType` |
| `commands.rs` | `CommandType`, and the `Request` sent over the socket |
| `wire.rs` | The envelope, `Reply`, the answer types and `Failure` |
| `run.rs` | Sending a request, printing the answer, exit codes |
| `item.rs` | Telling paths from links, and `file://` URIs |

Outside this crate:

- `crates/state/src/control.rs` carries a `Request` out against the app's state.
- `crates/sonora/src/single.rs` is the socket, `crates/sonora/src/console.rs` the Windows
  console, and `main.rs` calls `cli::parse` before anything else starts.

The crate knows nothing of GPUI or of the app's state, so it can be built and read on its own.

### Adding a command

1. Add the variant to its group in `commands.rs`, with a `///` naming the command it stands for.
2. Add the subcommand to `Sub` in `args.rs` and map it in `into_command`.
3. Carry it out in `state::Control`. Go through `Playback` for anything that plays, read the
   `shown_*` values, and answer only once the work is done.
4. If it answers with something new, add a `wire::Value` and print it in `run::show` and
   `run::json`.
5. List it in the table above.

## Backlog

- **Messages through Fluent.** Errors, table headers, statuses and `--help` are in English in
  the code for now. They move to `assets/i18n/en-US/main.ftl`, in the language the running
  Sonora uses or, for `--help` with none running, the system's.
