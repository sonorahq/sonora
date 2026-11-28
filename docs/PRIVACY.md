# Privacy Policy

Sonora does not require a Sonora account. It does not collect or store users' music libraries, listening history, streaming credentials or personal profiles on any server of its own.

Sonora does reach the network on its own in a few cases: the installation count, update checks and lyrics lookups. Everything else on this page happens only after the user does something that needs it, such as signing in to a service, turning a feature on, opening a shared link or opening the About page. This page lists every place Sonora sends data and what it sends.

## Installation count

On the first launch, the sign-in screen asks whether Sonora may send a one-time installation report to the installation counter (sonora-stats.nolight.dev), which estimates how many times the application has been installed. The checkbox is ticked by default, and the report is sent with the first action on that screen.

The report only contains:

- application version
- operating system

Sonora does not assign an installation identifier, device identifier, streaming account identifier, listening history, music library information, or other application data in this request.

Unticking the checkbox stops the report for good, and Sonora does not ask again. Opting out does not affect Sonora's functionality.

As with any network request, the hosting and network infrastructure (Cloudflare) may process standard connection metadata such as an IP address as necessary to deliver the request. Sonora does not include the IP address in the report or use it as an identifier.

## Update checks

Sonora asks GitHub (api.github.com) for the latest release when it starts and every six hours while it runs. With automatic updates on, it downloads the new release and its checksums from github.com. GitHub receives the request like it would from a web browser, along with the Sonora version.

Update checks are on by default on Windows and for copies that update themselves, such as the AppImage and the macOS app. They are off for copies installed through a package manager. They can be turned off with "Check for updates" in the settings. Switching the update channel checks once even when checks are off.

## Lyrics

Lyrics are on by default. While they are on, Sonora looks up lyrics for every track that plays and for the next one in the queue, whether or not the lyrics are on screen. Each enabled source receives the track's title and artist, and some also receive its album and length. Tracks from local files are looked up too.

The sources on by default are:

- lrc.red, listed as "Apple Music" in the settings, which receives title, artist, album and length
- Musixmatch, which receives title, artist, album and length, or the Spotify track ID for a Spotify track. Sonora obtains a Musixmatch user token, keeps it on the device and sends it with every request, so Musixmatch can tell requests from the same installation apart.
- LRCLIB, which receives title and artist
- YouTube Music, searched as a guest with title and artist, even without a YouTube account
- Spotify, searched with title and artist through the signed-in account, only when a Spotify account is signed in

Kugou and NetEase are off by default. When turned on, they receive the title and artist. Subsonic, Jellyfin and Plex servers are asked only for lyrics of their own tracks.

Lyrics can be turned off entirely, per source, or for local files alone in the settings. Offline mode stops all online lookups.

## Streaming and third-party services

When you use Spotify, Apple Music, YouTube Music, Deezer, Tidal, Subsonic, Jellyfin, Plex or another online integration, Sonora communicates with that service as needed to provide what you asked for. Data handled by those services is subject to their own privacy policies and terms.

Some services are told what you play. Apple Music receives a report for every catalog track played. Subsonic, Jellyfin and Plex servers receive now-playing and stop reports with the playback position. To check that the network is up, Sonora opens a connection to the current service's host.

Plex and Jellyfin receive the computer's hostname as the device name with their requests, along with a random identifier Sonora keeps on the device. Plex sees the same identifier for every Plex account on the device, and Jellyfin gets a new one at each sign-in.

Signing in to YouTube Music, Apple Music, Deezer or Plex opens the service's own sign-in page in a separate window that keeps no cookies or data after it closes. To play YouTube tracks, Sonora may load a YouTube page in a hidden window of the same kind, where Google's own scripts check that playback comes from a real client.

## Protected playback

Apple Music tracks are encrypted, and playing them needs Google's Widevine module. When no browser on the device already has a copy, Sonora asks before downloading it from Google (update.googleapis.com), then shows Google's terms from the download and installs the module only once the user accepts them. Google receives the operating system and processor architecture. After the first download, Sonora asks Google for a newer version each time it starts.

Spotify plays through Spotify's own player program. Sonora asks before downloading it from Spotify (soloist-builds.spotifycdn.com) and afterwards downloads new versions without asking.

## Spotify Connect

When a Spotify account is signed in, Sonora shows up as a Spotify Connect device by default, so Spotify's apps can see what it plays and control it. To do this, Sonora sends Spotify a device name and a device identifier.

With the default naming, the device name contains the computer's hostname, as in "Sonora (my-laptop)". Every device signed in to the same Spotify account shows this name in its device list. The name can be changed to "Sonora" alone or to a custom name, and Spotify Connect can be turned off, in the settings.

The device identifier is created once and kept on the device, so Spotify sees the same identifier every time Sonora starts. Starting, joining or managing a Jam sends this identifier to Spotify too.

