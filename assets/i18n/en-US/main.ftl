# common
common-on = On
common-off = Off
common-left = Left
common-right = Right
common-search = Search
common-unknown = Unknown
common-not-provided = Not provided
common-not-available = Not available
common-cancel = Cancel
common-save = Save
common-delete = Delete
common-play = Play
common-more = More
common-previous = Previous
common-next = Next
common-dismiss = Dismiss
common-clear = Clear
number-group = { "," }

# navigation
nav-history = History
nav-home = Home
nav-search = Search
nav-library = Your Library
nav-settings = Settings
nav-songs = Songs
nav-albums = Albums
nav-playlists = Playlists
nav-artists = Artists
nav-local = Local Music
nav-back = Back
nav-forward = Forward
nav-sidebar = Toggle sidebar
nav-sidebar-right = Show or hide lyrics and queue
nav-pinned = Pinned
nav-unpin = Unpin
nav-pin-hint = Drop here to pin
library-liked-songs = Favorites
library-play-liked-songs = Play
library-no-songs = No favorites yet
library-no-albums = No saved albums yet
library-no-playlists = No playlists yet
library-no-artists = No favorite artists yet
library-no-local-songs = No imported songs found
library-no-local-albums = No imported albums found
library-no-local-artists = No imported artists found
library-no-local-playlists = No local playlists yet
library-no-catalog-songs = No songs found
library-no-catalog-albums = No albums found
library-no-catalog-artists = No artists found
library-no-matches = No matches
library-not-loaded = Your library did not load
library-scanning = Looking through your folders…
library-scanning-found = Looking through your folders… { $found } so far
library-scanning-progress = Reading your music… { $read } of { $found }
library-part-not-loaded = This part of your library did not load
library-local-unconfigured = Configure your local library

# app menu
app-refresh-library = Refresh Library
app-sign-out = Sign Out
app-quit = Quit
app-settings = Settings…
app-hide = Hide Sonora
app-hide-others = Hide Others
app-show-all = Show All
app-edit = Edit
app-cut = Cut
app-copy = Copy
app-paste = Paste
app-select-all = Select All
app-window = Window
app-close-window = Close Window
app-minimize = Minimize
app-zoom = Zoom

# tray menu
tray-show = Show Sonora
tray-play = Play
tray-pause = Pause

# table columns
column-played-at = Played
column-index = #
column-title = Title
column-artist = Artist
column-album = Album
column-date-added = Date added
column-added-by = Added by
column-modified = Modified
column-length = Length
column-plays = Plays
column-name = Name
column-owner = Owner
column-year = Year
column-tracks = Tracks

# track menu
menu-add-to-playlist = Add to playlist
menu-add-tracks-to-playlist = { $count ->
    [one] Add { $count } track to playlist
   *[other] Add { $count } tracks to playlist
}
menu-new-playlist = New playlist
menu-edit-tags = Edit tags
menu-no-playlists = No playlists
menu-search-playlists = Search a playlist
menu-add-to-library = Add to Favorites
menu-add-tracks-to-library = { $count ->
    [one] Add { $count } track to Favorites
   *[other] Add { $count } tracks to Favorites
}
menu-remove-from-library = Remove from Favorites
menu-remove-tracks-from-library = { $count ->
    [one] Remove { $count } track from Favorites
   *[other] Remove { $count } tracks from Favorites
}
menu-remove-from-playlist = Remove from playlist
menu-remove-tracks-from-playlist = { $count ->
    [one] Remove { $count } track from playlist
   *[other] Remove { $count } tracks from playlist
}
menu-remove-from-history = Remove from history
menu-remove-tracks-from-history = { $count ->
    [one] Remove { $count } track from history
   *[other] Remove { $count } tracks from history
}
menu-delete-track-file = Delete track file
menu-delete-track-files = Delete { $count } track files
menu-play-next = Play next
menu-play-tracks-next = { $count ->
    [one] Play { $count } track next
   *[other] Play { $count } tracks next
}
menu-add-to-queue = Add to queue
menu-add-tracks-to-queue = { $count ->
    [one] Add { $count } track to queue
   *[other] Add { $count } tracks to queue
}
menu-play-last = Play last
menu-play-tracks-last = { $count ->
    [one] Play { $count } track last
   *[other] Play { $count } tracks last
}
menu-song-radio = Go to song radio
menu-go-to-album = Go to album
menu-go-to-artist = Go to artist
menu-view-details = View details
menu-copy-link = Copy link
menu-cut = Cut
menu-copy = Copy
menu-paste = Paste
menu-select-all = Select all
menu-remove-from-queue = Remove from queue
menu-open-playlist = Open playlist
menu-play-playlist = Play playlist
menu-rename-playlist = Rename playlist
menu-delete-playlist = Delete playlist
menu-add-playlist-to-library = Add to Library
menu-remove-playlist-from-library = Remove from Library
menu-library-add = Add to Library
menu-library-add-tracks = { $count ->
    [one] Add { $count } track to Library
   *[other] Add { $count } tracks to Library
}
menu-library-remove = Remove from Library
menu-library-remove-tracks = { $count ->
    [one] Remove { $count } track from Library
   *[other] Remove { $count } tracks from Library
}
menu-make-playlist-public = Make public
menu-make-playlist-private = Make private
menu-open-album = Open album
menu-play-album = Play album
menu-play-artist = Play artist

