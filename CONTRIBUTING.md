# Contributing to Sonora

Sonora is a native music streaming client, built with Rust and
[GPUI](https://github.com/zed-industries/zed), streaming through
[librespot](https://github.com/librespot-org/librespot). Linux is the platform we mainly
develop against. Windows and macOS are built by CI but usually not exercised locally except for
platform-specific features.

## Setup

With Nix, everything you need is in the dev shell:

```sh
nix develop          # or: direnv allow
cargo run --locked --package sonora
```

Without Nix, install the build and runtime dependencies yourself. The authoritative list is
`runtimeLibraries` plus `nativeBuildInputs` in `flake.nix`; these commands are that list translated.

Arch and derivatives:

```sh
sudo pacman -S --needed base-devel pkgconf mold \
  alsa-lib dbus fontconfig freetype2 sqlite \
  libx11 libxcb libxcursor libxi libxkbcommon libxkbcommon-x11 wayland \
  vulkan-icd-loader
```

Debian and Ubuntu:

```sh
sudo apt install build-essential pkg-config mold \
  libasound2-dev libdbus-1-dev libfontconfig-dev libfreetype-dev libsqlite3-dev \
  libx11-dev libxcb1-dev libxcursor-dev libxi-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libvulkan-dev mesa-vulkan-drivers
```

Fedora:

```sh
sudo dnf install @development-tools pkgconf-pkg-config mold \
  alsa-lib-devel dbus-devel fontconfig-devel freetype-devel sqlite-devel \
  libX11-devel libxcb-devel libXcursor-devel libXi-devel \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel \
  vulkan-loader-devel mesa-vulkan-drivers
```

Three things that bite people:

- **A Vulkan driver is a runtime requirement**, not just a build one — the GPUI renderer is
  Vulkan-based, so the loader alone is not enough. `mesa-vulkan-drivers` covers AMD and Intel on
  Debian and Fedora; on Arch install `vulkan-radeon` or `vulkan-intel`. NVIDIA users need the
  proprietary driver (`nvidia-utils` on Arch).
- **mold must be on `PATH`** — `.cargo/config.toml` passes `-fuse-ld=mold` for
  `x86_64-unknown-linux-gnu`. Build with `RUSTFLAGS="" cargo build …` to drop it.
- **Rust must be 1.85 or newer** — the workspace is edition 2024 with resolver 3, and there is no
  `rust-toolchain.toml`, so whatever is on `PATH` builds it. Arch and Fedora ship current toolchains;
  Debian 13 sits exactly on 1.85.0, and Debian 12 (1.63) and Ubuntu 24.04 are too old — use
  [rustup](https://rustup.rs) there.

The first build compiles GPUI from source, so expect several minutes.

Debug builds claim `sonora-dev.sock`, so `cargo run` starts beside an installed Sonora instead of
handing over to it. Logs go to `$XDG_STATE_HOME/sonora/sonora.log`; widen them with
`SONORA_LOG=sonora=debug,music=debug`. Durable settings live in
`$XDG_CONFIG_HOME/sonora/settings.json`; runtime state lives in
`$XDG_DATA_HOME/sonora/state.sqlite`.

## Checks

Run these before you open a pull request:

```sh
cargo fmt
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

Clippy is clean, so anything it reports is yours.

## Branches and commits

Work is pull-requested into `dev`. `main` moves only for a release, major pre-release changes.

Commits follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope):
description`, imperative, lowercase, no trailing period, and **no body**. Scopes seen in the log:
`views`, `ui`, `music`, `state`, `playback`, `player`, `settings`, `router`, `local`, `sonora`, `nix`.

```
feat(views): add a followed artists page to your library
fix(views): collapse sidebar sections from the current route
refactor(state): add toggle_origin for origin playback
chore(nix): point the flake at 0.10.0
```

Keep each commit buildable on its own.

## Pull requests

Ideally, follow this template, but if you believe that your changes require a different format,
feel free to deviate.

```markdown
## Summary

- add a followed artists page to your library, with circular cards and the usual toolbar tools
- follow and unfollow an artist from the artist page
- rename the local library tab to imported
```

## House rules

These are the ones reviews catch most often.

**Never call the Spotify Web API.** All data comes from librespot's `spclient`. Don't add `reqwest`
calls to `api.spotify.com`, a client secret, or `rspotify`. New endpoints go on the `MusicApi` trait
in `crates/music/src/lib.rs` and are implemented in a focused module under `music/src/spotify/`. Although most
Spotify functionality is probably already implemented by this point anyway.

**A component probably already exists.** Grep `crates/ui/src/lib.rs`, `crates/views/src/shared/cells.rs`
and `crates/views/src/chrome/` first. Extend what's there — add a builder method to `Button` rather
than writing an `IconButton`.

**Never hardcode a color, radius, or size.** Everything comes from the theme, and metrics scale with
the user's font size:

```rust
let theme = *cx.theme();
theme.foreground              // not rgb(0xffffff)
theme.metrics.row             // not px(44.)
theme.text(Text::Small)
```

**Never render a bare English literal.** Add the key to `assets/i18n/en-US/main.ftl` and translate it
in as many languages as you can. Resolve keys in `render`, never in a constructor, or the string
freezes in whatever language was active at construction:

```rust
t!("artist-follow")
t!("song-disc-track", disc = disc, track = number)
```

Developer-facing text — `.context("cannot …")`, `log::warn!`, wire values — stays in English.

**Register new assets.** SVGs go in `assets/icons/` and their stem goes in the `ICONS` list in
`crates/sonora/src/assets.rs`, otherwise loading logs `assets: … is not registered` and renders
nothing.

**One breakpoint ladder.** Use `ui::Room` (`Tight | Snug | Roomy | Wide | Vast`), and measure against
`Chrome::content`, not the raw viewport width:

```rust
if Chrome::room(window, cx).fits(Room::Roomy) { … }
```

**Network work runs on the tokio runtime, never on GPUI's executor.** Anything touching `MusicApi`,
librespot or sockets goes inside `io.spawn`, mutations happen in `this.update`, and the returned
`Task` is stored in a field — dropping it is how sign-out and navigation cancel in-flight loads.
Never `.detach()` a data load. New network-backed features belong in a `state` entity, not a view.

**Comments: essentially none.** Name things so they don't need one. If a comment is unavoidable it should stay concise, no trailing period.

## Translations

English is the source of truth and must carry every key. Other locales may lag: a missing key falls
back to English at runtime, so a partial translation is a fine contribution — don't machine-translate
a language you don't speak just to fill the table. The tests only check that English is complete and
that no locale invents a key.

The coverage table in [README.md](README.md#translations) shows where each locale stands; strings
live in `assets/i18n/<locale>/main.ftl`.

```sh
$EDITOR assets/i18n/uk/main.ftl        # fill in what it lacks
scripts/i18n-coverage.py              # refresh the table in README.md
cargo test -p i18n
```

A new language needs `assets/i18n/<locale>/main.ftl` plus a `Language` variant in
`crates/i18n/src/language.rs` (`id`, `label`, `tag`, `source`, `ALL`). Plural rules come from Fluent
selectors, so check `count-songs` against your language's categories rather than concatenating
strings.

## Generated files

Don't hand-edit these, re-run the generator:

- `THIRD-PARTY.md` — `scripts/generate-notices.py` (a new dependency means re-running it)
- the translation table in `README.md` — `scripts/i18n-coverage.py`
- `assets/linux/`, `assets/macos/sonora.icns`, `assets/windows/sonora.ico` —
  `scripts/generate-icons.py` from the master `assets/icon.svg`
- `cargoHash` in `flake.nix` goes stale when `Cargo.lock` changes: build once, take the `got:` hash
  from the failure, paste it in

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add user-facing
sentences to `## [Unreleased]` as features land, under `Added` / `Changed` / `Fixed` — say what
someone using Sonora can now do, and leave out work no user can observe. Cutting a release is then
only a rename.

## License

Sonora is GPL-3.0-or-later. By contributing you agree that your contribution is licensed under those
terms.
