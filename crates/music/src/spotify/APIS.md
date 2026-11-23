# Spotify APIs

Wire schemas and endpoint constants live in [spotifyapis](../../../spotifyapis/src/proto).

Relative HTTP paths use Spotify’s regional API host. Search and Lyrics send `app-platform: Linux`. Lyrics also requires a User-Agent, and Profile uses an Accept header to request protobuf.

## HTTP calls that use JSON

| Area | Endpoint | Use |
| --- | --- | --- |
| Discovery and offline recovery | `GET apresolve.spotify.com/v2?type=accesspoint&type=dealer&type=spclient` | Finds regional servers and checks whether Spotify is reachable. |
| OAuth | `POST accounts.spotify.com/api/token` | Exchanges or refreshes sign-in tokens. Sends form data and receives JSON. |
| Playlist cover upload | `POST image-upload.spotify.com/v4/playlist` | Sends a JPEG and receives an upload token. |
| Connect player commands | `POST /connect-state/v1/player/command/from/{from}/to/{to}` | Sends playback and queue commands as gzipped JSON. Forward queue jumps name the target in `skip_next`. History jumps use `play` with the selected track and its context when available. |
| Connect transfer | `POST /connect-state/v1/connect/transfer/from/{from}/to/{to}` | Moves playback between devices. |

Starting a history track can rebuild the queue from its context. A live test
kept the same context. Preservation of custom queue entries is unverified.

When Soloist owns playback, Sonora sends its history and upcoming tracks to
that device with `set_queue`. A live probe confirmed that Soloist reports
both lists without changing the current song or context.

## Protobuf HTTP calls

| Area | Endpoint | Use |
| --- | --- | --- |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext` | Reads ordered track URIs and occurrence IDs for Connect. Sonora uses them to publish the queue and find the requested track and what follows when another device sends a play command without tracks. Also reads track radio stations. |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage` | Fetches entries for page IDs returned by ResolveContext. Batches known page IDs and follows continuation pages in order when Spotify returns a chain. |
| Search | `GET /searchview/v3/search` · `entity_types=track` | Searches tracks. |
| Search | `GET /searchview/v3/search` · `entity_types=album` | Searches albums. |
| Search | `GET /searchview/v3/search` · `entity_types=artist` | Searches artists. |
| Search | `GET /searchview/v3/search` · `entity_types=playlist` | Searches playlists. |
| Metadata | `POST /extended-metadata/v0/extended-metadata` | Loads catalogue metadata, compact profiles, Browse cards, and track audio file IDs for playback and downloads. |
| Browse | `GET /browsita/v1/browse` | Supplies music categories and directory links for the flat Browse grid. |
| Home | `GET /casita/v1/home/default` | Loads Home shelves and shortcuts. |
| Lyrics | `GET /color-lyrics/v3/track/{id}` | Loads lyrics and timing. |
| Profiles | `GET /user-profile-view/v3/profile/{user}` | Loads a profile, follower and following counts, and public playlists. |
| Saved music | `POST /collection/v2/paging` | Reads pages of saved music. |
| Library pins | `POST /collection/v2/paging` · `ylpin` | Reads the account's Spotify pins. |
| Saved music and pins | `POST /collection/v2/write` | Updates saved music and pins. |
| Playlists | `POST /playlist/spotify.list.v1.ListReadService/GetLists` | Reads selected fields from playlists, the library list, and Browse sections, including playlist cards in those sections. |
| Playlists | `GET /playlist/v2/playlist/{id}` | Reads a playlist snapshot or a summary without entries. |
| Playlists | `POST /playlist/v2/playlist` | Creates a playlist. |
| Playlists | `POST /playlist/v2/playlist/{id}/changes` | Applies edits. Appends batch tracks without reading a revision. Name and cover can change together. |
| Playlist cover registration | `POST /playlist/v2/playlist/{id}/register-image` | Registers the uploaded image and returns its picture ID. |
| Playlists | `GET /playlist/v2/user/{user}/rootlist` | Reads a page of the user's playlist library. |
| Playlists | `POST /playlist/v2/user/{user}/rootlist/changes` | Updates the playlist library. Appends need no prior read. |
| Audio storage | `GET /storage-resolve/files/audio/interactive/{file-id}` | Finds signed CDN URLs for playback and downloads. |
| Connect device state | `PUT /connect-state/v1/devices/{device}` | Publishes device state and receives the account cluster. |
| Connect device state | `PUT /connect-state/v1/devices/{device}/inactive?notify=false` | Marks the device inactive. |
| Connect device state | `DELETE /connect-state/v1/devices/{device}` | Unregisters the device. |
| Connect volume | `PUT /connect-state/v1/connect/volume/from/{from}/to/{to}` | Sets another device’s volume. |
| Jam | `GET /social-connect/v2/sessions/current` | Reads the current Jam. |
| Jam | `GET /social-connect/v2/sessions/current_or_new` | Starts or resumes a Jam. |
| Jam | `GET /social-connect/v2/sessions/info/{token}` | Reads Jam invitation details. |
| Jam | `POST /social-connect/v2/sessions/join/{token}` | Joins a Jam by invitation. |
| Jam | `DELETE /social-connect/v3/sessions/{id}` | Ends a hosted Jam. |
| Jam | `POST /social-connect/v3/sessions/{id}/leave` | Leaves a Jam as a guest. |
| Jam | `POST /social-connect/v3/sessions/{id}/member/{member}/kick` | Removes a Jam member. |