# playlist editor
playlist-name-placeholder = Playlist name
playlist-create-title = Create playlist
playlist-rename-title = Rename playlist
playlist-delete-title = Delete playlist
playlist-delete-confirm = Delete “{ $name }”? This cannot be undone.
playlist-again-title = Add it again?
playlist-again-confirm = This track is already in “{ $name }”. Add another copy?
playlist-again-add = Add again

# confirm
confirm-remove-library-title = Remove from library
confirm-remove-playlist-title = Remove from playlist
confirm-remove-history-title = Remove from history
confirm-remove-songs = { $count ->
    [one] Remove this song from your library?
   *[other] Remove { $count } songs from your library?
}
confirm-remove-playlist-songs = { $count ->
    [one] Remove this song from the playlist?
   *[other] Remove { $count } songs from the playlist?
}
confirm-remove-history-songs = { $count ->
    [one] Remove this song from listening history?
   *[other] Remove { $count } songs from listening history?
}
confirm-remove-albums = { $count ->
    [one] Remove this album from your library?
   *[other] Remove { $count } albums from your library?
}
confirm-remove-artists = { $count ->
    [one] Remove this artist from Favorites?
   *[other] Remove { $count } artists from Favorites?
}
confirm-remove-playlists = { $count ->
    [one] Remove this playlist from your library?
   *[other] Remove { $count } playlists from your library?
}
confirm-delete-track-files-title = Delete track files?
confirm-delete-track-files = The file(s) will be permanently deleted from disk. This cannot be undone.

# queue panel
queue-title = Queue
queue-history = History
queue-now-playing = Now playing
queue-from = From
queue-next-in-queue = Next in queue
queue-up-next = Up next
queue-reset = Reset
queue-clear = Clear
queue-empty = Your queue is empty
queue-similar = Similar tracks
queue-radio = Autoplay similar tracks
queue-return-playing = Back to now playing

# player bar
player-nothing-playing = Nothing playing
player-percent = { $value }%
player-shuffle = Shuffle
player-repeat = Repeat
player-repeat-all = Repeat all
player-repeat-one = Repeat one
player-mute = Mute
player-unmute = Unmute
player-previous = Previous track
player-next = Next track
player-fullscreen = Fullscreen
player-fullscreen-leave = Leave fullscreen
fullscreen-artwork = Artwork

# filters
filter-history = Filter listening history
history-empty = Tracks you play will appear here.
history-not-loaded = Listening history could not be loaded.
history-clear = Clear history
history-clear-title = Clear listening history
history-clear-confirm = Every play is removed from this device. This cannot be undone.
filter-library = Filter your library
filter-album = Filter album tracks
filter-reset = Reset filters
filter-duration = Duration
filter-year = Year
filter-explicit = Explicit only
filter-playable = Playable only
filter-favorites = Favorites only
filter-owned = By you