When Spotify's player is paired with an account, it announces itself on the local network.

## Google Cast

While YouTube Music is the current service, Sonora looks for Google Cast devices on the local network and asks each one it finds whether it can play YouTube. Casting to a device gives that device and YouTube a random identifier that Sonora keeps on the device. This is controlled by the same setting as Spotify Connect and is on by default.

## Sonora Sessions

Sonora Sessions lets people listen together. It is the one online service Sonora runs, at sessions.sonorahq.org, and it is used only when the user starts or joins a session. A different server can be set in the settings.

While in a session, Sonora sends the server and every other member:

- the display name and profile picture link of the signed-in account
- which services the user is signed in to
- the playing track and up to 100 upcoming tracks, with their title, artists, album, ISRC and cover link
- whether playback is paused and where it is in the track

Local files are never shared with a session.

## Shared links

Opening a spotify.link or Deezer short link (link.deezer.com, deezer.page.link or dzr.page.link), or a YouTube channel link written with an @ handle, makes Sonora fetch that link from its host to find out where it points. The host receives the request like it would from a web browser.

## SponsorBlock

SponsorBlock is off by default. When it is on, Sonora asks SponsorBlock (sponsor.ajay.app) which parts of a YouTube video to skip. The request carries only the first four characters of a hash of the video's ID, and SponsorBlock answers with every video it knows whose hash starts the same way. SponsorBlock can only narrow the video down to those, and it can tell which one it was when no other video shares the prefix.

## Discord rich presence

Discord rich presence is off by default. When it is turned on and Discord is running on the same computer, Sonora sends Discord the title, artist and album of the playing track, a link to its cover, and when it started. Discord shows this on the user's profile. By default the status also carries a button linking to the track on its provider. The settings can hide the track details, so the status only says that music is playing.

Discord can only show a cover it can download. When the track's own cover is not public, as for local files and Subsonic, Jellyfin or Plex tracks, Sonora by default uploads a copy of the cover, at most 512 pixels wide, to uguu.se, a public file host that deletes it after three hours. Sonora uploads it again every two and a half hours while the status stays up. Uploading can be turned off in the settings.

When the upload is off or fails, Sonora searches public catalogs for a cover by the track's artist and album, or its title when it has no album. It asks Deezer first, then iTunes, then MusicBrainz and the Cover Art Archive. For local files this search can be turned off in the settings.

## Scrobbling

Scrobbling is off until an account is linked in the settings. Sonora can scrobble to Last.fm, Libre.fm, ListenBrainz, a self-hosted Maloja server and teal.fm. A linked service receives the artist, title, album and length of each track played long enough to count, and when it was played. Last.fm, Libre.fm, ListenBrainz and teal.fm are also told which track is playing right now.

teal.fm has no server of its own to send listens to. Sonora signs in to the user's ATProto account, usually on Bluesky, with an app password and writes each listen and the current track as records in that account's repository. Those records are public, like posts, and teal.fm and any other ATProto app can read them. To find the account's server, Sonora looks the handle up on the handle's own domain, then on Bluesky's public API, and reads the account's identity document from the PLC directory or the domain of a `did:web` identity. To link each listen to a recording, Sonora also looks the track up on MusicBrainz by its ISRC, or by its title and artist.

The keys, tokens and app passwords that link these accounts are stored in Sonora's accounts folder next to the streaming sign-ins, in files on Linux and macOS only the user can read.

## About page

The About section of the settings loads contributors' profile pictures from GitHub when it is opened.

## Authentication credentials

Authentication credentials used by Sonora are stored locally on the user's device.

Sonora does not transmit Spotify, Apple Music, YouTube Music, Deezer, Tidal, or other provider credentials to Sonora-operated servers.

Credentials are sent only to the relevant third-party service when required for authentication or API requests.

## Local data

Sonora stores application data locally on the user's device, including settings, playback state, provider credentials, local-library information, and other application state required for the application to function correctly.

By default, Sonora keeps the tracks you play on disk, so playing them again downloads nothing. This cache takes up to 2 GB, which can be changed in the settings, and Spotify keeps up to 1 GB more of its own files. Tracks you download are kept until you remove them. Played and downloaded tracks are encrypted with a key held in the operating system's keychain, or in a key file only the user can read when no keychain is available. Spotify's files stay encrypted under Spotify's own key. Turning off "Keep played tracks" deletes the played tracks and leaves downloads alone.

## Logs

Sonora may write diagnostic logs locally on the user's device.

These logs are not uploaded to Sonora-operated servers.

Users may choose to share logs manually when reporting bugs or requesting support.

## Changes to this policy

Material changes to this privacy policy will be published in the Sonora source repository.

## Contact

Privacy-related questions may be submitted through the Sonora GitHub repository:

https://github.com/sonorahq/sonora