## WebSocket

Sonora sends remote commands over HTTPS. Spotify delivers them to the target
device over Dealer. Sonora uses its Dealer socket to receive commands and
notifications and to send heartbeats and acknowledgements.

| Topic or connection | Payload | Use |
| --- | --- | --- |
| Dealer connection | TLS WebSocket with JSON envelopes | Opens a connection to a discovered Dealer host using OAuth, then exchanges heartbeats and acknowledgements. |
| `hm://pusher/v1/connections` | Connection ID in envelope headers | Identifies the Dealer connection for device registration and commands. |
| `hm://connect-state/v1/cluster` | Protobuf | Reports active playback, queue, and available devices. |
| `hm://connect-state/v1/connect/volume` | Protobuf | Reports volume changes. |
| `hm://connect-state/v1/player/command` | Compressed JSON, with embedded protobuf for transfer state | Delivers playback, queue, and transfer commands to Sonora. |
| `hm://collection/collection` | Protobuf | Reports saved music changes. |
| `hm://playlist/v2/playlist` | Protobuf | Refreshes changed playlists. |
| `hm://playlist/user` and `hm://playlist/v2/user/{user}/rootlist` | Protobuf | Refreshes the library playlist list when its revision changes. |
| `social-connect/v2/session_update` | JSON | Reports Jam session changes. Live start and end notifications used JSON even with protobuf HTTP replies. |
| Soloist | JSON over a local WebSocket | Controls the daemon, which manages its own upstream connections. |

## Other protocols and formats

| Area | Endpoint or source | Format and use |
| --- | --- | --- |
| Browser sign-in | `accounts.spotify.com/authorize` and a local callback | Browser pages and an HTTP loopback callback. |
| Artwork display | `image-cdn-{fa,ak}.spotifycdn.com` and other artwork URLs | Image bytes for covers and artist images. |
| Audio playback and downloads | `GET` signed audio CDN URL | Fetches encrypted Vorbis in byte ranges, including a range near the target when playback seeks ahead. |
| Native audio sign-in | Spotify access point over TCP | Uses a protobuf handshake and authenticates with the OAuth access token. |
| Audio decryption keys | Spotify access point over TCP | Requests the key for the selected track and audio file over the authenticated connection. |
| Short links | `GET spotify.link/{code}` | Follows redirects to resolve shared links. |

These audio rows describe Sonora’s native Spotify playback and downloads. Soloist manages its own Spotify connections.

## Native playback

A completed cached copy at the requested quality can be saved offline without
a catalogue, storage resolver, audio-key, or CDN request. Cold playback and
downloads still use TRACK_V4, the storage resolver, access-point audio keys,
and ranged Vorbis downloads.

## Optimization priorities

Prioritize time to the first usable content and time until the page is ready.
Shorten chains of dependent requests, run independent requests concurrently,
and show results as they arrive when that keeps the page stable. Use the
[benchmarks](BENCHMARKS.md) to track those timings, request counts, and transfer
sizes. Smaller responses matter most when they improve load times or materially
reduce traffic. Prefer a single proven route. Keep an alternate path only when
observed responses or protocol behavior show why it is needed. Preserve playback
behavior and check that each page still serves its purpose. Different shelves
or result counts can be acceptable.

🟢 Implementation complete, only verification remains · 🟠 Partly implemented or a concrete implementation lead · ⚪ Needs a viable approach

| Status | Target | Benchmark signal | What to change |
| --- | --- | --- | --- |
| ⚪ | Saved-track metadata size | TRACK_V4 alone uses fewer bytes and omits Genre, Plays, mood, and BPM. | Find a narrower projection that preserves the metadata and sorting Sonora uses. |
| 🟠 | Browse section reads | A ten-entry GetLists window returned the exact first ten entries for all sampled long shelves. Pop returned 90 of 222 entries and Made For You 63 of 103. | Use a bounded window for shelves identified as media-only. Confirm the identifying fields across more pages and keep full reads for mixed shelves. |
| 🟠 | Artist catalogue delivery | Mozart returned 195 preview releases in 159 ms and all 1,867 in 537 ms. Bach hit an HTTP/2 response-body error on two runs. | Reproduce and fix the large-catalogue transport failure. |
| ⚪ | Album detail | AlbumV4 returned only IDs for all 43 sampled track entries. | Find an album response with full track metadata and availability before removing hydration. |
| 🟠 | Playlist reads | One GetLists call returned four playlist summaries in 97 ms with matching revisions, names, descriptions, owners, timestamps, and track counts. | Batch summary refreshes through GetLists after checking permission values and artwork URLs. Verify public visibility before replacing the root-list read. |
| 🟠 | Playlist notification refreshes | Both notifications arrived after overlapping renames of two temporary playlists. | Count the app’s summary, root-list, and pin reads while a refresh is already in flight. |
| ⚪ | Playlist edit deltas | Sonora still reads a snapshot after a playlist edit notification. | Capture real notifications and check whether they carry the parent revision, ordered operations, and truncation flag needed to apply a complete delta. |
| ⚪ | Search request count | Five calls and 80 results in the benchmark. The rewrite downloaded 14.9 KiB versus 406.5 KiB upstream. | Verify availability and a ranking signal before dropping track hydration. A combined query still needs independent category limits. |