# view
view-list = List
view-cards = Grid

# toolbar
tool-columns = Columns
tool-sort = Sort
tool-filters = Filters

# login
login-signed-out = Sign in to load your music library
login-restoring = Checking your saved session…
login-authorizing = Waiting for authorization in your browser…
login-signed-in = Signed in as { $name }
login-failed-title = Sign-in failed
login-problem-region = Spotify will not open a session from the country you are in. Sign in from your home country, or change the country on your Spotify account.
login-problem-credentials = Your saved Spotify session is no longer valid. Sign in again to continue.
login-problem-network = Sonora could not reach Spotify. Check your internet connection and try again.
login-problem-cancelled = You closed the browser page before approving the sign-in. Start again to finish.
login-problem-refused = Spotify turned down the sign-in. Wait a moment and try again.
login-problem-premium = Sonora streams through Spotify Premium, and this account does not have it. Sign in with a Premium account to continue.
login-sign-in = Sign in with { $provider }
login-connect-cookies = Paste cookies manually
login-cookie-open = Open { $provider }
login-cookie-submit = Continue
login-cookie-hint = Paste the Cookie request header here
login-cookie-step-1 = Open music.youtube.com and make sure you are signed in. An incognito window works best.
login-cookie-step-2 = Press F12, open the Network tab and reload the page.
login-cookie-step-3 = Select any request named "browse" or "next".
login-cookie-step-4 = In Headers, find Cookie under Request Headers, right-click it and copy its value.
login-cookie-step-note = Make sure to paste the whole value, including SAPISID and __Secure-3PAPISID.
login-cookie-header-title = Paste your { $provider } cookies to finish signing in
login-cookie-apple-step-3 = Select any request to { $site }.
login-cookie-apple-note = Make sure to paste the whole value, including { $cookie }.
login-cookie-named-title = Paste your { $provider } cookie to finish signing in
login-cookie-named-hint = Paste the cookie value here
login-cookie-named-step-1 = Open { $site } and make sure you are signed in.
login-cookie-named-step-2 = Press F12 and open the Storage tab in Firefox, or the Application tab in Chrome.
login-cookie-named-step-3 = Expand Cookies, select { $site } and find the cookie named { $cookie }.
login-cookie-named-step-4 = Double-click its value and copy it.
login-cookie-named-note = The { $cookie } value alone is enough. A whole Cookie header copied from the Network tab works too.
login-window-title = Sign in to { $provider }
login-use = Use { $provider }
login-guest-title = Guest mode
login-guest-use = Use Guest mode
login-guest-detail = Browse and play without an account. Your library, likes and playlists stay out of reach.
login-usage-consent = Help us estimate how many people use Sonora.
login-device-code = Enter this code at { $url }
login-server-title = Connect to your Subsonic server
login-server-detail = Enter the address of any Subsonic or OpenSubsonic server (Navidrome, Airsonic, Gonic, …), then sign in with your server username and password. The session stays on this device.
login-server-hint = https://music.example.com
login-username-hint = Username
login-password-hint = Password
login-server-submit = Connect
login-account-title = Choose an account
login-account-detail = This session is signed in to more than one Google account. Pick the one Sonora should use.
login-choose-title = Sign in to { $provider }
login-choose-detail = Choose how to sign in to { $provider }.

# album and playlist pages
detail-album = Album
detail-playlist = Playlist
detail-play-album = Play album
detail-play-playlist = Play playlist

# play button
play-pause = Pause
play-resume = Resume
play-loading = Loading…
play-shuffle = Shuffle

# artist page
artist-eyebrow = Artist
artist-monthly-listeners = { $count ->
    [one] { $value } monthly listener
   *[other] { $value } monthly listeners
}
artist-play = Play now
artist-popular = Popular
artist-popular-eyebrow = Explore this artist
artist-popular-empty = Nothing to play from this artist yet
artist-popular-more = Show all
artist-popular-less = Show less
artist-releases = Releases
artist-releases-more = Show all
artist-releases-less = Show less
artist-filter-all = All
artist-filter-albums = Albums
artist-filter-singles = Singles
artist-filter-eps = EPs

