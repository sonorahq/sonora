# Contributing to Sonora

Before you build a feature, read the [vision](https://sonorahq.org/docs/vision) and the
[roadmap](https://sonorahq.org/docs/roadmap). When unsure, please open an issue first.
We hate closing pull requests for changes that don't align with the vision as much as
you do.

## Building

Use Nix where available. The dev shell has everything:

```sh
nix develop          # or: direnv allow
cargo run --locked --package sonora
```

Without Nix you need [rustup](https://rustup.rs), which picks up `rust-toolchain.toml`, plus a
C++ compiler, `cmake`, `pkg-config`, `mold`, and the development packages for ALSA, D-Bus,
fontconfig, libxkbcommon, libxcb, libX11 and SQLite. Running it also needs a Vulkan driver. Install
those from your package manager.

Run `cargo fmt`, `cargo test --workspace` and `cargo clippy --workspace --all-targets` before you
open a pull request.

## Pull requests

Open them against `main`. Commits follow [Conventional Commits](https://www.conventionalcommits.org/)
with no body, and any major user-facing changes should be mentioned under `## [Unreleased]` in
`CHANGELOG.md`. Keep each entry to one short line about what users can now do, without
implementation details.

## Translations

English must be present for every key. Other locales can lag behind, since a missing key falls back to English.
It's okay to machine-translate a language you don't speak. Run `scripts/i18n-coverage.py` afterwards to refresh
the table in the README.

## Accessibility

Sonora must stay usable with screen readers. Every interactive element needs a stable id, an
AccessKit role, and an accessible name resolved from Fluent in `render`; icon-only buttons take
their name from their tooltip, and list rows expose position and size of set. Run
`cargo test -p ui --test a11y_tree` and extend it when you add a primitive: it runs in CI on
every pull request, so a missing role or name fails the build. Roles on composed views have no
headless coverage yet, so new screens need a manual pass with a screen reader before merge.

## AI

Use whatever tools you like, but you must understand, review and test what you submit. PR descriptions,
issues and replies in discussions must be your own words. A generated summary of a small change is fine,
and so is AI proofreading or translation of text you wrote. Contributions that ignore this may be closed
without comment.

## License

Sonora is GPL-3.0-or-later, and your contributions are licensed under the same terms.