### Saved tracks and albums

The mixed collection read preserved the IDs, order, and saved dates of all
2,391 tracks and 17 albums. It used eight requests versus ten for separate
reads. The first 500 tracks arrived in 272–276 ms in two runs. The full read
took 510–685 ms. A cold desktop pass showed saved songs with dates.

On the same 500 tracks, TRACK_V4 alone used 430,239 decoded response bytes
versus 1,088,267 with the four current extensions. It preserved the track
payload and cover availability for all 500. It would lose data used by the
Genre and Plays columns, as well as mood and BPM. Sonora now emits fully
hydrated batches of 500 in collection order. A narrower projection needs to
preserve sorting and details for the whole library.

### Browse and Home

The Browse root gives an ordered list of dynamic section URIs. GetLists
`meta_items` projects attributes of those references, so it cannot return
their nested entries in the same request. Casita returned no reusable
metadata or refresh token with the tested eager and incremental options.
Keep both page and section reads until a response supplies the missing data.
The current UI keeps at most ten media cards per shelf and preserves every
category link. In one paired GetLists probe with the same playlist-card
projection, Pop’s section read went from 175 to 113 ms and Made For You’s
from 262 to 123 ms. The long shelves sampled contained media only. Use a
stable shelf classification before applying the limit to other pages.
Pop still downloads 119.6 KiB
versus 92.0 KiB upstream for the same 90 items. Browse landing uses two calls
versus one upstream and downloads 3.4 KiB versus 30.4 KiB.

Home emits the main feed and then appends Made For You. In five fresh
provider sessions per build, Made For You supplied all 42 supplemental cards
through its section projection. The sixth request came from splitting
Casita’s 109 playlist cards into two batches at a limit of 100. A 128-card
batch returned all 128 results in a separate probe. Across the five Home
loads, median time to the first feed fell from 454 to 398 ms and time to the
full feed fell from 454 to 399 ms. Both returned 178 main cards and 220 total.

### Artist rankings

Artist rankings use ArtistV4. It supplied one usable ranking for 161 of 166
sampled artists on one account. The former SequenceProxy fallback returned
ten ranked tracks for Nusrat Fateh and no context for the four other artists
whose ArtistV4 replies had no ranking. An artist with no agreed ranking has
no ranked top tracks. The artist’s playable discography is still available.

### Search hydration

SearchView's track schema exposes availability candidates such as
`content_access`, `windowed`, `premium_only`, and `gating_decision`. Earlier
samples contained only playable tracks. Compare explicit denials, restricted
tracks, and market relinks to TRACK_V4 before using those fields to decide
whether a row can play. SearchView has no known popularity field. Compare
ranking against the current results or decide explicitly to use Spotify's
ranked order before removing hydration.

The APK's `/searchview/v3/native` returns a different response with snippet and
recent-search rows. Decode it with its own schema when evaluating it. A
replacement for the four concurrent searches needs independent result limits
and the artist and album fields used by Sonora. Broad queries such as “love”
and “jazz” exposed category underfill in the blended probes.

### Playlist reads

The root-list GetLists projection returns visibility from `items.attributes`
with URI field 1 and `PublishedToProfile` field 15. A prior probe matched the
26 private entries in the current root-list response. Verify a public playlist
owned by the signed-in account before switching. Visibility was absent from
standalone playlist metadata.

GetLists with a zero-entry window returned four owned playlist summaries in
one request. Each matched the snapshot’s revision, name, description, owner,
update time, count, and picture file ID. Capabilities and picture-size
presence also matched. Compare capability values and picture URLs before
switching. The root-list projection reported an explicit private value for
all 26 playlists. The account had no owned public playlist to verify.

### Playlist edit notifications

- Playlist notifications carry a new revision (field 2), parent (3), operations (4), and truncation flag (7). Apply the ordered batch only when the cached revision matches the parent, the new revision is present and different, and every operation is supported. Preserve item positions and duplicates. Name and cover changes can refresh the header without hydrating tracks again. Missing or unsupported deltas still need a snapshot.
