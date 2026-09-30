# Remote control APIs

Sonora can expose playback to apps on the same device over HTTP (REST) and a
WebSocket. Both transports speak the same typed schema, defined in
`crates/control`: the same commands, snapshots and events, only framed
differently.

Nothing listens until you turn it on. The server is off by default and binds
to loopback (`127.0.0.1`) only, serving both HTTP and WebSocket on one port.
Turn it on in Settings > Integrations > Remote control, where the switch
shows whether the listener is up and where, and a second switch selects
token-checked or open. Token mode requires a bearer token on every call,
reads included; open mode skips every check for clients that cannot send
headers. The token lives only in memory: quitting Sonora forgets it, turning
remote control off forgets it, and rotating it drops every open connection.
The next enable mints a fresh one.

## Concepts

**Snapshots.** Every read returns the player state at a `snapshot_revision`,
a counter the app bumps on each change. Pass a snapshot's `queue.revision`
back as `expected_revision` with queue commands; if the queue moved under
you, the command fails with `conflict` and you fetch a fresh snapshot and
retry.

**Local tracks.** A local file's id is its absolute path, so it never crosses
the API. Local tracks report `is_local: true` with no `provider_track_id`.

**Errors.** Failures use `{"v":1,"error":{"code":"...","message":"..."}}`
with a stable snake_case `code` (`invalid_argument`, `not_found`,
`conflict`, `unavailable`, `permission_denied`, `resource_exhausted`,
`internal`) and a message safe to show. REST maps codes to HTTP statuses
(400, 404, 409, 503, 403, 429, 500); unknown commands and malformed bodies
are `invalid_argument`. The WebSocket carries the same error object in
results.

## REST

Default port `47630`, configurable in settings. Requests are capped at 64 KiB.

| Method | Path           | Body                       | Reply              |
| ------ | -------------- | -------------------------- | ------------------ |
| GET    | `/v1/state`    | —                          | full snapshot      |
| GET    | `/v1/playback` | —                          | `playback` section |
| GET    | `/v1/queue`    | —                          | `queue` section    |
| POST   | `/v1/commands` | one `Command` as JSON      | `{"v":1,"accepted":true,"snapshot_revision":N}` |

In token mode, every request needs `Authorization: Bearer <token>` and, for
commands, `Content-Type: application/json`. Missing or wrong credentials
answer 401 with a `Bearer` challenge. Unknown paths answer `not_found`.
Open mode answers everything without credentials.

A command body names the command in `command` with its arguments beside it:

```json
{ "command": "play" }
{ "command": "seek", "position_ms": 90000 }
{ "command": "set_volume", "level": 0.6 }
{ "command": "set_repeat", "mode": "all" }
{ "command": "set_shuffle", "enabled": true }
{ "command": "queue_move_upcoming", "from": 0, "to": 2, "expected_revision": 17 }
```

The full set is `play`, `pause`, `toggle`, `next`, `previous`, `seek`,
`set_volume` (0.0–1.0), `set_repeat` (`off`/`all`/`one`), `set_shuffle`,
`queue_play_upcoming`, `queue_remove_upcoming`, `queue_move_upcoming` and
`queue_clear_upcoming`, each queue command carrying `expected_revision`.

```sh
curl -H "Authorization: Bearer $SONORA_TOKEN" \
  http://127.0.0.1:47630/v1/playback
curl -H "Authorization: Bearer $SONORA_TOKEN" -H 'Content-Type: application/json' \
  -d '{"command":"toggle"}' http://127.0.0.1:47630/v1/commands
```

## WebSocket

Same port as REST, path `/v1/ws`, for native clients. In token mode the upgrade
request needs `Authorization: Bearer <token>`, and any request carrying an
`Origin` header is refused, so browser pages cannot connect. Open mode
upgrades anything, origins included. Messages are text frames capped at 64
KiB. One listener serves at most 8 clients; further upgrades are refused
until one disconnects. Idle connections are pinged every 30 seconds, and a
dead one is dropped on the first failed send.

The server opens with the full snapshot:

```json
{ "type": "snapshot", "v": 1, "data": { "snapshot_revision": 42, "playback": {}, "queue": {}, "session": {} } }
```

Send commands with a client-chosen correlation `id`:

```json
{ "v": 1, "id": "req-12", "type": "command", "data": { "command": "next" } }
```

Each command gets one result echoing its `id`:

```json
{ "type": "result", "v": 1, "id": "req-12", "data": { "accepted": true, "snapshot_revision": 43 } }
{ "type": "result", "v": 1, "id": "req-13", "data": { "error": { "code": "conflict", "message": "..." } } }
```

State changes arrive as events with a broadcast `seq`:

```json
{ "type": "event", "v": 1, "seq": 7, "event": "playback.changed", "data": { "snapshot_revision": 44, "status": "playing" } }
```

The event names are `playback.changed`, `queue.changed`, `session.changed`
and `position.changed` (sampled about once a second while playing). A client
that falls behind gets a seq-less `resync.required` marker followed by a
fresh snapshot to continue from; anything unparsable gets an id-less
`{"type":"error","v":1,"data":{"error":{...}}}` and the connection stays up.

## Security model

- Off by default, loopback only, bearer token on reads and writes in token mode.
- Open mode skips every check: any local process can drive playback, and over
  WebSocket any website you visit can too. Prefer token mode; the settings
  warning says when remote control is open.
- The token is 32 random bytes, base64-encoded, kept in memory, never
  written to `settings.json` or the log. Rotation restarts the listener
  so old connections drop; disabling remote control closes its port.
- No CORS headers are served and token mode refuses browser origins, so a
  web page cannot drive or read the player, even with a leaked token.
- Responses carry no file paths, credentials, provider errors or account
  details: local ids are withheld, failures name no reason, and nothing is
  cacheable (`Cache-Control: no-store`).
- Queues, message sizes and request counts are bounded; unknown paths and
  versions fail closed.

## Limitations

- The APIs drive playback and the queue only. There is no enqueue-by-ID,
  no library or account administration, no settings access and no audio
  streaming; local tracks expose no path-bearing IDs at all.
- The listener binds loopback only, with no LAN mode, discovery or port
  forwarding. Browser clients are refused in token mode.
- The token is ephemeral: it changes on restart and whenever remote
  control has been off. Clients should ask the user to re-copy it
  after a 401, the way they would re-pair any remote.
