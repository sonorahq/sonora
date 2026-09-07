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
nav-favorites = Favorites
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
library-no-artists = No followed artists yet
library-no-local-songs = No imported songs found
library-no-local-favorites = No local favorites yet
library-no-local-albums = No imported albums found
library-no-local-artists = No imported artists found
library-no-local-playlists = No local playlists yet
library-no-matches = No matches
library-not-loaded = Your library did not load
library-part-not-loaded = This part of your library did not load
library-local-unconfigured = Configure your local library

# app menu
app-refresh-library = Refresh Library
app-sign-out = Sign Out
app-quit = Quit

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
confirm-unfollow-title = Unfollow
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
confirm-unfollow-artists = { $count ->
    [one] Unfollow this artist?
   *[other] Unfollow { $count } artists?
}
confirm-remove-playlists = { $count ->
    [one] Remove this playlist from your library?
   *[other] Remove { $count } playlists from your library?
}

# queue panel
queue-title = Queue
queue-history = History
queue-now-playing = Now playing
queue-from = From
queue-up-next = Up next
queue-reset = Reset
queue-clear = Clear
queue-empty = Your queue is empty
queue-similar = Similar tracks
queue-radio = Autoplay similar tracks

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
player-seek-back = Seek back
player-seek-forward = Seek forward
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
login-use = Use { $provider }
login-guest-title = Guest mode
login-guest-use = Use Guest mode
login-guest-detail = Browse and play without an account. Your library, likes and playlists stay out of reach.
login-usage-consent = Help us estimate how many people use Sonora.
login-device-code = Enter this code at { $url }
login-cookie-open = Open YouTube Music
login-cookie-submit = Continue
login-cookie-hint = Paste the Cookie request header here
login-cookie-step-1 = Open music.youtube.com and make sure you are signed in.
login-cookie-step-2 = Press F12, open the Network tab and reload the page.
login-cookie-step-3 = Select any request named "browse" or "next".
login-cookie-step-4 = In Headers, find Cookie under Request Headers, right-click it and copy its value.
login-cookie-step-note = Make sure to paste the whole value, including SAPISID and __Secure-3PAPISID.
login-cookie-title = Paste your YouTube Music cookies to finish signing in
login-account-title = Choose an account
login-account-detail = This session is signed in to more than one Google account. Pick the one Sonora should use.

# album and playlist pages
detail-album = Album
detail-playlist = Playlist
detail-play-album = Play album
detail-play-playlist = Play playlist

# play button
play-pause = Pause
play-resume = Resume
play-loading = Loading…

# artist page
artist-eyebrow = Artist
artist-monthly-listeners = { $count ->
    [one] { $value } monthly listener
   *[other] { $value } monthly listeners
}
artist-play = Play now
artist-follow = Follow
artist-unfollow = Unfollow
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
home-listen-again = Listen again
home-quick-picks-eyebrow = Start from a song
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
settings-tab-general = General
settings-tab-appearance = Appearance
settings-tab-playback = Playback
settings-tab-privacy = Privacy
settings-theme = Theme
settings-theme-detail = Choose the application colour palette
settings-opacity = Opacity
settings-opacity-detail = Adjust the app background opacity
settings-opacity-value = { $percent }%
settings-theme-config = Open config
settings-adaptive = Adaptive theme
settings-adaptive-detail = Tint the palette with the artwork of the playing album
settings-visualizer = Visualizer
settings-visualizer-detail = Show spectrum bars behind fullscreen artwork
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
settings-window-controls = Window controls
settings-window-controls-detail = Draw minimise, maximise and close in the title bar
settings-controls-side = Controls side
settings-controls-side-detail = Which end of the title bar the controls sit on
settings-close-to-tray = Keep playing when closed
settings-close-to-tray-detail = Keep Sonora in the system tray and continue playing after its window closes
settings-normalisation = Normalize loudness
settings-normalisation-detail = Keeps tracks at a consistent volume
settings-gapless = Gapless playback
settings-gapless-detail = Runs one track into the next without a pause, the way an album was sequenced
settings-seek-step = Seek step
settings-seek-step-detail = How far the seek buttons and arrow keys jump
seek-step-5 = 5 seconds
seek-step-10 = 10 seconds
seek-step-30 = 30 seconds
settings-panel-lyrics-size = Lyrics size (panel)
settings-panel-lyrics-size-detail = Size of the lyrics text in the side panel, on top of the base font size
settings-fullscreen-lyrics-size = Lyrics size (fullscreen)
settings-fullscreen-lyrics-size-detail = Size of the lyrics text on the fullscreen player, on top of the base font size
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = Lyrics for local files
settings-lyrics-for-local-files-detail = Use metadata from local files to fetch lyrics from the internet
settings-karaoke-lyrics = Karaoke lyrics
settings-karaoke-lyrics-detail = Highlight lyrics word by word when timing is available
settings-blur-lyrics = Blur inactive lyrics
settings-blur-lyrics-detail = Blur upcoming and previous lines in the lyrics panel
settings-romanized-lyrics = Romanized lyrics
settings-romanized-lyrics-detail = Show locally generated pronunciation for selected writing systems
settings-romanization-writing-systems = Writing systems
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
settings-group-title-bar = Title bar
settings-group-window-style = Window style
settings-group-lyrics = Lyrics
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
settings-local-folder = Imported music folder
settings-local-folder-empty = Not configured
settings-choose-folder = Choose folder…
settings-rescan = Rescan
settings-clear-folder = Clear
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
toast-queued-album = Album added to the queue
toast-next-album = Album plays next
toast-queued-playlist = Playlist added to the queue
toast-next-playlist = Playlist plays next
toast-queued-artist = Artist added to the queue
toast-next-artist = Artist plays next
toast-queue-failed = That could not be added to the queue
toast-keys-refused = Spotify is not granting this account playback keys
toast-sign-in-to-play = { $name } only streams to a signed-in listener
toast-track-unplayable = { $name } could not be played
toast-library-add-failed = { $name } could not be added to your library
toast-library-remove-failed = { $name } could not be removed from your library

# lyrics
lyrics-title = Lyrics
lyrics-idle = Play something to see its lyrics
lyrics-loading = Looking for lyrics…
lyrics-missing = No lyrics found, sorry!
lyrics-instrumental = This song is instrumental
lyrics-failed = Could not reach the lyrics service
lyrics-follow = Follow the song again
lyrics-source = Lyrics from { $source }
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
