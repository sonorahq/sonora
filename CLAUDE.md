# sonora

A native music streaming client, built with Rust and [GPUI](https://github.com/zed-industries/zed)
(Zed's UI framework), streaming through [librespot](https://github.com/librespot-org/librespot) and [ytmusic-rs](https://github.com/noligh132/ytmusic-rs). Cargo workspace, edition 2024, resolver 3.

## Read this before writing code

1. **A component probably already exists.** Check `crates/ui/src/lib.rs`, `crates/views/src/shared/cells.rs`
   and the tables below before writing a new element. See [Before you build a component](#before-you-build-a-component).
2. **Never call the Spotify Web API.** There is no `reqwest` call to `api.spotify.com` anywhere and
   there must not be one. All data comes from librespot's `spclient`. See [Backend](#backend-how-data-actually-arrives).
3. **Never hardcode a color, radius, or size.** Everything comes from `cx.theme()`.
   See [Theme and metrics](#theme-and-metrics).
4. **Network work runs on the tokio runtime (`Io`), never on GPUI's executor.**
   See [Async: two runtimes](#async-two-runtimes).
5. **Assets are picked up from their folder, never from a list.** Drop an SVG into an
   `assets/icons/<pack>/` folder or a face into `assets/fonts/`; the build scripts walk the
   directory. See [Icons](#icons).
6. **Never push changes without the user's explicit confirmation.** Committing does not imply permission
   to run `git push`; ask immediately before every push.

## Crate layout

```
crates/
  sonora/     binary: main, window, actions, asset registry, HTTP client shim, tray, dock
  views/      screens plus app chrome: title bar, sidebar, player bar, filter/search field
  state/      GPUI entities holding app state; owns all async orchestration
  music/      provider traits + models; spotify/ = librespot data access and playback (no GPUI)
  storage/    canonical state.sqlite path, schema, and one-release legacy migration
  ui/         design system: theme, metrics, and reusable elements (gpui only)
  router/     Destination enum, navigation history, Link trait
  input/      text input element + global actions and keybindings
  i18n/       Fluent localization: the `t!` macro, locale selection, embedded .ftl
  icons/      the icon packs: registry, active pack, path resolution, AssetSource
  embed/      build-script helper that walks a folder and writes include_bytes! literals
```

Dependency direction is strict; do not create a back edge:

```
sonora → views → state → music
         state, music → storage
         all ui-side crates → ui, router, input → ui → gpui
         every ui-side crate → i18n, icons → gpui
```

- `music` holds the provider abstraction (`MusicApi`, `MusicProvider`, `Player`, `PlaybackFactory`)
  and the models in its root; each provider lives in a submodule (`music::spotify`,
  `music::youtube`, `music::local`). `state` and `views` see only the root traits and models — never
  a provider module. Only `sonora/src/main.rs` names a concrete provider.
- `ui` depends only on `gpui`, `serde` and `i18n`. It must never know about `music`, `state`, or
  playback.
- `music` must never depend on `gpui`. It is plain async Rust.
- `storage` is a leaf shared by `state` and `music`; it owns the only state database path, schema,
  connection setup and legacy database migration.
- `state` depends on `ui` only for `ThemeOverrides`, `MIN_FONT`, `MAX_FONT` (settings persistence).
- Widgets that need app state (player bar, sidebar) live in `views/src/chrome/`, not `ui`.
- `i18n` is a leaf: it depends on `fluent-bundle`, `unic-langid`, `sys-locale` and `gpui` (for
  `SharedString`) and on nothing else in the workspace.
- `icons` is a leaf too: `gpui`, `anyhow`, `log`, plus `embed` at build time. It never depends on
  `ui`, so `ui` and `views` can both reach it.
- `embed` is a build-support crate. Nothing links it at runtime; it is a `[build-dependencies]`
  entry of `icons` and `sonora` only.

## Building

### Any platform: what the build needs

The GPUI renderer is Vulkan-based, so a Vulkan ICD is a **runtime** requirement, not just a build
one. Link-time deps: `vulkan-loader`, `wayland`, `libxkbcommon`, `libxcb`, `libx11`, `libxcursor`,
`libxi`, `fontconfig`, `freetype`, `alsa-lib`, `dbus`, `sqlite`, plus `pkg-config`.

`.cargo/config.toml` passes `-fuse-ld=mold` for `x86_64-unknown-linux-gnu`, so **mold must be on
PATH** for that target. If it isn't, either install mold or build with
`RUSTFLAGS="" cargo build …` to drop the flag.

### Nix (primary)

```sh
nix develop          # or: direnv allow  (.envrc runs `use flake`)
cargo run --locked --package sonora
nix run              # run the released binary
nix build            # ./result/bin/sonora, fetched not compiled
```

The flake packages the **released** binary and never builds from source: `default`, `sonora` and
`sonora-bin` all resolve to the asset named by `release.version`. So it cannot build the working
tree — use `cargo` for that — and a local change is invisible to `nix build` until it is tagged.
There is no `cargoHash` to keep in step with `Cargo.lock` any more.

The devShell supplies `rustc`, `rustfmt`, `rust-analyzer`, `mold`, `pkg-config`, `sccache` and the
runtime libs via `LD_LIBRARY_PATH`. It does **not** ship `cargo` or `cargo-clippy` — those come from
the ambient system profile here. If `cargo` is missing inside the shell, that's why.

Nothing in `flake.nix` tracks `Cargo.lock`, so a lockfile change never makes the flake stale. The
per-target `hash` values follow the release assets instead, and only move when a version is cut.

### Arch / CachyOS

```sh
sudo pacman -S --needed base-devel rust pkgconf alsa-lib dbus fontconfig freetype2 sqlite \
  libx11 libxcb libxcursor libxi libxkbcommon libxkbcommon-x11 wayland \
  vulkan-icd-loader mold
# plus a Vulkan driver: vulkan-radeon | vulkan-intel | nvidia-utils
# plus the ALSA bridge for your sound server: pipewire-alsa | pulseaudio-alsa

cargo run --locked --package sonora
cargo build --release --locked --package sonora && ./target/release/sonora
```

### Debian/Ubuntu and Fedora

Not exercised in this repo; package sets translated from the dependency list above.

```sh
# Debian/Ubuntu
sudo apt install build-essential pkg-config mold libasound2-dev libfontconfig1-dev \
  libfreetype-dev libsqlite3-dev libx11-dev libxcb1-dev libxcursor-dev libxi-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libdbus-1-dev \
  mesa-vulkan-drivers

# Fedora
sudo dnf install @development-tools pkgconf-pkg-config mold alsa-lib-devel fontconfig-devel \
  freetype-devel sqlite-devel libX11-devel libxcb-devel libXcursor-devel libXi-devel \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel dbus-devel \
  mesa-vulkan-drivers
```

### Flatpak

`flatpak/flatpak-builder.yaml` builds against the freedesktop 25.08 runtime with the `rust-stable`
extension, which also supplies the mold that `.cargo/config.toml` asks for. `flatpak/generate-sources.sh`
turns `Cargo.lock` into `cargo-sources.json` (generated, never committed) and `flatpak/build-flatpak.sh`
runs the build locally. The release workflow builds both arches in Flathub's builder image, imports them
into the signed OSTree repo on the `flatpak-repo` branch, which GitHub Pages serves at
`https://nolight132.github.io/sonora`, and only then attaches `.flatpak` bundles to the release:
`flatpak/export-bundles.sh` re-exports them from that repo so each carries the commit signature, the
repo URL and the public key, and `flatpak update` follows the repo afterwards. `flatpak-bundles.yml`
reruns that export for an existing release and swaps its bundles and checksum lines.
`flatpak/pages/` holds the `.flatpakref` and `.flatpakrepo` that point there, with the public half
of the `FLATPAK_GPG_KEY` secret embedded — a new key means regenerating both. Flathub is not an
option: its requirements forbid AI-assisted code.

### macOS / Windows

Released, but not developed against here. `.github/workflows/release.yml` builds both Apple targets
and `x86_64-pc-windows-msvc`; the `x11`/`wayland` features on `gpui_platform` are inert off Linux,
so the pin does no harm. There is no local toolchain for either — the release workflow is the only
thing that exercises them, and it only runs on a tag. The flake declares `x86_64-linux`/`aarch64-linux`
only, and the mold linker flag targets Linux.

macOS ships as a universal `Sonora.app` inside a disk image: `lipo` merges the two arch builds,
`codesign --force --sign -` signs it ad-hoc, `hdiutil` wraps it. Ad-hoc signing only makes the
binary runnable on Apple Silicon — it does not satisfy Gatekeeper, so an unnotarized build still
needs `xattr -dr com.apple.quarantine` on first launch. Do not add `--deep`; Apple deprecates it for
signing and the bundle has no nested code.

Windows embeds `assets/windows/sonora.ico` through `crates/sonora/build.rs` and `winresource`. It is
also the one target that compiles SQLite instead of linking the system one: `crates/sonora/Cargo.toml`
turns on `state/bundled-sqlite` under `cfg(windows)`, because MSVC has no `libsqlite3` to find.

### Checks

```sh
cargo fmt                      # rustfmt.toml: edition 2024, style_edition 2024
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

The first build compiles GPUI from source; expect several minutes.

Clippy is **clean** — `--all-targets` included — and stays that way. Boxed callback fields get a
module-local `type` alias (`Click`, `Change`, `Slide`, `Release` in `ui`, `cells::Tap` in `views`)
rather than a `type_complexity` warning. The one lint that cannot be satisfied on its merits is
`reversed_empty_ranges` on the deliberate `clamp_range("abc", &(2..1))` case in
`crates/ui/src/input/mod.rs`; it carries a targeted `#[allow]` on that test. Don't rewrite the case
to please the lint.

### Profiling

`gpui` carries a `profiler` feature, exposed as sonora's own `profiler` feature so normal builds
pay nothing:

```sh
cargo prof                             # alias for run --release --features profiler
cargo run --features profiler          # then set any of:
GPUI_DEBUG_OVERLAY=minimal|full        # frame time, phase split, cache hits, dirty count
GPUI_SHOW_REPAINTS=1                   # wash every view a notify named, fading over 160ms
GPUI_LOG_NOTIFIES=1                    # once a second, which views were notified and how often
ZED_MEASUREMENTS=1                     # raw frame durations on stderr
```

`full` paints its readout as quads and costs a few ms itself, so time with it hidden. The wash marks
the view a notify named, not the ancestors dragged along with it — dirtiness always walks up the view
path, so a leaf can never repaint without its ancestors' render functions running. There is no
damage tracking in the renderer: the whole window is redrawn every frame, so caching saves element
construction, layout and scene assembly, never GPU fill.

### Runtime environment

|                   |                                                                                                                                   |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Settings          | `$XDG_CONFIG_HOME/sonora/settings.json` (durable preferences and local music folder)                                              |
| App state         | `$XDG_DATA_HOME/sonora/state.sqlite` (window/layout/playback state, pins, history, local playlists, usage flags)                  |
| Credentials cache | `$XDG_CACHE_HOME/sonora/<provider>/credentials.json`, one per provider slug (`spotify`, `youtube`), owner-only mode                |
| OAuth redirect    | `http://127.0.0.1:8989/login`, override with `SONORA_REDIRECT_URI`                                                                |
| Instance socket   | `sonora.sock`, `sonora-dev.sock` in debug builds, so `cargo run` starts beside an installed Sonora rather than handing over to it |
| Log file          | `$XDG_STATE_HOME/sonora/sonora.log`, rotated to `.1` past 8 MiB                                                                   |
| Console logging   | `RUST_LOG`; default filter `warn,symphonia=error,lofty=error`                                                                     |
| File logging      | `SONORA_LOG`; default adds `sonora=debug,ui=debug`                                                                                |

Startup runs one migration pass before constructing app state. It moves volatile values out of a
pre-v2 `settings.json`, imports `history.sqlite3`, `flags.sqlite3` and
`local-playlists.sqlite3` into `state.sqlite`, and moves `local-music.json`'s folder into
`settings.json`. `music::credentials::migrate` runs in the same pass: it rewrites the Spotify
`credentials.json` from the cache root into `spotify/` and folds the YouTube `cookies.txt`,
`authuser.txt` and `guest` files into `youtube/credentials.json`, each owner-only, so the providers
only ever read the new paths. A legacy file is removed only after its replacement has been written
successfully.
This compatibility code is intentionally temporary and can be removed after the next release.

## Before you build a component

Grep first. In order: `crates/ui/src/lib.rs` (exports), `crates/views/src/shared/cells.rs` (grid cell
renderers), `crates/views/src/chrome/` (chrome). Extend what's there — add a builder method to
`Button` rather than writing `IconButton`.

### `ui` — reusable elements

| Item                                                                        | Use for                                                                                                                                         |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `Button`                                                                    | every button. Variants `.ghost() .outline() .primary() .danger()`, plus `.small() .icon() .trailing() .label() .tint() .selected() .disabled()` |
| `Card`                                                                      | artwork + title + eyebrow + meta row/tile. `.art()` for tile mode, `.circle()`, `.loading()`, `.trailing()`, `.explicit()`, `.press()`          |
| `Artwork`                                                                   | cover images with skeleton loading and a music-note fallback                                                                                    |
| `Skeleton`, `Initials`                                                      | pulsing loading placeholder; avatar initials                                                                                                    |
| `Table`, `TableSource`, `TableDelegate`, `TableState`, `ColumnSpec`, `Cell` | every table. Virtualized, sortable, filterable, hideable columns, columns dropped by `rank` when the room runs out                              |
| `Scroller` + `Scrollbar`                                                    | any scrolling region. Do not use bare `overflow_y_scroll`                                                                                       |
| `Scrubber` + `ScrubberState`                                                | any draggable 0..1 track (seek bar, volume)                                                                                                     |
| `Panel` + `Side`                                                            | a resizable side panel shell: clamped width, drag grip, pixel snapping. `.limits()`, `.fill()`, `.on_resize()`                                  |
| `Menu`, `MenuItem`                                                          | dropdowns (deferred + occluded, with `on_dismiss`)                                                                                              |
| `InlineLinks`, `InlineLink`                                                 | comma-joined clickable artist lists                                                                                                             |
| `eyebrow()`, `heading()`                                                    | the two standard text styles                                                                                                                    |
| `ExplicitBadge`                                                             | the "E" badge                                                                                                                                   |
| `WindowControls`                                                            | minimize/maximize/close, honoring platform decorations                                                                                          |
| `Rising` / `veiled()`                                                       | the one entrance: fade, 1% zoom and a 1.5px blur. `.rising(id)` for anything that appears; `veiled` for a hand-driven progress                  |
| `clock()`                                                                   | `Duration` → `m:ss`                                                                                                                             |
| `snapped()`                                                                 | round a `Pixels` to the device pixel grid                                                                                                       |

Also available: `Input` (`input` crate — full text editing, IME, selection, clipboard) and
`Link` (`router` — makes a `Stateful<Div>` navigate on click).

### `views/src/shared/cells.rs` — grid cell renderers

`index` (play/pause/now-playing transport with hover preload), `artists`, `link`, `text`, `dim`,
`title`, `artwork`, `avatar`, `blank`, `transport`, `toggle`, `artist_links`. Reuse these in any new
`TableSource::cell`.

### Element conventions

New elements follow one shape — copy `ui/src/button.rs`:

```rust
#[derive(IntoElement)]
pub struct Thing { base: Stateful<Div>, /* … */ }

impl Thing {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self { … }
    pub fn variant(mut self) -> Self { …; self }   // consuming builders
}

impl Styled for Thing { fn style(&mut self) -> &mut StyleRefinement { self.base.style() } }
impl InteractiveElement for Thing { … }

impl RenderOnce for Thing {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let overrides = std::mem::take(base.style());   // caller styles win
        let mut thing = base./* defaults */;
        thing.style().refine(&overrides);
        thing
    }
}
```

The `mem::take` / `refine` dance is deliberate: it lets a call site override any default without
the element re-applying it afterwards. Keep it.

Stateful components (`Scrollbar`, `Input`, `TableState`) are `Render` entities instead, created with
`cx.new(…)` and held by the parent.

## Theme and metrics

`ui/src/theme.rs` is the single source of every color, radius, and font size.

```rust
use ui::ActiveTheme as _;
let theme = *cx.theme();          // Theme is Copy
theme.foreground, theme.muted_foreground, theme.secondary_hover, theme.table_row_border, …
theme.radius
theme.text(Text::Small)           // Tiny Small Label Body Large Title Display
theme.metrics.row / .header / .pad / .inset / .control / .field
             .title_bar / .player_bar / .list_row / .thumb / .cover
```

- Every metric scales off the user's font size (`Metrics::new(base)`), so **never** hardcode a
  height for a row, control, or bar — read it from `theme.metrics`.
- A literal `px(…)` is acceptable only for a local, non-scaling detail, declared as a `const` at
  module top (see `NUMBER`, `DATE` in `views/src/shared/cells.rs`). Responsive thresholds are **not** a
  local detail — see [Breakpoints and content width](#breakpoints-and-content-width).
- Adding a color token means adding it to `Theme`, to every `Theme::*()` constructor, to
  `ThemeOverrides`, and to the `apply_color!` list — all four, or overrides break.
- Eight themes exist (`ThemeKind::ALL`). `ocean`/`rose`/`lavender`/`amber` are derived by mutating
  `midnight()`/`dark()`; follow that pattern rather than writing a full palette.
- Users can override any token via `settings.json`; `Theme::set` re-renders all windows.

## Icons

Icons live in `assets/icons/<pack>/`, one folder per interchangeable set:

```
assets/icons/
  common/    never follows the pack: window controls and brand marks
  lucide/    the base pack, and the fallback for every other one
  iconoir/   solar/   remix/   the alternatives
```

Every pack names its files the same way, so `heart.svg` means the same thing in all of them. A pack
is allowed to lag, the way a locale is: `icons::path` looks in the active pack, then `common`, then
`lucide`, so a set with no `guitar` quietly borrows Lucide's.

- **A screen still writes `"icons/heart.svg"`.** The pack is never part of the string a call site
  spells; `icons::path` turns it into `icons/<active pack>/heart.svg`. Resolve in the one place a
  path meets `svg()` — `svg().path(icons::path(icon))` — never in a constructor. `Button::icon`,
  `MenuItem::icon` and friends store the bare name and resolve at render, for the same reason `t!`
  is banned in constructors: the value has to follow a setting change.
- **Never register an icon by hand.** `crates/icons/build.rs` walks every folder with `embed::tree`
  and writes the registry, so dropping in a file is the whole job. `crates/embed` is the reusable
  half of that: `embed::folder(dir, kind)` and `embed::tree(dir, kind)` return the files, sorted,
  and `embed::embedded` renders the `include_bytes!` table. Sonora's own build script uses it for
  `assets/fonts`.
- `icons::packs()` is what the settings picker lists — `common` is not among them — and
  `Pack::title` is the folder id capitalised, so a new pack needs no Rust edit at all.
  `icons::SAMPLES` names the glyphs each entry previews with.
- The active pack is a process global, like `i18n`'s: `main.rs` sets it from `settings.json`
  (`appearance.icons`) at boot, and `AppSettings::set_icons` changes it and repaints every window.
  Because each pack has its own paths, GPUI's svg cache cannot serve a stale glyph.
- `scripts/fetch-icons.py` rebuilds `iconoir`, `solar` and `remix` from the Iconify API. It carries
  the canonical name → upstream name map and asks only for the icons in it; `lucide` and `common`
  are hand-kept. A name it has no equivalent for is left out on purpose — that is a fallback, not a
  bug. `panel-right-close` and `panel-right-open` are the exception: no set draws them, so the
  script mirrors each pack's own left variant through `MIRROR` rather than leaving two Lucide
  glyphs among a pack's. `SLASH` fills a missing `mic-off` the same way: it masks a band out of the
  pack's own `mic-vocal` and strokes the diagonal across the gap. Each pack keeps its licence beside its files, and `flake.nix`, the release workflow and
  `THIRD-PARTY.md` all ship the whole set.
- `cargo run -p ui --example icons` opens a gallery: every icon as a row, every pack as a column,
  with borrowed glyphs faded.

## Localization

Every user-facing string comes from Fluent. **Never render a bare English literal**; add a key to
`assets/i18n/en-US/main.ftl`, which is the source of truth, and translate it in `ru`, `uk` and `pl`
where you can.

**A locale is allowed to lag.** `lookup` falls back to English for a key the active language lacks
(and to the key itself if even English lacks it), logging `i18n: … is missing from <id>`. The tests
in `crates/i18n` enforce only that English carries every key and that no locale invents one — a
missing translation is a gap to fill, not a build failure. Machine-translating four languages to
satisfy a test was the old cost; leave the gap and let a speaker fill it. `scripts/i18n-coverage.py`
regenerates the coverage table in `README.md` between the `i18n:start` / `i18n:end` markers; re-run
it when you add keys or a language.

```rust
use i18n::t;

t!("song-credits")                                   // -> SharedString
t!("song-disc-track", disc = disc, track = number)   // named args, any number or &str
i18n::lookup(key, None)                              // when the key is a runtime &str
```

- The `.ftl` files are embedded with `include_str!` in `crates/i18n/src/language.rs`, so they do
  **not** go through `crates/sonora/src/assets.rs`.
- Keys are scoped by area: `nav-`, `column-`, `menu-`, `queue-`, `player-`, `search-`, `song-`,
  `settings-`, `theme-`, `common-`. Values stay natural case — `ui::eyebrow()` / `ui::upper()`
  do the shouting.
- Counts use Fluent selectors (`{ $count -> [one] … [few] … *[other] … }`), never string
  concatenation; ru/uk/pl need `one`/`few`/`other` to be right.
- Dates are assembled from `month-1`..`month-12` plus `date-full`, never `strftime`.
- A missing key logs `i18n: … is missing` and falls back to English, then to the key itself.
- `ColumnSpec::header` and `Slot::Header` hold a **key**, not a label; call `ColumnSpec::label()`.
- **Never call `t!` in a constructor.** Anything stored on a struct and rendered later must hold the
  key and resolve in `render`, or it freezes in whatever language was active at construction and
  never follows a language change. `Input::new`/`set_hint` and `Searchable::hint()` take keys for
  exactly this reason.
- Developer-facing text — `.context("cannot …")`, `log::warn!`, `PlaybackState::Failed` — stays
  in English. So do wire values in `music::spotify`.
- The language lives in `settings.json` (`language`, default `"auto"`). Changing it calls
  `i18n::set` then `cx.refresh_windows()`, the same way `Theme::set` repaints.

## Breakpoints and content width

Every responsive decision in the app uses one ladder, defined in `ui/src/layout.rs`. **Never write a
bare `px(…)` breakpoint** — no `width < px(640.)`, no per-view `NARROW`/`STACK_BREAKPOINT` const.

```rust
use ui::{Room, ALWAYS, SNUG, ROOMY, WIDE, VAST};   // 0 · 420 · 620 · 740 · 1180

Room::of(width)              // Tight | Snug | Roomy | Wide | Vast
Room::of(width).fits(Room::Roomy)   // ">= Roomy", the only comparison you need
```

`Room` is `Ord`, so `fits` is just `>=`. The raw `Pixels` consts exist for the places that need a
number rather than a step: centering maths, mostly.

A breakpoint is a branch — "below this width, lay the page out differently". A packing minimum is
not: `MIN_COLUMN_WIDTH` in `home/quick_picks.rs`, `PANEL` in `screens/song.rs`, the column widths in
`shared/cells.rs`. Those stay plain `px` consts at module top; forcing them onto a five-step ladder
would change how content packs.

Two modules own the measurement:

- **`ui::layout`** — the ladder itself. Pure `gpui`; knows nothing about panels.
- **`views::chrome::Chrome`** — how much horizontal room content actually has, after the left and
  right sidebars take their cut.

```rust
use crate::chrome::Chrome;
Chrome::content(window, cx)   // viewport width − both sidebars, floored at ui::MIN_CONTENT
Chrome::room(window, cx)      // the same, classified into a Room
chrome::cap(min, max, window) // a panel's ceiling: never eat ui::MIN_CONTENT
```

`Chrome::room` is how a view classifies its width: never rebuild `Room::of(…)` from a width you
derived yourself.

Rules:

- **Measure against `Chrome::content`, never `window.viewport_size().width`.** A raw viewport width
  ignores both side panels, so tables overflow under the right sidebar and grids pick a column count
  that does not fit. Raw viewport width is legitimate in exactly two places: chrome that spans the
  whole window (`PlayerBar`, `TitleBar`, and `Menu`'s submenu flip), and a side panel deciding _its
  own_ size. `Chrome::content` would be circular for a panel, because the panel is a term in it.
- **One panel never sets another's width.** Each clamps itself against the viewport through
  `chrome::cap`, so neither can squeeze content below `ui::MIN_CONTENT` on its own, and neither
  panel's size is ever a function of the other's.
- **Hiding is a different question from sizing, and it does look at both panels.** `SidebarLeft`
  auto-hides once the room left for content — viewport minus its own width _and_
  `Chrome::sidebar_right` — drops below `Room::Wide`, so opening the queue pushes it out of the way
  without resizing it. It reads last frame's publish, which cannot loop: the decision changes
  visibility, never width. `SidebarRight` decides its own takeover from the viewport alone.
  Toggling a hidden `SidebarLeft` back on while the window is that narrow overlays it at the width
  the user last chose, so its drag ceiling then comes from the viewport alone — an overlay takes no
  content space, so capping it against content room would only disable resizing.
- **A panel's width changes only when the user drags it.** `chrome::cap` feeds `Panel::reach`, which
  bounds the _drag_, not the render — narrowing the window must never silently shrink a panel the
  user sized. `Panel::limits` stays the static floor and ceiling.
- `Workspace::render` publishes the widths every frame via `Chrome::publish`; it notifies only when
  a width actually changed, so it cannot loop.
- **`Chrome` is only current after that publish.** `Workspace` renders _before_ it and owns both
  panels, so it reads them directly; everything it renders into — content views and both panels —
  sees this frame's values.
- **A view whose layout depends on width must observe the chrome**, or it will not repaint when a
  panel is resized or toggled:
  ```rust
  let chrome = Chrome::entity(cx);
  cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
  ```
  Nothing outside `Workspace` holds an `Entity<SidebarLeft>` — that is what `Chrome` replaced.
- `views::cells::content_width(window, inset, cx)` is the shortcut for grid pages: `Chrome::content`
  minus the page's own padding.

Both side panels are built on `ui::Panel` (`Side::Left` / `Side::Right`), which owns the width
clamping, the drag grip and the snap to the device pixel grid; each reports its new width through
`on_resize` and persists it in `state.sqlite` (`sidebar_width`, `sidebar_right_width`).

## Async: two runtimes

GPUI has its own executor; librespot and `reqwest` need tokio. `state::Io` wraps a multi-thread
tokio `Runtime` and is a GPUI global.

```rust
let io = Io::global(cx);
self.task = Some(cx.spawn(async move |this, cx| {      // GPUI executor
    let loaded = join(io.spawn(async move {            // tokio: all network work
        client.album_tracks(&id).await
    })).await;
    this.update(cx, |this, cx| { /* apply, cx.notify() */ }).ok();
}));
```

Rules:

- Anything touching `MusicApi`, librespot, or sockets goes inside `io.spawn`.
- Only mutate entity state inside `this.update`, and end with `cx.notify()`.
- Store the returned `Task` in a field (`task`, `load`, `fetch`) — dropping it cancels the work,
  which is how sign-out and navigation cancel in-flight loads. Never `.detach()` a data load.
- `join(handle)` (crate-private in `state`) flattens `JoinHandle<Result<T>>`.
- `cx.subscribe(…)` / `cx.observe(…)` **are** `.detach()`-ed, in the constructor.
- Because `join` and `Io` plumbing live in `state`, new network-backed features belong in a `state`
  entity — not in a view.

## Backend: how data actually arrives

### There is no Spotify Web API here

`music::spotify` (`crates/music/src/spotify/`) talks to Spotify through
`librespot_core::Session::spclient()` — the same internal endpoints the official client uses,
returning protobuf or JSON. Consequences:

- Do not add `reqwest` calls to `api.spotify.com`, do not add a client secret, do not add
  `rspotify`. The only `reqwest` in the tree is `crates/sonora/src/http.rs`, a `gpui::HttpClient`
  adapter that exists purely so GPUI can fetch cover images.
- Auth is OAuth PKCE via `librespot-oauth` against **Spotify's own client id**
  (`DEFAULT_CLIENT_ID` in `auth.rs`). A developer-app client id will be refused at session connect —
  `auth::denied` documents this. Don't "fix" auth by swapping in a registered app id.

### Module map

`crates/music/src/lib.rs` holds the provider traits (`MusicApi`, `MusicProvider`, `Player`,
`PlaybackFactory`, `PlaybackEvents`) and `models.rs` the shared models (`Track`, `Album`,
`AlbumDetail`, `Artist`, `ArtistRef`, `SavedArtist`, `Playlist`, `UserProfile`, …). Under `src/spotify/`:

| Module                         | Endpoint / mechanism                                                            |
| ------------------------------ | ------------------------------------------------------------------------------- |
| `mod.rs`                       | `SpotifyProvider`: implements `MusicProvider`, wires client + playback factory  |
| `auth.rs`                      | OAuth login, credential cache, `restore` / `login` / `forget`                   |
| `client.rs`                    | `LibrespotClient`, the `MusicApi` implementation                                |
| `collection.rs`                | saved tracks; `metadata()` — batched `get_extended_metadata` (TRACK_V4)         |
| `collection2.rs`               | `/collection/v2/paging` — hand-rolled protobuf, paged, honors tombstones        |
| `albums.rs`, `artists.rs`      | extended metadata for albums/artists, artist portraits                          |
| `pathfinder.rs`, `pathfinder/` | GraphQL `api-partner…/pathfinder/v2/query`: album, artist overview, playcounts  |
| `playlists.rs`                 | `get_playlist` → `SelectedListContent` → uri list → `collection::metadata`      |
| `search.rs`                    | `get_context("spotify:search:…")` → track uris → `collection::metadata`         |
| `radio.rs`                     | `get_radio_for_track` → a generated playlist id → `playlist_tracks`             |
| `profiles.rs`                  | display-name lookups, fanned out over a `JoinSet`                               |
| `wire.rs`                      | protobuf → `models` conversion, `image_url` (file id → `i.scdn.co/image/<hex>`) |
| `pb.rs`                        | minimal protobuf `Reader`/`Writer` for endpoints with no generated schema       |
| `playback.rs`, `sink.rs`       | librespot playback engine + custom rodio sink (see [Audio](#audio))             |

Working notes:

- Prefer generated messages from `librespot-protocol`. `pb.rs` exists only because the collection-v2
  paging schema isn't in that crate — don't hand-roll a new one if a generated type exists.
- The recurring pattern is **uris → `collection::metadata` → `Track`**. A new track-listing endpoint
  should resolve uris and reuse `metadata`, not re-parse track fields.
- `Track::id` is `Option<String>` base62 (no `spotify:track:` prefix); the prefix is added at the
  audio boundary. `Track::playable` is false for unavailable tracks — check it before playing.
- New endpoints: add the method to the `MusicApi` trait in `crates/music/src/lib.rs`, implement on
  `LibrespotClient` by delegating to a focused module. The trait exists so `state` depends on an
  interface, not on librespot directly — a second provider (`music::youtube`) implements the same
  trait.
- **The collection has more than one set.** `collection2::saved_items`/`set_saved` take the set name:
  `COLLECTION` holds saved tracks and albums, `ARTISTS` ("artist") holds followed artists. Reading the
  artist set is confirmed against the live API; if it ever comes back empty, `artists::followed`
  falls back to scanning `COLLECTION` for `spotify:artist:` uris. The follow _write_ path shares the
  same set name but has not been exercised against a live account — a failure surfaces as
  `library: cannot update the followed artist`.
- Errors use `anyhow` with `.context("cannot …")` in lowercase.

### Audio

`music::spotify::playback` owns `librespot_playback::Player` plus `BlazingSink` (`sink.rs`), a
custom rodio sink with smooth gain ramping and flush-on-seek. `Factory` implements
`music::PlaybackFactory`; `Engine` implements `music::Player`; events arrive as
`music::PlaybackEvent` (`Loading/Playing/Paused/Position/Ended/Unavailable`).

Never drive a player from a view. Go through `state::Playback`, which owns the engine, pumps
events into `PlaybackState`, and handles shuffle, repeat, skip debouncing, and the cooldown after
an `Unavailable` track.

### State entities

`state::init` installs a `Sonora` global holding `session`, `library`, `playback`, `queue`,
`settings`. Reach them with `Sonora::global(cx)`.

| Entity                                     | Responsibility                                                                                                                                                             |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Session`                                  | auth lifecycle; emits `SessionEvent::{SignedIn, SignedOut}`; hands out `Arc<dyn MusicApi>` and `Arc<dyn PlaybackFactory>`                                                  |
| `Library`                                  | saved tracks / playlists / albums / followed artists; `LibraryState` is `Empty \| Loading \| Ready{..,problems} \| Failed` — partial failure is normal, surface `problems` |
| `Playback`                                 | engine ownership, transport, shuffle/repeat, volume, `Origin` tracking, `toggle_origin`                                                                                    |
| `Queue`                                    | past / current / upcoming; `start`, `next`, `next_random`, `previous`, `rewind`                                                                                            |
| `Home`, `Detail`, `ArtistDetail`, `Search` | per-screen loaders, each owning its `Task`                                                                                                                                 |
| `AppSettings`                              | debounced durable settings in JSON and runtime state in SQLite                                                                                                             |

Everything reacts to `SessionEvent`: signing out must clear derived state. If you add an entity that
caches Spotify data, subscribe to `Session` and clear on `SignedOut`.

## UI patterns

**Navigation.** `router::Destination` is the route enum; `router::navigate(dest, cx)`, `back`,
`forward`. `Root` subscribes to `NavigationEvent::Moved` and swaps the workspace content. For a
clickable region, use the `Link` trait (`div().id(..).link(Destination::Album(id))`) instead of a
manual click handler.

`router::Screen` is the separate, user-pickable list of launch screens (home, search and every
library tab); it maps to a `Destination` through `Screen::destination` and is stored by id in
`settings.json` as `startup`. `sonora/src/main.rs` resolves it at boot, and a link on the command
line still wins over it.

**Sidebar sections only ever expand on their own.** `SidebarLeft` opens Your Library or Settings
whenever the route enters one (`expanded(&Destination)` in `sidebar_left.rs`), but nothing collapses
a group except the chevron — leaving through a card, back/forward or an external `spotify:` link
keeps it open. Don't reintroduce route-driven collapsing.

**The Your Library, Local Music and Settings rows navigate nowhere.** They are expanders: a click
toggles the group and nothing else, so a route change only ever comes from a tab underneath. That is
also why an overlaid `SidebarLeft` survives opening a group — it dismisses on navigation, and there
is none.

**Local Music is a top-level route that reuses `LibraryView`.** `Destination::Local(LocalTab)` owns
the imported library, and `Root` builds a second `LibraryView` with `Shelf::Local`; the shelf picks
the state (`Library::local_state`, `local_favorites`) and the settings keys, and every
`TableSource` takes it through a `shelved(.., local)` constructor. Local Music adds a Songs section
holding every scanned track, with a heart on each row, beside the Favorites section the streaming
shelf also has. Its `local-*` settings keys and the `nav-local` i18n key keep the old names so
stored layouts survive; `Screen::Imported` keeps the stored id `imported` for the same reason.
Rename the value, not the key.

**Local files carry their own metadata.** `music::local::wire` resolves artwork by convention:
embedded picture, then `cover`/`folder` beside the track, then the same beside the album folder;
an artist folder answers to `artist` first, then `folder`, then `cover`, in jpg, jpeg, png or webp.
`music::local::tags` reads and writes the embedded tags through lofty, reaching the UI as
`MusicApi::{track_tags, set_track_tags}` — both default to an error, so only the local provider
answers. `state::Tags` owns the read and the write and rescans the folder afterwards;
`views::shared::tag_editor` is the dialog.

**Saved tracks are called Favorites.** `LibraryTab::Songs`, `Section::Favorites`, the `songs`
settings key and `library-liked-songs` all keep their old names; only the wording changed. Local
favorites live in the local tables of `state.sqlite` and reach the same
`MusicApi::set_track_saved` path, so
`Library::saved`/`toggle` route by `music::is_local_id`. `MusicApi::all_tracks` is the odd one out:
it defaults to `saved_tracks` and only the local provider gives it a different answer.

**Which entries the sidebar shows is a setting.** `NavEntry::ALL` (router) is the list; a hidden one
is stored by id in `hidden_nav` and read through `AppSettings::nav_shown`. Your Library still needs
an authenticated provider; Local Music does not need a folder, because an unscanned one is what the
setup screen is for. The setting only hides.

**An empty page is a `ui::Vacancy`, not a line of text.** A large muted glyph over the caption, with
an optional action beneath it: `Section::glyph` picks the icon per library section, and
`shared::local::unconfigured` is the Local Music setup screen — `folder-plus` over a Choose folder
button that both it and Settings build with `shared::local::choose_button`. `ui::vacant` stays the
bare centred text underneath a table.

**Anything that appears rises.** `Menu`, `Modal` and `Toast` wrap themselves in `Rising::rising`,
and `Workspace` drives the same curve by hand for the page transition. There is one entrance in the
app; do not write a second one. Two rules keep it from moving pixels: `layer_scale` and `blur` are
paint-only, so hitboxes never move, and the page transition fades with a scrim over the content
rather than `opacity`, because element opacity is baked into primitives at paint time and would
force the content view out of its `cached` layout path for the length of the animation.

**Shells.** `crates/views/src/shells/` holds the two top-level layouts, `Workspace` and
`FullscreenView`; `Root` swaps between them. A shell owns its own chrome — `Workspace` builds both
sidebars and the player bar — and answers for its title bar through the `shells::Shell` trait
(`title_bar(content, cx) -> TitleBarOptions`). `Root` supplies only the current screen's toolbar and
asks the active shell; it never reaches into a panel.

**New screen checklist:** add a `Destination` variant → add a state entity if it loads data → add
the view under `crates/views/src/` → construct it in `Root::new` and wire it in `Root::show` →
add a sidebar entry in `views/src/chrome/sidebar_left.rs` if it's top-level.

**New library section checklist:** `LibraryView` keeps one `TableState` per `Section`, and the
fixed-size arrays (`views`, `sliders`, `Section::ALL`, `tables()`) are all indexed by
`Section::slot()` — a new section means bumping every one of them, plus a `LibraryTab` or
`LocalTab` variant, a `key(shelf)` for settings persistence, a `vacancy(shelf)` i18n key, a card
renderer, a `deck` arm and a `LIBRARY_TABS` or `LOCAL_TABS` entry. `library/artists.rs` is the
smallest complete example.

**Tables.** Implement `TableSource` (`columns`, `rows`, `cell`, and optionally `compare`, `matches`,
`playing`, `is_loading`), define a `&'static [ColumnSpec<Field>]`, hold a
`TableState<Source>` entity, render `table(&state)`. Column widths use `Width::{Fixed, Fill, Thumb}`.
A table never carries pixel breakpoints: every column declares a `rank` from `ui::rank`
(`SPARE` < `NICE` < `HANDY` < `USEFUL` < `ESSENTIAL`, the default), and the layout drops the
lowest-ranked column whenever the survivors no longer fit their comfortable widths, keeping at least
one unanchored column. Give the columns of one table distinct ranks — equal ranks are evicted
right-to-left. Hidden columns persist through `AppSettings::{hidden_columns, set_hidden_columns}`.

**Toolbar.** `chrome::Toolbar` owns only the search field and lays out whatever a screen hands it.
A screen implements `chrome::Tooled` — `toolbar()` returns its `Entity<Toolbar>`, `tools(&self, cx)`
returns the finished `Vec<AnyElement>` — and wires itself once with `Toolbar::wire`. Adding a
control to a screen never touches `toolbar.rs`. Build the standard controls with the shared builders
in `chrome::tools` (`columns`, `filters`, `sorts`, `views`) rather than writing a menu twice; each
screen owns one `ui::Popovers` so only one of its popovers is open at a time, and holds its own
`tools::Sliders` cache so scrubber positions survive across frames (`LibraryView` keeps one per
section, so tab switches cannot bleed).

**Filtering.** Implement `chrome::Searchable` on the view; `Toolbar::bind` binds it to the search
field in the title bar. Don't build a second search box.

**Actions and keys.** Declare actions in `crates/input/src/lib.rs` (`actions!` macro), bind them in
`bindings()`, handle them with `cx.on_action` (global, in `sonora/src/actions.rs`) or
`.on_action(cx.listener(…))` (scoped). Key contexts: `Workspace`, `Input`, `Table`. Both `cmd-` and
`ctrl-` bindings are registered for every shortcut.

**The tray outlives the window.** `sonora/src/tray.rs` owns one `Tray` entity driven by two
backends: `tray/native.rs` (`tray-icon`, macOS and Windows) and `tray/sni.rs` (`ksni`, Linux over
D-Bus, no gtk). Both expose the same `Icon::new(sender) -> Option<Icon>` / `Icon::show(&Shown)`
pair; the entity turns tray events into `Playback` calls the way `state::remote` does and rebuilds
the labels from `t!` on every playback change, so they follow the language. `install` returns
`false` when no tray can be placed — no StatusNotifierWatcher on the bus, say — and
`actions::register` then keeps the old quit-on-last-window behaviour, so a headless Sonora never
lingers unreachable. With a tray and `close_to_tray` on, the last window closing only flips
`dock::show(false)` (Accessory policy on macOS; a no-op elsewhere) and `show_window` in `main.rs`
brings it back from the tray, a Dock relaunch (`on_reopen`) or a `spotify:` link. `ksni` must stay
on `async-io`: `gpui_linux` already drives `zbus` on that executor, and mixing in `zbus/tokio`
panics at runtime. The icons come from `assets/tray/`, which `scripts/generate-icons.py` derives
from the master like every other artefact — a template glyph for the macOS menu bar, the round
one for Windows and Linux.

**Assets.** `crates/sonora/src/assets.rs` answers GPUI for both icons and fonts: icons come from
the `icons` crate, fonts from a `FONTS` table its build script writes by walking `assets/fonts`.
Neither is a hand-kept list any more — see [Icons](#icons). The UI font is Inter.

**App icons are generated, never hand-edited.** `assets/icon.svg` is the master; `scripts/generate-icons.py`
derives every platform artefact from it — a circle for `assets/linux/` (scalable SVG plus the hicolor
PNG set), an Apple squircle for `assets/macos/sonora.icns`, and a rounded rect for
`assets/windows/sonora.ico`. Change the master and re-run the script; do not touch the outputs.

**`THIRD-PARTY.md` is generated too.** `scripts/generate-notices.py` drives `cargo about` over
`about.toml` and writes the file; a new dependency means re-running it, not editing the output. The
binary must ship it alongside `COPYING`, `assets/fonts/LICENSE.txt` and `assets/icons/LICENSE` —
`flake.nix` and `.github/workflows/release.yml` both do that, and a package that skips it is
distributing unlicensed code.

## Code style

- **Document with doc comments.** Every type and every function whose behaviour is not obvious
  from its name carries a `///` comment saying what it is for and what a caller has to know:
  invariants, ordering, what happens on failure. One or two plain sentences, no restating the
  signature.
  Skip it when the name already says everything. Older files have few comments. Add them as
  you touch the code.
- `use gpui::prelude::*;` then explicit imports; traits imported anonymously (`use ui::ActiveTheme as _;`).
- Module shape: `use` block → `const`s in SCREAMING_SNAKE → types → impls → private free helpers at
  the bottom.
- Prefer combinator chaining over branching in render: `.when()`, `.when_some()`, `.when_else()`,
  `.map()`. Prefer `match` over `if/else` for two-arm boolean choices — that's the house style
  (`match small { true => …, false => … }`).
- Use let-else for early exits; return early rather than nesting.
- `anyhow::Result` at boundaries, `.context("cannot …")` lowercase; log with `log::warn!`/`error!`
  prefixed by subsystem (`"playback: …"`, `"settings: …"`, `"assets: …"`).
- **Do not add tests unless asked.** A change ships with the tests that already exist; writing new
  ones is a separate request. Offering is fine — say what a test would pin down and let the user
  decide — but do not write it, and do not treat a nearby `mod tests` as an invitation to extend it.
- Tests are `#[cfg(test)] mod tests` at the bottom of the file they cover — see
  `music/src/spotify/{collection2,radio,search}.rs` and `ui/src/input/mod.rs`. They're pure-function tests;
  there is no UI or network test harness.
- Dependencies go in the root `[workspace.dependencies]`, then `dep.workspace = true` in the crate.
  `gpui`/`gpui_platform` are pinned to one git rev — bump both together or the build breaks.

## Commits

Conventional Commits: `type(scope): description`, imperative, lowercase, no trailing period, no body.
Scopes in use: `views`, `ui`, `music`, `state`, `playback`, `player`, `settings`, `router`, `local`,
`sonora`, `nix`. Never add a `Co-Authored-By` trailer or any
assistant attribution.

## Pull requests

One heading, one list. Nothing else:

```markdown
## Summary

- add playlist create, rename, delete, visibility, and track mutation support
- add album and playlist queue actions
- add playlist management menus and localized editor UI
```

Bullets are lowercase, imperative, one line each, and describe behaviour rather than files. No
Why/User impact/Validation sections, no checklists, no screenshot boilerplate, no test plan. This
overrides any wider PR template. Never sign off or attribute the assistant.

## Releases

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and **must be
edited in the same commit that cuts a release** — a release that ships without its changelog entry
is incomplete. Cutting a release therefore takes three steps:

1. `chore: release <version>` — `version` in the root `Cargo.toml`, then `Cargo.lock`
   (`cargo check --workspace` rewrites it), and `CHANGELOG.md`: rename `## [Unreleased]` to
   `## [<version>] - <YYYY-MM-DD>`, open a fresh empty `## [Unreleased]` above it, and fix the link
   refs at the bottom — point `[unreleased]` at `compare/v<version>...HEAD` and add a `[<version>]`
   line comparing against the previous tag.
2. The tag `v<version>`, which is what `.github/workflows/release.yml` triggers on.
3. `chore(nix): point the flake at <version>` — once the tag has built, the `release.version` and
   per-target `hash` values in `flake.nix`.

Entries are user-facing sentences under `Added` / `Changed` / `Fixed`, not commit subjects: say what
someone using Sonora can now do, and leave out work no user can observe. Add to `## [Unreleased]` as
features land so cutting a release is only a rename.
