<div align="center">

# Sonora

[![Build](https://img.shields.io/github/actions/workflow/status/sonorahq/sonora/release.yml?style=flat-square&label=build)](https://github.com/sonorahq/sonora/actions/workflows/release.yml)
[![License](https://img.shields.io/github/license/sonorahq/sonora?style=flat-square&label=license)](./COPYING)
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

* **Apple Music, Spotify, YouTube Music, Deezer, Subsonic/Navidrome** and local playback
* Gapless playback, audio normalization, shuffle, sleep timer
* Synced/karaoke lyrics, background vocals, and romanization
* Scrobbling with LastFM, ListenBrainz, LibreFM, and Maloja
* Themes, fonts, icons, transparency, blur, and window styling
* Discord Rich Presence, native file opening
* macOS, Windows, Linux, and (probably) FreeBSD support

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

### Linux

#### Arch

Install from the AUR with your AUR helper of choice:

```sh
yay -S sonora-bin
```

`sonora-bin` installs the prebuilt release binary. `sonora` builds the same version from source
instead, which takes a while on a Rust and GPUI tree but links against your own system libraries:

```sh
yay -S sonora
```

Either `pipewire-alsa` or `pulseaudio-alsa` is required, matching your sound server.

#### Flatpak

Add the Sonora repository (updates with `flatpak update`):

```sh
flatpak install --user https://sonorahq.github.io/sonora/sonora.flatpakref
```

#### AppImage

Download the `x86_64` AppImage from the
[latest release](https://github.com/sonorahq/sonora/releases/latest), make it executable and run
it:

```sh
chmod +x sonora-*.AppImage
./sonora-*.AppImage
```

An `aarch64` build is published beside it. The AppImage carries no Vulkan driver and no ALSA
bridge, so both still come from your system. It does not update itself, but it carries its update
information, so [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) or an
AppImage manager such as [AppManager](https://github.com/kem-a/AppManager) can fetch a new release
for you.

### Nix

The flake packages the latest tagged release binary or builds from source if unavailable for your platform.

```nix
inputs.sonora.url = "github:sonorahq/sonora";
```

```text
inputs.sonora.packages.${system}.default
inputs.sonora.packages.${system}.sonora (build from source)
inputs.sonora.packages.${system}.sonora-bin (prebuilt, if available)
```

You can set configuration options via the included Home Manager module under `programs.sonora`:

```nix
{
  imports = [ inputs.sonora.homeManagerModules.default ];
  programs.sonora = {
    enable = true;
    settings = {
      provider = "youtube";
      appearance.theme = "dark";
    };
  };
}
```

### Windows

#### Installer

Download and run the [installer](https://github.com/sonorahq/sonora/releases/latest/download/Sonora-Setup.exe),
or the [ARM installer](https://github.com/sonorahq/sonora/releases/latest/download/Sonora-Setup-arm64.exe)
on Windows on ARM.

#### Portable

Download the latest `windows-msvc.exe` for your architecture from [Releases](https://github.com/sonorahq/sonora/releases/latest).

## Custom themes

Themes live in `sonora/themes` under the platform config directory, such as
`$XDG_CONFIG_HOME/sonora/themes` on Linux. Open the folder from Appearance settings, then add a
JSON file such as `my-theme.json`:

```json
{
  "name": "My Theme",
  "author": "Your Name",
  "version": 1,
  "theme": {
    "background": "#101010",
    "foreground": "#f5f5f5",
    "primary": "#8aadf4",
    "primary_foreground": "#101010",
    "selection": "#4f69a3"
  }
}
```

Select it in Appearance settings or set `appearance.theme` to `"my-theme"`. The identifier is
the filename before `.json`; built-in theme identifiers are reserved. `name`, `author`, and
`version` are required, and the supported version is `1`. Custom themes are treated as Dark and
use its palette for omitted values. They accept the same keys as `appearance.theme_overrides`,
including `radius` and `font_size`; existing inline overrides are applied last.

Sonora picks up valid file changes, additions, and deletions while it runs. A broken edit keeps
the last valid version until the file is valid again, as long as Sonora remains open.

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
| English (`en-US`) | 745/745 | 100% |
| Deutsch (`de`) | 628/745 | 84% |
| Español (`es`) | 691/745 | 93% |
| Français (`fr`) | 628/745 | 84% |
| Italiano (`it`) | 606/745 | 81% |
| Bahasa Indonesia (`id`) | 606/745 | 81% |
| 日本語 (`ja`) | 606/745 | 81% |
| Русский (`ru`) | 712/745 | 96% |
| Українська (`uk`) | 712/745 | 96% |
| Polski (`pl`) | 712/745 | 96% |
| Português (Brasil) (`pt-BR`) | 606/745 | 81% |
| 简体中文 (`zh-CN`) | 606/745 | 81% |
| Türkçe (`tr`) | 606/745 | 81% |
| Shqip (`sq`) | 715/745 | 96% |

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

## Code signing
Sonora has applied for code signing through SignPath Foundation. Current releases are not yet signed through SignPath Foundation. If approved, signed releases will use free code signing provided by SignPath.io, with a certificate by SignPath Foundation.

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
