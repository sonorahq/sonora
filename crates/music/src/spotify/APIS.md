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

Soloist is a separate Connect device. Sonora sends it the same playback
commands it sends to other devices and reads the queue Spotify reports.

## Protobuf HTTP calls

| Area | Endpoint | Use |
| --- | --- | --- |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext` | Reads occurrence IDs when publishing a Connect queue, resolves play commands whose track or queue cannot be reconstructed locally, and reads track radio stations. |
| Playback contexts | `POST /sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage` | Fetches entries for page IDs returned by ResolveContext. Batches known page IDs and follows continuation pages in order when Spotify returns a chain. |
| Search | `GET /searchview/v3/search` · `entity_types=track` | Searches tracks. |
| Search | `GET /searchview/v3/search` · `entity_types=album` | Searches albums. |
| Search | `GET /searchview/v3/search` · `entity_types=artist` | Searches artists. |
| Search | `GET /searchview/v3/search` · `entity_types=playlist` | Searches playlists. |
| Metadata | `POST /extended-metadata/v0/extended-metadata` | Loads catalogue metadata, compact profiles, Home and Browse cards, and track audio file IDs for playback and downloads. |
| Browse | `GET /browsita/v1/browse` | Supplies music categories and directory links for the flat Browse grid. |
| Home | `GET /casita/v1/home/default` | Loads Home shelves and shortcuts. |
| Lyrics | `GET /color-lyrics/v3/track/{id}` | Loads lyrics and timing. |
| Profiles | `GET /user-profile-view/v3/profile/{user}` | Loads a profile, follower and following counts, and public playlists. |
| Saved music | `POST /collection/v2/paging` | Reads pages of saved music. |
| Library pins | `POST /collection/v2/paging` · `ylpin` | Reads the account's Spotify pins. |
| Saved music and pins | `POST /collection/v2/write` | Updates saved music and pins. |
| Playlists | `POST /playlist/spotify.list.v1.ListReadService/GetLists` | Reads playlist details, dated entries and contributors, library pages, and Browse sections. Also batches summaries. |
| Playlists | `POST /playlist/v2/playlist` | Creates a playlist. |
| Playlists | `POST /playlist/v2/playlist/{id}/changes` | Applies edits. Appends batch tracks without reading a revision. Name and cover can change together. |
| Playlist cover registration | `POST /playlist/v2/playlist/{id}/register-image` | Registers the uploaded image and returns its picture ID. |
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