# user profile page
user-eyebrow = Profile
user-followers = { $count ->
    [one] { $value } follower
   *[other] { $value } followers
}
user-following = { $count ->
   *[other] { $value } following
}
user-playlists = Public playlists
user-playlists-empty = No public playlists yet

# release kinds
release-album = Album
release-single = Single
release-compilation = Compilation
release-ep = EP
release-audiobook = Audiobook
release-podcast = Podcast
release-meta = { $year } • { $kind }

# home page
home-quick-picks = Quick picks
home-quick-picks-empty = Like a few songs and they will show up here

# search page
search-placeholder = What do you want to listen to?
search-browse = Browse all
genre-empty = Nothing to show here yet
search-best-match = Best match
search-no-matches = No matches
search-results = Results
search-songs = Songs
search-artists = Artists
search-albums-playlists = Albums & playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } song in Library
       *[other] { $count } songs in Library
    }
kind-song = Song
kind-artist = Artist
kind-album = Album
kind-playlist = Playlist

# song page
song-eyebrow = Song
song-play = Play song
song-view-album = View album
song-loading = Loading song information…
song-about = About this song
song-album = Album
song-released = Released
song-streams = Streams
song-position = Position
song-label = Label
song-popularity = Popularity
song-popularity-value = { $value }%
song-disc-track = Disc { $disc }, track { $track }
song-track = Track { $track }
song-credits = Credits
song-performed-by = Performed by
song-details = Genres & details
song-genres = Genres
song-language = Language
song-content = Content
song-explicit = Explicit
song-clean = Clean
artist-about = About the artist
artist-about-fallback = Explore the artist's popular songs and releases.
artist-about-open = Go to artist
song-copyright = © { $notice }

# song languages
language-ar = Arabic
language-de = German
language-en = English
language-es = Spanish
language-fr = French
language-hi = Hindi
language-it = Italian
language-ja = Japanese
language-ko = Korean
language-pt = Portuguese
language-ru = Russian
language-tr = Turkish
language-uk = Ukrainian
language-zh = Chinese
language-zxx = No linguistic content

# counts
count-songs =
    { $count ->
        [one] { $count } song
       *[other] { $count } songs
    }
count-tracks =
    { $count ->
        [one] { $count } track
       *[other] { $count } tracks
    }

# dates
date-just-now = Just now
date-minute-ago = A minute ago
date-minutes-ago = { $count } minutes ago
date-today = Today at { $time }
date-yesterday = Yesterday at { $time }
date-time = { $date }, { $time }
date-full = { $month } { $day }, { $year }
month-1 = Jan
month-2 = Feb
month-3 = Mar
month-4 = Apr
month-5 = May
month-6 = Jun
month-7 = Jul
month-8 = Aug
month-9 = Sep
month-10 = Oct
month-11 = Nov
month-12 = Dec

