# sonora

A native music streaming client, built with Rust and [GPUI](https://github.com/zed-industries/zed)
(Zed's UI framework). Cargo workspace, edition 2024, resolver 3.

## Crate layout

```
crates/
  sonora/     binary: main, window, actions, asset registry, HTTP client shim, tray, dock
  views/      screens, shells (Workspace, FullscreenView) and app chrome
  state/      GPUI entities holding app state, and all async orchestration
  music/      provider traits and models in the root, one submodule per provider (no GPUI)
  storage/    the state.sqlite and cache.sqlite paths, schema and connection setup
  ui/         design system: theme, metrics, layout ladder and reusable elements
  router/     Destination enum, navigation history, Link trait
  input/      text input element, global actions and keybindings
  i18n/       Fluent localization: the `t!` macro, locale selection, embedded .ftl
  icons/      the icon packs: registry, active pack, path resolution, AssetSource
  embed/      build-script helper that walks a folder and writes include_bytes! literals
  webview/    a native browser window over a throwaway session, for cookie sign-ins and PO tokens
  widevine/   the Widevine CDM host, pssh and CENC fMP4 parsing, for any DRM'd provider
```

Dependency direction is strict. Do not create a back edge:

```
sonora → views → state → music
         state, music → storage
         state → webview
         music → widevine
         all ui-side crates → ui, router, input → ui → gpui
         every ui-side crate → i18n, icons → gpui
```

- `state` and `views` see only the traits and models in the root of `music`, never a provider
  module. Only `sonora/src/main.rs` names a concrete provider.
- `music` never depends on `gpui`. `ui` never knows about `music`, `state` or playback. Widgets
  that need app state live in `views/src/chrome/`.
- `music::engine` is the one playback engine and `music::stream` the one progressive download. A
  provider implements `engine::Fetch` and `stream::Body` rather than writing its own. The engine
  applies loudness normalisation, so a provider only reports a track's level through
  `Fetch::loudness`. Spotify is the one exception, because librespot decodes and normalises
  inside its own player.
- `music::drm` is the only way `state` and `views` reach protected playback. The CDM is never
  shipped in any artefact or package.

## Rules

- **Reuse before you build.** Check `crates/ui/src/lib.rs`, `crates/views/src/shared/` and
  `crates/views/src/chrome/` first, and extend an element with a builder method rather than
  writing a sibling. New elements copy the shape of `ui/src/button.rs`, including the
  `mem::take` and `refine` of caller styles.
- **Never call the Spotify Web API.** No `reqwest` to `api.spotify.com`, no client secret, no
  `rspotify`. Spotify data comes from librespot's `spclient`, and auth uses Spotify's own client id
  in `auth.rs`.
- **Never hardcode a color, radius or size.** Read them from `cx.theme()` and `theme.metrics`. A
  literal `px(…)` is allowed only for a local, non-scaling detail declared as a module `const`. A
  new color token goes into `Theme`, every `Theme::*()` constructor, `ThemeOverrides` and
  `apply_color!`.
- **Never write a bare breakpoint.** Use the `ui::Room` ladder, and classify width with
  `Chrome::room` or `Chrome::content`, never the raw viewport. A view whose layout depends on
  width observes `Chrome::entity(cx)`.
- **Every user-facing string comes from Fluent.** Add the key to `assets/i18n/en-US/main.ftl`. Other
  locales may lag. Never call `t!` in a constructor: store the key and resolve it in `render`.
  Counts use Fluent selectors. Log messages and `.context(…)` stay in English.
- **Every interactive element is announced by screen readers.** A node reaches the
  accessibility tree only with both `.id()` and `.role()`; pair every role with an
  `aria_label` resolved from Fluent in `render` (icon-only buttons take their name from the
  tooltip). Keep ids stable across frames and unique within a list: wrap rows in
  `div().id(("kind", stable_id))` with `aria_position_in_set`/`aria_size_of_set`, never a bare
  row index. Extend `crates/ui/tests/a11y_tree.rs` with every new primitive.
- **Every interactive element is announced by screen readers.** A node reaches the
  accessibility tree only with both `.id()` and `.role()`; pair every role with an
  `aria_label` resolved from Fluent in `render` (icon-only buttons take their name from the
  tooltip). Keep ids stable across frames and unique within a list: wrap rows in
  `div().id(("kind", stable_id))` with `aria_position_in_set`/`aria_size_of_set`, never a bare
  row index. Extend `crates/ui/tests/a11y_tree.rs` with every new primitive.
- **Network work runs on tokio.** Anything touching `MusicApi`, librespot or sockets goes inside
  `Io::global(cx).spawn`, awaited from `cx.spawn` and applied in `this.update` ending with
  `cx.notify()`. Store the returned `Task` in a field and never detach a data load. Network-backed
  features belong in a `state` entity, not a view. An entity that caches provider data clears on
  `SessionEvent::SignedOut`.
- **Never drive a player from a view.** Go through `state::Playback`.
- **Assets are picked up from their folder.** Drop an icon into `assets/icons/<pack>/` or a face
  into `assets/fonts/`. Call sites spell `"icons/<name>.svg"` and resolve it with `icons::path` at
  render.
- **Generated files are regenerated, never edited.** App icons come from
  `scripts/generate-icons.py`, `THIRD-PARTY.md` from `scripts/generate-notices.py`, the README
  translation table from `scripts/i18n-coverage.py`.
- **Do not add tests unless asked.** Offering is fine. Existing tests are `#[cfg(test)] mod tests`
  at the bottom of the file they cover. Accessibility regression tests in
  `crates/ui/tests/a11y_tree.rs` are the exception: extend them with every new primitive. Accessibility regression tests in
  `crates/ui/tests/a11y_tree.rs` are the exception: extend them with every new primitive.

## Building and checks

```sh
nix develop                    # or direnv allow
cargo run --locked --package sonora
cargo fmt
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

The devShell has no `cargo` or `cargo-clippy`; those come from the system profile. The flake's
default package is the released binary, so `nix build` never sees the working tree, while
`nix build .#sonora-src` compiles it.
`.cargo/config.toml` links with mold on `x86_64-unknown-linux-gnu`, so mold must be on PATH, or
build with `RUSTFLAGS=""`. `crates/widevine` compiles C++, so a C++ compiler is needed too.

Clippy is clean, `--all-targets` included, and stays that way. Boxed callback fields get a
module-local `type` alias rather than a `type_complexity` warning. The one `#[allow]` is
`reversed_empty_ranges` on the `clamp_range("abc", &(2..1))` test in `crates/ui/src/input/mod.rs`.

## Code style

- Every type and every function whose behaviour is not obvious from its name carries a `///`
  comment saying what it is for and what a caller has to know. One or two plain sentences.
- `use gpui::prelude::*;` then explicit imports. Traits are imported anonymously
  (`use ui::ActiveTheme as _;`).
- Module order: `use` block, `const`s, types, impls, private free helpers at the bottom.
- In render, prefer `.when()`, `.when_some()` and `.map()` over branching. Two-arm boolean choices
  use `match flag { true => …, false => … }`. Use let-else and return early.
- `anyhow::Result` at boundaries with lowercase `.context("cannot …")`. Logs are prefixed by
  subsystem (`"playback: …"`).
- Dependencies go in the root `[workspace.dependencies]`. `gpui` and `gpui_platform` are pinned to
  one git rev and move together.

## Commits

Conventional Commits: `type(scope): description`, imperative, lowercase, no trailing period, no
body. Scopes in use: `views`, `ui`, `music`, `state`, `playback`, `player`, `settings`, `router`,
`local`, `sonora`, `nix`.

## Pull requests

One heading, one list and, for a visual change, screenshots. Nothing else:

```markdown
## Summary

- add playlist create, rename, delete, visibility, and track mutation support
- add album and playlist queue actions
```

Bullets are lowercase, imperative, one line each, and describe behaviour rather than files.

A pull request that noticeably changes what the user sees gets screenshots of the result. That
means a new screen or element, a reworked layout, a theme change, or anything else a reviewer
would only judge by looking at it. When the change alters something that already exists, show it
before and after. The images go below the list with no heading. A small spacing nudge or a change
nobody can see needs none.

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add to
`## [Unreleased]` as features land, under `Added`, `Changed` or `Fixed`. Each entry is one short
line saying what someone using Sonora can now do, since the notes are posted to Discord. Leave out
implementation details and any work no user can observe. Cutting a release is the
`release-sonora` skill.
