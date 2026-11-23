# Spotify APIs

Wire schemas and endpoint constants live in [spotifyapis](../../../spotifyapis/src/proto).

Relative HTTP paths use Spotify’s regional API host. Search and Lyrics send `app-platform: Linux`. Lyrics also requires a User-Agent, and Profile uses an Accept header to request protobuf.

## HTTP calls that use JSON

| Area | Endpoint | Use |
| --- | --- | --- |
| Discovery and offline recovery | `GET apresolve.spotify.com/v2?type=accesspoint&type=dealer&type=spclient` | Finds regional servers and checks whether Spotify is reachable. |
| OAuth | `POST accounts.spotify.com/api/token` | Exchanges or refreshes sign-in tokens. Sends form data and receives JSON. |
| Playlist cover upload | `POST image-upload.spotify.com/v4/playlist` | Sends a JPEG and receives an upload token. |
| Connect player commands | `POST /connect-state/v1/player/command/from/{from}/to/{to}` | Sends playback and queue commands to another device as gzipped JSON. |
| Connect transfer | `POST /connect-state/v1/connect/transfer/from/{from}/to/{to}` | Moves playback between devices. |

## Protobuf HTTP calls

| Area | Endpoint | Use |
| --- | --- | --- |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext` | Resolves Connect queues, track radio, and artist rankings. |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage` | Loads one or more sequence pages by ID. |
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
| `social-connect/v2/session_update` | JSON observed, protobuf accepted | Reports Jam session changes. Live start and end notifications used JSON even with protobuf HTTP replies. |
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

Prioritize fewer requests and shorter chains of dependent calls, especially on
frequently used pages. Prefer API routes that can replace several endpoints,
schemas, or fallback paths and meaningfully shrink the code we maintain. Use
the [benchmarks](BENCHMARKS.md) to track load time and bytes in both directions.
Preserve playback behavior, metadata, and result coverage when comparing routes.
The latest provider pass used 76 requests versus 192 upstream. Each
comparison used one run per build.

🟢 Implemented, verification pending · ⚪ Open

| Status | Target | Benchmark signal | What to change |
| --- | --- | --- | --- |
| 🟢 | Saved tracks and albums | Larger pages cut Liked Songs from 14 to 7 calls and saved albums from 10 to 3. A full library load now shares the mixed collection read. | Reduce catalogue hydration or render the first page earlier while preserving saved dates and availability. The API pass verifies the individual reads. The mixed walk still needs a separate measurement. |
| 🟢 | Browse and Home delivery | Pop and Made For You take three calls each. Home can show its main shelves before Made For You finishes, but the sampled feed arrived just 1 ms earlier and used six calls instead of five. | Share the two playlist-card lookups without delaying the main shelves. |
| ⚪ | Browse section reads | Spotify returns section references first, then their entries. | Find a response with both, or bound the payload for shelves known to contain only media. Preserve the full category directory. |
| ⚪ | Catalogue detail | AlbumV4 returned only IDs for all 43 sampled track entries. Large artist catalogues still wait for all release batches. | Find an album response with full tracks and measure earlier batches on large artist catalogues. |
| ⚪ | Playlist reads | A single summary takes one request. Four direct reads replace one batched revision check for four unchanged summaries. | Batch summaries and library pages through GetLists once its field coverage matches. |
| ⚪ | Playlist edits | Outside the read-only benchmark. Live tests verified batched appends and a combined name-and-cover write. | Avoid broad refreshes for root-list events and overlapping targeted reads. Check whether complete Dealer updates can replace local edit refreshes. |
| ⚪ | Search | Five calls both ways and 80 results in the benchmark. The rewrite downloaded 14.9 KiB versus 406.5 KiB upstream. | Let category tabs show results as their queries finish, while keeping the combined ranking stable and discarding old query results. Drop track hydration only after verifying availability and a ranking signal. A combined query still needs independent category limits. |

### Saved tracks and albums

One collection walk returned 2,391 tracks and 17 albums, with saved dates on
every track. A full library load now splits one walk by URI and hydrates the
two lists independently, preserving their order and saved dates. For that
account this should cut collection paging from four calls to two.

On the same 500 tracks, TRACK_V4 alone used 430,239 decoded response bytes
versus 1,088,267 with the four current extensions. It preserved the track
payload and cover availability for all 500. It would lose data used by the
Genre and Plays columns, as well as mood and BPM. Start by emitting the first
fully hydrated 100 tracks through the existing paged library interface. That
batch took 56 ms after the IDs were known. Compare time to first rows and
total transfer, since smaller batches may cost more requests. A narrower
projection needs a way to preserve sorting and details for the whole library.

### Browse and Home

The Browse root gives an ordered list of dynamic section URIs. GetLists
`meta_items` projects attributes of those references, so it cannot return
their nested entries in the same request. Casita returned no reusable
metadata or refresh token with the tested eager and incremental options.
Keep both page and section reads until a response supplies the missing data.
Test a bounded item window on proven media-only shelves. The current UI
keeps at most ten media cards per shelf and preserves every category link,
including links beyond the first ten entries. Pop still downloads 119.6 KiB
versus 92.0 KiB upstream for the same 90 items. Browse landing uses two calls
versus one upstream and downloads 3.4 KiB versus 30.4 KiB.

Progressive delivery here means Sonora emits results from existing requests
as they finish. The routes tested so far return complete protobuf responses.
Home emits the main feed while Made For You is still loading, then appends
new shelves in order. The second pass reuses cards from the first. Compare
first usable content, total calls, and total bytes separately.

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

A warm summary probe returned 372 decoded bytes from GetLists with or without
the current revision. It requested URI, name, description, update time,
owner, capabilities, approximate count, artwork, and format. This measured
response shape and size. It did not establish field-by-field parity. The
legacy summary snapshot returned 359 bytes, so the case for GetLists rests
on batching multiple summaries and preserving all the fields Sonora uses.

### Playlist edit notifications

- Give root-list notifications a playlist-specific event so they reload the playlist list and reconcile pins. The generic library event currently reloads saved tracks, albums, and artists too. Keep the revision match that already skips duplicate events.
- Keep a targeted summary request running when another notification arrives. Queue the later playlist IDs for one targeted follow-up. Today an overlap can turn a targeted refresh into a full playlist-list read.
- Playlist notifications carry a new revision (field 2), parent (3), operations (4), and truncation flag (7). Apply the ordered batch only when the cached revision matches the parent, the new revision is present and different, and every operation is supported. Preserve item positions and duplicates. Name and cover changes can refresh the header without hydrating tracks again. Missing or unsupported deltas still need a snapshot.

## Extracting spotifyapis

The [spotifyapis crate](../../../spotifyapis/src) owns the wire types, OAuth, discovery, the shared HTTP clients, Dealer, and the service requests. The goal is to publish it as a standalone Rust alternative to librespot.

- Keep Spotify request construction and Dealer framing in spotifyapis. Typed service methods return Spotify’s protobuf responses for Sonora to interpret. Keep Connect and Jam orchestration in the Sonora adapter until any reusable behavior has a clear public API.
- Move native media primitives into spotifyapis, including range reads and access-point key exchange. Give callers the primitives needed to use their own decoder, output, and download storage.
- Keep Sonora’s provider traits, model conversion, playback engine and seek integration, Soloist management, and app storage in `music/spotify` or the shared Sonora modules they already use.
- Keep the crate independent of Sonora’s paths, database, UI, and playback policy. OAuth takes a credential path and lets the caller open the browser.
- Move one service at a time and verify Sonora’s requests and playback behavior after each move. Before spinning out the repository, verify the crate builds and can be used without the Sonora workspace.