# settings
settings-search = Search settings
settings-tab-general = General
settings-tab-appearance = Appearance
settings-tab-playback = Playback
settings-tab-privacy = Privacy
settings-tab-integrations = Integrations
settings-theme = Theme
settings-theme-detail = Choose the application colour palette
settings-opacity = Opacity
settings-opacity-detail = Adjust the app background opacity
settings-opacity-value = { $percent }%
settings-theme-config = Open config
settings-adaptive = Adaptive theme
settings-adaptive-detail = Tint the palette with the artwork of the playing album
settings-ambient = Ambient background
settings-ambient-detail = Fill the fullscreen player with colours drawn from the cover
settings-ambient-motion = Ambient motion
settings-ambient-motion-detail = Drift the ambient colours instead of holding them still
settings-stage = Stage
settings-stage-detail = How fullscreen stages the cover
settings-stage-style-cover = Cover only
settings-stage-style-starry = Starry
settings-stage-style-vinyl = Spinning vinyl
settings-stage-style-sleeve = Sleeve and vinyl
settings-particles = Drifting particles
settings-particles-detail = How many particles drift off the cover
settings-particles-value = { $count }
settings-visualizer = Visualizer
settings-visualizer-detail = How the spectrum is drawn behind fullscreen artwork
settings-visualizer-style-none = Off
settings-visualizer-style-bars = Bars
settings-visualizer-style-wave = Wave
settings-visualizer-style-both = Bars and wave
settings-fullscreen-controls-autohide = Hide fullscreen controls
settings-fullscreen-controls-autohide-detail = Fade out playback controls when fullscreen is inactive
settings-icons = Icon pack
settings-icons-detail = Choose the icon set the interface draws from
settings-motion = Reduce motion
settings-motion-detail = Skip interface animations and transitions
settings-pace = Animation speed
settings-pace-detail = How fast interface animations play
settings-saver = Battery saving
settings-saver-detail = Cap the frame rate of animations while Sonora is not focused, applied from the next launch
settings-corners = Corners
settings-corners-detail = How rounded surfaces and controls are
settings-blur = Blur UI
settings-blur-detail = Frost the menus and floating controls over whatever they cover
settings-blur-window = Blur window
settings-blur-window-detail = Draw the window over a blurred desktop. Needs an opacity below 100%
settings-font = Font size
settings-font-detail = Base text size, everything else scales with it
settings-font-value = { $size } px
settings-startup = Show on startup
settings-startup-detail = The screen Sonora opens on launch
settings-entries = Sidebar entries
settings-entries-detail = The sections listed in the sidebar
settings-entries-pick = Choose entries
settings-language = Language
settings-language-detail = The language Sonora uses across the interface
settings-language-system = System
settings-language-search = Search a language
settings-language-none = No languages found
settings-typeface = Font
settings-typeface-detail = The typeface Sonora uses across the interface
settings-typeface-system = Default
settings-typeface-search = Search a font
settings-typeface-none = No fonts found
settings-server-side-decorations = Server-side decorations
settings-server-side-decorations-detail = Let the compositor draw the title bar, border and shadow
settings-typeface-loading = Loading…
settings-window-controls = Window controls
settings-window-controls-detail = Draw minimise, maximise and close in the title bar
settings-traffic-light-controls = Traffic light controls
settings-traffic-light-controls-detail = Draw minimise, maximise and close as colored dots
settings-window-rounding = Window corners
settings-window-rounding-detail = How rounded the window's own corners are
settings-controls-side = Controls side
settings-controls-side-detail = Which end of the title bar the controls sit on
settings-close-to-tray = Keep playing when closed
settings-close-to-tray-detail = Keep Sonora in the system tray and continue playing after its window closes
settings-discord = Show on Discord
settings-discord-detail = Put the track you are playing on your Discord profile
settings-discord-name = Status name
settings-discord-name-detail = What the status is called after the "Listening to" that your friends see
settings-discord-name-sonora = Sonora
settings-discord-name-provider = Provider
settings-discord-name-music = Music
settings-discord-name-title = Title
settings-discord-name-artist = Artist
settings-discord-name-artist-title = Artist - Title
settings-discord-show-paused = Show when paused
settings-discord-show-paused-detail = Keep the status on your Discord profile while the track is paused
settings-discord-badge = Show the provider badge
settings-discord-badge-detail = Mark the status with a small icon of the service the track came from
settings-discord-anonymous = Hide details
settings-discord-anonymous-detail = Say only that music is playing, without the title, artist or artwork
settings-discord-buttons = Buttons
settings-discord-buttons-detail = Links under the status for your friends to open, one to the track on its service and one to Sonora
settings-discord-buttons-pick = Choose buttons
# the Discord status when the track is left out of it
discord-listening = Listening to music
get-sonora = Get Sonora
settings-fullscreen-controls-autohide-automatic = Automatic
settings-fullscreen-controls-autohide-always-hidden = Always hidden
settings-fullscreen-controls-autohide-always-shown = Always shown
settings-normalisation = Normalize loudness
settings-normalisation-detail = Keeps tracks at a consistent volume
settings-gapless = Gapless playback
settings-gapless-detail = Runs one track into the next without a pause, the way an album was sequenced
settings-sleep = Sleep timer
settings-sleep-detail = Lets the music stop on its own after a set time, so it can play you to sleep
settings-sleep-configure = Configure…
settings-sleep-off = Off
settings-sleep-end-of-track = End of track
settings-sleep-minutes = { $count } mins
settings-widevine = Widevine module
settings-widevine-detail = Apple Music tracks are encrypted and need Google's Widevine module. Sonora uses the copy a browser here already has, or one it downloaded from Google with your consent.
settings-widevine-none = Apple Music tracks are encrypted and need Google's Widevine module. No browser here has one, so Sonora can download it from Google with your consent.
settings-widevine-looking = Looking…
settings-widevine-asking = Waiting for your answer
settings-widevine-fetching = Downloading…
settings-widevine-installing = Installing…
settings-widevine-installed = Found in a browser
settings-widevine-fetched = Downloaded from Google
settings-widevine-missing = Not installed
settings-widevine-configured = Set by the environment
settings-widevine-fetch = Download
settings-widevine-uninstall = Uninstall
confirm-uninstall-widevine-title = Uninstall the Widevine module?
confirm-uninstall-widevine = Sonora deletes the copy it downloaded from Google. Apple Music tracks stop playing until it is downloaded again.
widevine-prompt-title = Widevine module
widevine-prompt-wanted = Apple Music tracks are encrypted and need Google's Widevine module. No browser on this computer has one. Sonora can download it from Google's servers, the same file Chrome installs, and keep it in its own folder.
widevine-prompt-downloading = Downloading…
widevine-prompt-terms = Version { $version } is downloaded. Installing it means accepting Google's terms for the module:
widevine-prompt-installing = Installing…
widevine-prompt-download = Download
widevine-prompt-later = Not now
widevine-prompt-accept = Accept and install
widevine-prompt-decline = Decline
settings-equalizer = Equalizer
settings-equalizer-detail = Shapes the sound across ten bands, one per octave
settings-equalizer-preset = Preset
settings-equalizer-preset-detail = A ready-made curve for the bands below
settings-equalizer-custom = Custom
settings-equalizer-flat = Flat
settings-equalizer-bass-boost = Bass boost
settings-equalizer-bass-reducer = Bass reducer
settings-equalizer-treble-boost = Treble boost
settings-equalizer-vocal = Vocal
settings-equalizer-rock = Rock
settings-equalizer-pop = Pop
settings-equalizer-jazz = Jazz
settings-equalizer-classical = Classical
settings-equalizer-electronic = Electronic
settings-equalizer-acoustic = Acoustic
settings-equalizer-loudness = Loudness
settings-equalizer-decibels = { $db } dB
settings-equalizer-hertz = { $hz } Hz
settings-equalizer-kilohertz = { $khz } kHz
settings-panel-lyrics-size = Lyrics size (panel)
settings-panel-lyrics-size-detail = Size of the lyrics text in the side panel, on top of the base font size
settings-fullscreen-lyrics-size = Lyrics size (fullscreen)
settings-fullscreen-lyrics-size-detail = Size of the lyrics text on the fullscreen player, on top of the base font size
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = Lyrics for local files
settings-lyrics-for-local-files-detail = Use metadata from local files to fetch lyrics from the internet
settings-prefer-local-lyrics = Prefer local lyrics
settings-prefer-local-lyrics-detail = Use the lyrics stored in a local file's tags or its .lrc file instead of searching the other providers
settings-karaoke-lyrics = Karaoke lyrics
settings-karaoke-lyrics-detail = Highlight lyrics word by word when timing is available
settings-blur-lyrics = Blur inactive lyrics
settings-blur-lyrics-detail = Blur upcoming and previous lines in the lyrics panel
settings-romanized-lyrics = Romanized lyrics
settings-romanized-lyrics-detail = Show locally generated pronunciation for selected writing systems
settings-romanization-writing-systems = Writing systems
settings-lyrics-providers = Lyrics providers
settings-lyrics-providers-detail = Choose which services to search for lyrics
settings-lyrics-providers-selected = { $count ->
    [one] { $count } selected
   *[other] { $count } selected
    }
