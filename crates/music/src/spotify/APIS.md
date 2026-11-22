# Spotify APIs

Paths without a hostname use Spotify's regional API host. Search and Lyrics
send the Linux platform header.

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
| Metadata | `POST /extended-metadata/v0/extended-metadata` | Loads catalogue metadata, compact profiles, and Browse cards. |
| Browse | `GET /browsita/v1/browse` | Loads Search landing shelves and their Browse links. |
| Home | `GET /casita/v1/home/default` | Loads Home shelves and shortcuts. |
| Lyrics | `GET /color-lyrics/v3/track/{id}` | Loads lyrics and timing. |
| Profiles | `GET /user-profile-view/v3/profile/{user}` | Loads a profile, follower and following counts, and public playlists. |
| Saved music | `POST /collection/v2/paging` | Reads pages of saved music. |
| Library pins | `POST /collection/v2/paging` · `ylpin` | Reads the account's Spotify pins. |
| Saved music and pins | `POST /collection/v2/write` | Updates saved music and pins. |
| Playlists | `POST /playlist/spotify.list.v1.ListReadService/GetLists` | Reads selected fields from playlists, the library list, and Browse pages. |
| Playlists | `POST /playlist/spotify.list.v1.ListReadService/DiffLists` | Checks whether cached playlist revisions are still current. |
| Playlists | `GET /playlist/v2/playlist/{id}` | Reads a playlist snapshot. |
| Playlists | `POST /playlist/v2/playlist` | Creates a playlist. |
| Playlists | `POST /playlist/v2/playlist/{id}/changes` | Applies playlist edits. |
| Playlist cover registration | `POST /playlist/v2/playlist/{id}/register-image` | Registers the uploaded image and returns its picture ID. |
| Playlists | `GET /playlist/v2/user/{user}/rootlist` | Reads a page of the user's playlist library. |
| Playlists | `POST /playlist/v2/user/{user}/rootlist/changes` | Updates the playlist library. |
| Audio storage | `GET /storage-resolve/files/audio/interactive/{file-id}` | Finds signed audio download URLs. |
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

## Other HTTP formats

| Area | Endpoint or source | Format and use |
| --- | --- | --- |
| Browser sign-in | `accounts.spotify.com/authorize` and a local callback | Browser pages and an HTTP loopback callback. |
| Artwork display | `image-cdn-{fa,ak}.spotifycdn.com` and other artwork URLs | Image bytes for covers and artist images. |
| Native audio streams and downloads | Signed CDN URLs | Encrypted Vorbis audio fetched in byte ranges. |
| Short links | `spotify.link` | HTTP redirects and web pages. |

Native playback and downloads get audio keys over an authenticated Spotify
access-point TCP connection.

## Optimization priorities

| Target | What to change |
| --- | --- |
| Search | Drop the track metadata follow-up if SearchView can provide playback eligibility and a usable ranking signal. A blended query would also need to preserve each category’s result limit. |
| Playlist reads | Batch summaries and library pages through GetLists once it can preserve counts, dates, revisions, permissions, and public visibility. Visibility is available on root-list entries. |
| Playlist edits | Check whether create can set the name in one write. Use edit responses or complete Dealer updates to avoid rereading changed lists. |
| Browse and Home | Reduce the page, section, and card lookups by getting display fields in the earlier responses. Tested Home eager loading returned no metadata to reuse. |
| Connect commands | Look for a verified protobuf command route that covers playback and queue edits. The current HTTP command and Dealer delivery both use JSON. |

## WebSocket and Dealer

Sonora sends remote commands over HTTPS. Spotify delivers them to the target
device over Dealer. Sonora uses its Dealer socket to receive commands and
notifications and to send heartbeats and acknowledgements.

| Topic or connection | Payload | Use |
| --- | --- | --- |
| `hm://pusher/v1/connections` | Connection ID in envelope headers | Identifies the Dealer connection for device registration and commands. |
| `hm://connect-state/v1/cluster` | Protobuf | Reports active playback, queue, and available devices. |
| `hm://connect-state/v1/connect/volume` | Protobuf | Reports volume changes. |
| `hm://connect-state/v1/player/command` | Compressed JSON, with embedded protobuf for transfer state | Delivers playback, queue, and transfer commands to Sonora. |
| `hm://collection/collection` | Protobuf | Reports saved music changes. |
| `hm://playlist/v2/playlist` and rootlist topics | Protobuf playlist changes and rootlist notifications | Refreshes changed playlists and the library playlist list. |
| `social-connect/v2/session_update` | Protobuf or JSON | Reports Jam session changes. |
| Soloist | JSON over a local WebSocket | Controls the daemon, which manages its own upstream connections. |
