members-title = Members
members-empty = Start or join a Session to see who is listening with you

# login
login-restoring = Checking your saved session…
login-authorizing = Waiting for authorization in your browser…
login-signed-in = Signed in as { $name }
login-failed-title = Sign-in failed
login-problem-region = Spotify will not open a session from the country you are in. Sign in from your home country, or change the country on your Spotify account.
login-problem-credentials = Your saved session for this service is no longer valid. Sign in again to continue.
login-problem-password = The server did not accept that username and password. Check them and try again.
login-problem-network = Sonora could not reach the music service. Check your internet connection and try again.
login-problem-blocked = The music service is refusing connections from your network. If you use a VPN, try another server or turn it off.
login-problem-cancelled = The sign-in was not approved in the browser, or ran out of time. Start again to finish.
login-problem-refused = Spotify turned down the sign-in. Wait a moment and try again.
login-problem-premium = Sonora streams through Spotify Premium, and this account does not have it. Sign in with a Premium account to continue.
login-problem-subscription = No active subscription on this Apple ID.
login-subscription-check = Check Apple Music
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
login-guest-title = Guest mode
login-guest-use = Use Guest mode
login-usage-consent = Help us estimate how many people use Sonora.
login-device-code = Enter this code at { $url }
login-server-title = Connect to your Subsonic server
login-server-detail = Navidrome, Gonic or any OpenSubsonic server.
login-server-hint = https://music.example.com
login-jellyfin-title = Connect to Jellyfin
login-jellyfin-detail = Sign in with your Jellyfin account.
login-quick-connect = Sign in with Quick Connect
login-username-hint = Username
login-password-hint = Password
login-server-submit = Sign in
login-account-title = Choose an account
login-account-detail = This session is signed in to more than one Google account. Pick the one Sonora should use.
login-profile-title = Choose a profile
login-profile-detail = This account has more than one profile. Pick the one Sonora should use.
login-choose-title = Sign in to { $provider }
login-choose-detail = Choose how to sign in to { $provider }.
login-providers = Music services
login-welcome-title = Welcome to Sonora
login-welcome-detail = You can add more services later in Settings.
login-or = or

# album and playlist pages

# artist page
artist-eyebrow = Artist
artist-monthly-listeners = { $count ->
    [one] { $value } monthly listener
   *[other] { $value } monthly listeners
}
artist-followers = { $count ->
    [one] { $value } follower
   *[other] { $value } followers
}
artist-popular = Popular
artist-popular-eyebrow = Explore this artist
artist-popular-empty = Nothing to play from this artist yet
artist-favorite-songs = Your favorite songs
artist-favorite-songs-count = { $count ->
    [one] { $count } song in your Favorites
   *[other] { $count } songs in your Favorites
}
artist-releases = Releases
artist-filter-all = All
artist-filter-singles = Singles
artist-filter-eps = EPs
artist-appears-on = Appears on
album-also-like = You might also like
album-tab-artists = Artists
album-label = ℗ { $label }

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

# home page
home-quick-picks = Quick picks
home-quick-picks-empty = Like a few songs and they will show up here
home-recently-added = Recently added
home-recently-played = Recently played
home-most-played = Most played
home-favorite-albums = Favorite albums
home-artists = Artists
home-collection-albums = Albums from your collection

# powerbar (quick search)
powerbar-title = Quick search
powerbar-open = Open
powerbar-song = Go to song

# search page
search-placeholder = What do you want to listen to?
search-browse = Browse all
search-submit = Search
search-open-link = Open link in Sonora
search-recent = Recent searches
search-forget = Remove from history
genre-empty = Nothing to show here yet
search-best-match = Best match
search-no-matches = No matches
search-results = Results
search-artists = Artists
search-albums-playlists = Albums & playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } song in Library
       *[other] { $count } songs in Library
    }

# song page
song-eyebrow = Song
song-view-album = View album
song-about = About this song
song-album = Album
song-released = Released
song-streams = Streams
song-position = Position
song-label = Label
song-popularity = Popularity
song-disc-track = Disc { $disc }, track { $track }
song-track = Track { $track }
song-credits = Credits
song-performed-by = Performed by
song-details = Genres & details
song-genres = Genres
song-language = Language
song-content = Content
song-explicit = Explicit
song-local = Local file
song-clean = Clean
artist-about = About the artist
artist-about-fallback = Explore the artist's popular songs and releases.
artist-about-open = Go to artist
song-copyright = © { $notice }
track-row-name = { $title } by { $artists }

# tags
tags-edit-title = Edit tags
tags-sheet-song = Song
tags-sheet-album = Album
tags-sheet-details = Details
tags-title = Title
tags-artist = Artist
tags-artists = Artists
tags-add-artist = Add an artist
tags-remove-artist = Remove this artist
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