settings-lyrics-provider-local = Local files
settings-lyrics-provider-spotify = Spotify
settings-lyrics-provider-youtube = YouTube Music
settings-lyrics-provider-apple-music = Apple Music
settings-lyrics-provider-musixmatch = Musixmatch
settings-lyrics-provider-lrclib = LRCLIB
settings-lyrics-provider-kugou = Kugou
settings-lyrics-provider-netease = NetEase
settings-romanization-japanese = Japanese
settings-romanization-chinese = Chinese
settings-romanization-korean = Korean
settings-romanization-cyrillic = Cyrillic
settings-romanization-greek = Greek
settings-romanization-arabic = Arabic
settings-romanization-other = Other writing systems
settings-advanced = Advanced
settings-group-window = Window
settings-group-accounts = Accounts
settings-group-library = Library
settings-group-text = Text
settings-group-motion = Motion
settings-group-fullscreen = Fullscreen
settings-group-title-bar = Title bar
settings-group-window-style = Window style
settings-group-lyrics = Lyrics
settings-group-equalizer = Equalizer
settings-group-discord = Discord
settings-group-project = Project
settings-adaptive-menu = Adaptive context menu
settings-adaptive-menu-detail = Leaves out entries the row already shows, such as the album or the artist
settings-accounts = Manage accounts
settings-accounts-detail = The services this device can play from
settings-provider-none = Not connected
settings-provider-connected = Connected
settings-provider-current = Playing from this service
settings-provider-guest = Playing as a guest
settings-provider-switch = Switch to
settings-sign-out = Sign out
settings-group-scrobbling = Scrobbling
settings-lastfm = Last.fm
settings-lastfm-detail = Sonora scrobbles through your own Last.fm API account. Create one, then paste the key and the secret here.
settings-librefm = Libre.fm
settings-listenbrainz = ListenBrainz
settings-listenbrainz-detail = Paste a user token from your ListenBrainz settings page.
settings-maloja = Maloja
settings-maloja-detail = Point Sonora at your Maloja server and paste one of its API keys.
settings-scrobble-off = Not connected
settings-scrobble-on = Connected
settings-scrobble-waiting = Waiting for your browser…
settings-scrobble-as = Scrobbling as { $name }
settings-scrobble-failed = Could not connect
settings-scrobble-connect = Connect
settings-scrobble-disconnect = Disconnect
settings-scrobble-title = Connect { $service }
settings-scrobble-request = Get an API key
settings-scrobble-token-request = Get a token
settings-scrobble-key = API key
settings-scrobble-secret = API secret
settings-scrobble-token = User token
settings-scrobble-server = Server address
settings-local-folder = Music folders
settings-local-folder-empty = Not configured
settings-choose-folder = Choose folder…
settings-add-folder = Add folder
settings-remove-folder = Remove folder
settings-rescan = Rescan
settings-scan-walking = Scanning…
settings-scan-progress = { $percent }%
settings-scan-done = Done in { $seconds }s
settings-tab-about = About
settings-version = Version
settings-version-detail = The build of sonora you are running
settings-license = License
settings-license-detail = GNU General Public License version 3 or later
settings-license-view = Read the license
settings-source = Source code
settings-source-detail = The corresponding source for this build
settings-source-view = Open the repository
settings-team = Team
settings-team-github = GitHub
settings-role-lead-maintainer = Lead Maintainer
settings-role-maintainer = Maintainer
settings-role-contributor = Contributor
settings-notice = Copyright © 2026 Sonora Contributors. Sonora comes with absolutely no warranty. It is free software, and you are welcome to redistribute it under the terms of the GNU General Public License version 3 or later. Sonora is unofficial and is not affiliated with Spotify AB.

