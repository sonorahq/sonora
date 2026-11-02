<div align="center">

<img src="./.github/readme-icon.svg" alt="" width="60" height="60">

<h1>Sonora</h1>

<p align="center">
  <a href="https://sonorahq.org/changelog">Changelog</a>
  ·
  <a href="https://sonorahq.org/docs">Docs</a>
  ·
  <a href="https://sonorahq.org/docs/roadmap">Roadmap</a>
  ·
  <a href="https://sonorahq.org/faq">FAQ</a>
</p>

[![Build](https://img.shields.io/github/actions/workflow/status/sonorahq/sonora/release.yml?style=flat-square&label=build)](https://github.com/sonorahq/sonora/actions/workflows/release.yml)
[![License](https://img.shields.io/github/license/sonorahq/sonora?style=flat-square&label=license&cacheSeconds=3600)](./COPYING)
![Installs](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fsonora-stats.nolight.dev%2Fcount&query=%24.count&label=Installs&color=blue&style=flat-square)
\
[![Discord](https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white)](https://discord.gg/a8N8Tx23rV)
[![Matrix](https://img.shields.io/badge/Matrix-000000?style=for-the-badge&logo=matrix&logoColor=white)](https://matrix.to/#/#sonora:nolight.dev)

### A native music streaming client, built with Rust and GPUI

Stream from your favorite services and play local files — all in one **native** app.
</div>

<div align="center">
    <table>
      <tr>
        <td colspan="2">
          <img width="1613" height="981" alt="image" src="https://github.com/user-attachments/assets/7952a912-7fbc-4186-b467-a08dd7e71e22" />
        </td>
      </tr>
      <tr>
        <td width="50%">
          <img width="1623" height="987" alt="image" src="https://github.com/user-attachments/assets/580bf9d6-db85-4fde-b599-82ba2a28cc51" />
        </td>
        <td width="50%">
          <img width="1623" height="987" alt="image" src="https://github.com/user-attachments/assets/64fcd709-5917-432c-a418-2e07527343d2" />
        </td>
      </tr>
    </table>
</div>
<div align="center">
    <sub>
      Adaptive themes are optional. Everything is (or will be) customizable.
    </sub>
</div>

> [!IMPORTANT]
> **Sonora is not a piracy tool.**
>
> Sonora is not a platform for obtaining or sharing copyrighted material. We will not implement any functions that can be used to export decrypted streams, DRM licenses, content keys, or to convert protected streams into media files.
>
> Sonora is not designed to circumvent subscriptions or other restrictions put in place by music streaming platforms. If the service demands that you have a valid subscription in order to play back their tracks, so will Sonora.
>
> Features aimed at ripping, downloading, distributing, or gaining access to protected streaming content are out of scope for the project.

## Features

- **Apple Music, Spotify, YouTube Music, Deezer, Tidal, Subsonic/Navidrome** and local playback
- Gapless playback, audio normalization, shuffle, sleep timer
- Synced/karaoke lyrics, background vocals, and romanization
- Scrobbling with LastFM, ListenBrainz, LibreFM, and Maloja
- Themes, fonts, icons, transparency, blur, and window styling
- Discord Rich Presence, native file opening
- macOS, Windows, Linux, and (probably) FreeBSD support

## Installation

### macOS

Install with [Brew](https://brew.sh/):

```sh
brew install --cask nolight132/tap/sonora
```

After installing (thanks Apple):

```sh
xattr -dr com.apple.quarantine /Applications/Sonora.app
```

Or install the [disk image](https://github.com/sonorahq/sonora/releases/latest), which updates itself.

### Linux

#### Arch

Install from the AUR with your AUR helper of choice:

```sh
yay -S sonora-bin
```

Or:

```sh
yay -S sonora
```

#### Flatpak

```sh
flatpak install --user https://sonorahq.github.io/sonora/sonora.flatpakref
```

#### AppImage

Download from the [latest release](https://github.com/sonorahq/sonora/releases/latest), make it executable and run
it:

```sh
chmod +x sonora-*.AppImage
./sonora-*.AppImage
```

### Nix

Add either of those inputs:

```nix
inputs.sonora.url = "github:sonorahq/sonora";
```

```text
inputs.sonora.packages.${system}.default     (prebuilt where available, else from source)
inputs.sonora.packages.${system}.sonora      (same as default)
inputs.sonora.packages.${system}.sonora-bin  (prebuilt, x86_64-linux, aarch64-linux, aarch64-darwin)
inputs.sonora.packages.${system}.sonora-src  (always built from source)
```

Building from source on macOS needs Xcode added to the Nix store, because the shaders compile with Apple's Metal compiler.

Configuration with Home Manager is provided by the module under `programs.sonora`.

### Windows

#### Installer

Download and run the [installer](https://github.com/sonorahq/sonora/releases/latest/download/Sonora-Setup.exe),
or the [ARM installer](https://github.com/sonorahq/sonora/releases/latest/download/Sonora-Setup-arm64.exe)
on Windows on ARM.

#### Portable

Download the latest `windows-msvc.exe` for your architecture from [Releases](https://github.com/sonorahq/sonora/releases/latest).

### Nightly

[Nightly builds](https://github.com/sonorahq/sonora/releases/tag/nightly) are available.

```sh
brew install --cask nolight132/tap/sonora-nightly
yay -S sonora-nightly-bin
nix run github:sonorahq/sonora/nightly
```

## Community

Feel free to join our [Discord](https://discord.gg/a8N8Tx23rV) server and [Matrix](https://matrix.to/#/#sonora:nolight.dev) space.
Discord is the primary one, but we do have a Matrix bridge.

## AI policy

We have nothing against the usage of LLMs in the project — in fact, we use them ourselves.
We believe that AI can speed up development in a lot of meaningful ways and be a useful
tool for learning new concepts. We have also found it particularly helpful for
contributing to substantial codebases such as GPUI and librespot, where it has helped
us quickly locate the relevant parts of the code.

**However**, using AI cannot act as an excuse for failing to
understand, review, and test the changes proposed. Furthermore, we expect communication
with a real person, not a computer. This includes but is not limited to PR/issue text
generation, comments in discussions, etc. A short summary of minor changes can be
generated and does not need to be disclosed explicitly, but the reasoning and motivation
behind a change must come from the contributor and reflect their own understanding.

Note that PRs that fail to adhere to these requirements may be rejected without further notice.

AI-assisted proofreading and translation of human-written text are permitted.

## Translations

<!-- i18n:start -->

| Language | Translated | Coverage |
| --- | --- | --- |
| English (`en-US`) | 1220/1220 | 100% |
| Deutsch (`de`) | 1128/1220 | 92% |
| Español (`es`) | 1128/1220 | 92% |
| Français (`fr`) | 1129/1220 | 93% |
| Italiano (`it`) | 1128/1220 | 92% |
| Bahasa Indonesia (`id`) | 1128/1220 | 92% |
| 日本語 (`ja`) | 1128/1220 | 92% |
| Русский (`ru`) | 1170/1220 | 96% |
| Українська (`uk`) | 1128/1220 | 92% |
| Polski (`pl`) | 1128/1220 | 92% |
| Čeština (`cs`) | 1128/1220 | 92% |
| Português (Brasil) (`pt-BR`) | 1128/1220 | 92% |
| 简体中文 (`zh-CN`) | 1128/1220 | 92% |
| Türkçe (`tr`) | 1128/1220 | 92% |
| Shqip (`sq`) | 1128/1220 | 92% |

<!-- i18n:end -->

## Star History

<a href="https://www.star-history.com/?repos=sonorahq%2Fsonora&type=date&logscale=&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=sonorahq/sonora&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=sonorahq/sonora&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=sonorahq/sonora&type=date&legend=top-left" />
 </picture>
</a>

## Credits

Sonora is built with the help of some incredible open-source projects, including:

- [Zed](https://github.com/zed-industries/zed) — a wonderful editor (~~ab~~)used by all core team members. Conveniently provides `gpui` — their native Rust rendering stack.
- [librespot](https://github.com/librespot-org/librespot) — Spotify playback and library integration.
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — certain YouTube ideas implemented in [ytmusic-rs](https://github.com/sonorahq/ytmusic-rs) :)

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

The Windows executable and installer are built by GitHub Actions from this repository and signed
through SignPath.

- Reviewers: members of sonorahq
- Approvers: [@nolight132](https://github.com/nolight132)

## License

Copyright (C) 2026 Sonora Contributors.

Sonora is free software, released under the [GNU General Public License version
3 or later](COPYING).

Sonora is an unofficial client and is not affiliated with, endorsed by, or
sponsored by Spotify AB.

The binary also embeds the [Inter](https://github.com/rsms/inter) typeface (SIL
Open Font License 1.1) and four interchangeable icon sets:
[Lucide](https://lucide.dev) (ISC), [Iconoir](https://iconoir.com) (MIT),
[Remix Icon](https://remixicon.com) 4.8.0 (Apache 2.0) and the
[Solar](https://www.figma.com/community/file/1166831539721848736) Linear set
(CC BY 4.0, by 480 Design). Each pack keeps its licence beside its files in
`assets/icons`. `THIRD-PARTY.md` lists every bundled dependency.
