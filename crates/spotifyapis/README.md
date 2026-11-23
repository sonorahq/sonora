# spotifyapis

`spotifyapis` is a low-level Rust client for Spotify’s internal APIs. It runs on Tokio and handles OAuth, server discovery, HTTP requests, Dealer WebSocket messages, and audio-key exchange. Service methods return Spotify’s wire types or decoded results for the caller to use.

## API coverage

| Area | Operations |
| --- | --- |
| Account | Browser sign-in with a loopback callback, saved grant validation, automatic token refresh, and regional service discovery |
| Catalogue | Search tracks, albums, artists, and playlists. Fetch extended metadata in batches, read public profiles and their playlists, and fetch synced or unsynced lyrics. |
| Discovery | Read personalized Home shelves and the Browse directory. Read projected Browse section entries through GetLists. |
| Collection | Page through saved items, write collection changes, and update pins. |
| Playlists | Read snapshots, summaries, and library pages. Batch field projections with GetLists. Create playlists, apply revision-bound edits, update the library list, and upload and register covers. |
| Connect | Publish device state, deactivate or unregister a device, set remote volume, transfer playback, and send playback or queue commands. |
| Jam | Read the current session or an invitation, start or join a session, leave or end it, and remove a member. |
| Dealer | Receive state updates, library changes, Jam notifications, and playback commands. Reply to commands on the connection that received them. |
| Playback | Resolve contexts and SequenceProxy pages, resolve signed audio CDN URLs, and retrieve audio keys from a Spotify access point. |
| Identifiers | Convert between binary catalogue IDs and their 22-character base62 form. |

## Sign-in and requests

Create an [OAuth grant](src/auth.rs) with a caller-owned credential file, client ID, and loopback redirect. `OAuth::login` passes the authorization URL to a callback so the application can open it in a browser. Subsequent launches can use `OAuth::restore`. The grant file contains a refresh credential and should be kept private.

This example restores an existing grant and searches for tracks. Run it inside a Tokio runtime.

```rust
use std::path::PathBuf;

use spotifyapis::{Session, auth::OAuth, proto::searchview::{MainViewResponse, SearchKind}};

async fn search_tracks(
    credentials: PathBuf,
    client_id: &str,
    query: &str,
) -> anyhow::Result<Option<MainViewResponse>> {
    let Some(grant) = OAuth::restore(credentials, client_id)? else {
        return Ok(None);
    };
    grant.validate().await?;
    let session = Session::new(grant);
    Ok(Some(session.search(query, SearchKind::Track, 20).await?))
}
```

Use one [`Session`](src/session.rs) per signed-in account. The crate shares HTTP clients and refreshes the bearer when needed. Some Connect and Jam operations take a [Dealer](src/dealer.rs) connection ID.

For projected list reads, pass a set of list, entry, or referenced-list fields to `Session::get_lists`. It preserves query order and returns a separate result for each list, so one failed lookup does not discard the others. [List projections](src/proto/list_read_service.rs)

## Using the audio APIs

Resolve a context with `Session::resolve_sequence` when you need the ordered tracks for playback. After selecting an audio file from track metadata, use `Session::audio_urls` to obtain its signed URLs and [`AudioKeys`](src/access_point.rs) to get the key for the binary track and file IDs. Resolve the URLs again when they expire.

The application handles downloading, decryption, decoding, seeking, output, and offline storage. It also owns queue policy and how Connect or Jam events affect its player.

## Compatibility

The crate is developed in the Sonora workspace. Spotify can change internal routes or response formats, so validate the operations your application uses when upgrading.

Wire types and request methods live together by Spotify service. For example, Browsita owns the Browse landing page, while ListReadService reads category contents. A replacement with a different wire format can live alongside the existing service while callers migrate. [Browsita](src/proto/browsita.rs) · [ListReadService](src/proto/list_read_service.rs)

Playlist v2 owns mutations and cover registration. The JPEG upload and Connect player command each have a separate module for their HTTP formats. [Playlist v2](src/proto/playlist_v2.rs) · [Image upload](src/image_upload.rs) · [Player command](src/player_command.rs)