# themes
theme-system = System
theme-dark = Dark
theme-light = Light
theme-midnight = Midnight
theme-forest = Forest
theme-ocean = Ocean
theme-rose = Rose
theme-lavender = Lavender
theme-amber = Amber

# corners
corners-square = Square
corners-subtle = Subtle
corners-rounded = Rounded
corners-round = Round

# motion
motion-system = System
motion-always = Always
motion-never = Never
pace-slow = Slow
pace-base = Standard
pace-quick = Quick
saver-off = Off
saver-light = Light ({ $fps } FPS)
saver-medium = Medium ({ $fps } FPS)
saver-strong = Strong ({ $fps } FPS)

toast-playlist-created = Playlist created
toast-playlist-renamed = Playlist renamed
toast-playlist-deleted = Playlist deleted
toast-local-delete-failed = Some track files could not be deleted
toast-playlist-added = Playlist added to your library
toast-playlist-removed = Playlist removed from your library
toast-playlist-visibility = Playlist visibility changed
toast-track-added = Added to { $name }
toast-track-removed = Removed from { $name }
toast-playlist-failed = That change could not be saved
toast-playlist-busy = Another change is still running
toast-playlist-signed-out = Sign in to change playlists
toast-queued-track = { $name } added to the queue
toast-next-track = { $name } plays next
toast-last-track = { $name } plays last
toast-queued-album = Album added to the queue
toast-next-album = Album plays next
toast-last-album = Album plays last
toast-queued-playlist = Playlist added to the queue
toast-next-playlist = Playlist plays next
toast-last-playlist = Playlist plays last
toast-queued-artist = Artist added to the queue
toast-next-artist = Artist plays next
toast-last-artist = Artist plays last
toast-queue-failed = That could not be added to the queue
toast-keys-refused = Spotify is not granting this account playback keys
toast-sign-in-to-play = { $name } only streams to a signed-in listener
toast-track-unplayable = { $name } could not be played
toast-library-add-failed = { $name } could not be added to your library
toast-library-remove-failed = { $name } could not be removed from your library
toast-library-added = Added to your library
toast-library-removed = Removed from your library

