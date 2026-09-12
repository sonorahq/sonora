# YouTube OAuth compatibility probe

This example is an opt-in research tool. It does not participate in Sonora's login flow, does not
persist credentials, and never prints access or refresh tokens.

It uses the documented Google installed-application flow: a user-created desktop OAuth client,
PKCE, the system browser, and an ephemeral 127.0.0.1 callback. The probe uses the YouTube scope
only to test whether the resulting bearer credential is accepted by the private InnerTube calls
that Sonora needs. The TVHTML5 row is an InnerTube client-context control; it is not a return of
the removed TV OAuth implementation.

Build and run it with a Google desktop OAuth client id supplied at runtime:

~~~
export SONORA_YOUTUBE_OAUTH_CLIENT_ID='your-desktop-client-id'
export SONORA_YOUTUBE_PROBE_PLAYLIST_ID='your-owned-playlist-id'
export SONORA_YOUTUBE_PROBE_VIDEO_ID='a-video-id-to-test'
cargo run -p music --example youtube-oauth-probe
~~~

Optional identity checks compare the profile and Brand Account responses against the supplied
values. A page ID is also sent as `X-Goog-PageId` so the matrix exercises that selected identity;
`X-Goog-AuthUser` defaults to `0` and can be overridden for multi-account sessions. The probe
never stores any of these values:

~~~
export SONORA_YOUTUBE_PROBE_EMAIL='account@example.com'
export SONORA_YOUTUBE_PROBE_PAGE_ID='brand-account-page-id'
export SONORA_YOUTUBE_PROBE_AUTHUSER='0'
~~~

The matrix records the InnerTube client, case, HTTP status, whether the request returned a response
without an auth error, the optional identity match, and a compact response shape. The no-op
mutation sends an empty action list to an owned playlist so authorization can be tested without
changing its contents. A successful probe is only evidence to investigate further; it is not a
production OAuth backend or a replacement for Sonora's cookie/SAPISID session.

## Cookie-backed baseline

Before treating a probe result as useful, the current session path can be checked with the
read-only baseline smoke test. It restores the existing cookie/SAPISID credential and exercises
profile, home, search, liked songs, playlists, one playlist page, and player resolution:

~~~
export SONORA_YOUTUBE_BASELINE_PLAYLIST_ID='your-playlist-id'
export SONORA_YOUTUBE_BASELINE_VIDEO_ID='a-video-id-to-test'
cargo test -p music --lib youtube_cookie_session_covers_baseline_reads -- --ignored
~~~

The existing ignored album, artist, and playlist-privacy tests cover mutations. Account selection,
Brand Account selection, restart restoration, and sign-out still require the normal manual login
check because they intentionally cross the native browser/session boundary.
