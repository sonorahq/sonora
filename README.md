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
[![Support us](https://img.shields.io/badge/Support_us-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white)](https://ko-fi.com/R5A72AGYNZ)

### A native, familiar music streaming client

Stream from your favorite services and play local files — all in one **native** app.
</div>

<div align="center">
    <table>
      <tr>
        <td colspan="2">
          <img width="1556" height="967" alt="image" src="https://github.com/user-attachments/assets/c797cb19-9631-45b0-91b6-695f28f4143d" />
        </td>
      </tr>
      <tr>
        <td width="50%">
          <img width="1555" height="966" alt="image" src="https://github.com/user-attachments/assets/b556bcd0-2823-485f-83f3-41e1099fdc45" />
        </td>
        <td width="50%">
          <img width="1555" height="966" alt="image" src="https://github.com/user-attachments/assets/f27400ab-9264-4d82-9a70-e0a60e0c3a5f" />
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

Pick a channel with the input's ref and install `inputs.sonora.packages.${system}.default`:

```nix
inputs.sonora.url = "github:sonorahq/sonora/stable";   # the latest release
inputs.sonora.url = "github:sonorahq/sonora/nightly";  # the nightly
inputs.sonora.url = "github:sonorahq/sonora";          # main
```

Every channel is built for x86_64-linux and aarch64-linux and pushed to a binary cache, and `stable` only moves to a
release once its build is there. Trust the cache and Nix downloads Sonora instead of compiling it:

```nix
nix.settings = {
  extra-substituters = [ "https://sonora.cachix.org" ];
  extra-trusted-public-keys = [ "sonora.cachix.org-1:qbPeDsvZPbwUWHpQS5Oe0bjz5oHAVscHem+MCzwrc9Q=" ];
};
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

## Translations

<!-- i18n:start -->

| Language | Translated | Coverage |
| --- | --- | --- |
| English (`en-US`) | 1282/1282 | 100% |
| Deutsch (`de`) | 1108/1282 | 86% |
| Español (`es`) | 1106/1282 | 86% |
| Français (`fr`) | 1107/1282 | 86% |
| Italiano (`it`) | 1106/1282 | 86% |
| Bahasa Indonesia (`id`) | 1106/1282 | 86% |
| 日本語 (`ja`) | 1106/1282 | 86% |
| Русский (`ru`) | 1207/1282 | 94% |
| Українська (`uk`) | 1108/1282 | 86% |
| Polski (`pl`) | 1108/1282 | 86% |
| Čeština (`cs`) | 1106/1282 | 86% |
| Português (Brasil) (`pt-BR`) | 1106/1282 | 86% |
| 简体中文 (`zh-CN`) | 1106/1282 | 86% |
| Türkçe (`tr`) | 1106/1282 | 86% |
| Shqip (`sq`) | 1106/1282 | 86% |

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