# lyrics
lyrics-title = Lyrics
lyrics-idle = Play something to see its lyrics
lyrics-loading = Looking for lyrics…
lyrics-missing = No lyrics found, sorry!
lyrics-instrumental = This song is instrumental
lyrics-failed = Could not reach the lyrics service
lyrics-follow = Follow the song again
lyrics-source = Lyrics from { $source }
lyrics-source-local = Lyrics from the local file
lyrics-writers = Written by { $writers }

update-available = Sonora { $version } is out
update-detail = You are on { $running }. Read what changed, or update now.
update-detail-notes = You are on { $running }. Read what changed, then update Sonora the way you installed it.
update-notes = What's new
update-now = Update
update-later = Later
update-working = Downloading the update…
update-failed = The update could not be installed. Try again from the releases page.
settings-check-updates = Check for updates
settings-check-updates-detail = Ask GitHub once at startup whether a newer version is out. Sonora installs the update itself on Windows only; elsewhere it points you at what changed
settings-log = Log file
settings-log-detail = What Sonora wrote while running. Attach it to a bug report
settings-log-open = Open log

# tags
tags-edit-title = Edit tags
tags-sheet-song = Song
tags-sheet-album = Album
tags-sheet-details = Details
tags-title = Title
tags-artist = Artist
tags-track = Track number
tags-track-total = Tracks on release
tags-disc = Disc number
tags-disc-total = Discs in release
tags-album = Album
tags-album-artist = Album artist
tags-year = Year
tags-genre = Genre
tags-composer = Composer
tags-publisher = Publisher
tags-isrc = ISRC
tags-comment = Comment
toast-tags-saved = Saved the tags for { $name }
toast-tags-failed = The tags could not be saved

nav-pin = Pin
toast-library-pin-failed = Could not update the pin.
nav-nothing-pinned = Nothing here
nav-pins-alphabetical = Alphabetical
nav-pins-kind = By type
nav-show-full-library = Show full library
nav-return-top = Back to top

# trouble
trouble-offline = No connection
trouble-offline-detail = Check your internet connection and try again.
trouble-not-loaded = Could not load
trouble-retry = Try again
toast-offline = No connection. Nothing will stream until it is back.
