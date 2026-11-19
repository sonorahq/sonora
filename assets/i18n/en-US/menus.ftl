# track menu
menu-new-playlist = New playlist
menu-import-playlist = Import playlist…
menu-no-playlists = No playlists
menu-search-playlists = Search a playlist
menu-save-library = Library
menu-add-to-library = Add to Favorites
menu-remove-from-library = Remove from Favorites
menu-remove-tracks-from-playlist = { $count ->
    [one] Remove { $count } track from playlist
   *[other] Remove { $count } tracks from playlist
}
menu-remove-tracks-from-history = { $count ->
    [one] Remove { $count } track from history
   *[other] Remove { $count } tracks from history
}
menu-download = Download
menu-remove-download = Remove download
menu-downloading = Downloading. Click to stop
menu-song-radio = Go to song radio
menu-go-to-album = Go to album
menu-go-to-artist = Go to artist
menu-go-to-playlist = Go to playlist
menu-view-details = View details
menu-copy-link = Copy link
menu-select-all = Select all
menu-remove-from-queue = Remove from queue
menu-export-playlist = Export playlist…
menu-add-playlist-to-library = Add to Library
menu-remove-playlist-from-library = Remove from Library
menu-make-playlist-public = Make public
menu-make-playlist-private = Make private
menu-play-artist = Play artist

# playlist editor
menu-copy-link-none = This has no link to share
menu-copy-link-local = Local artists and playlists have no link
menu-play-next = { $count ->
    [1] Play next
    [one] Play { $count } track next
   *[other] Play { $count } tracks next
}
menu-add-to-queue = { $count ->
    [1] Add to queue
    [one] Add { $count } track to queue
   *[other] Add { $count } tracks to queue
}
menu-play-last = { $count ->
    [1] Play last
    [one] Play { $count } track last
   *[other] Play { $count } tracks last
}
menu-delete-track-files = { $count ->
    [1] Delete track file
    [one] Delete { $count } track file
   *[other] Delete { $count } track files
}
menu-add-to-playlist = { $count ->
    [1] Add to playlist
    [one] Add { $count } track to playlist
   *[other] Add { $count } tracks to playlist
}

# smart rules
smart-rules-create-title = New smart playlist
smart-rules-edit-title = Smart rules
smart-rules-loading = Reading the rules from the server…
smart-rules-saving = Saving…
smart-rules-failed-title = The rules could not be read
smart-rules-match-all = Match all
smart-rules-match-any = Match any
smart-rules-match-toggle = Match all of these rules, or any of them
smart-rules-of-following = of the following
smart-rules-add-condition = Add condition
smart-rules-add-group = Add group
smart-rules-remove-group = Remove this group
smart-rules-remove-condition = Remove this rule
smart-rules-empty-group = No rules here yet
smart-rules-field = Field
smart-rules-operator = Operator
smart-rules-value-placeholder = Value
smart-rules-range-and = and
smart-rules-true = Yes
smart-rules-false = No
smart-rules-unknown-note = Kept as the server sent it
smart-rules-synced-title = This playlist syncs from a file
smart-rules-synced-message = The next scan can overwrite an edit made here.
smart-rules-sort-label = Sort by
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = No direction
smart-rules-order-asc = Ascending
smart-rules-order-desc = Descending
smart-rules-order-toggle = Sort direction
smart-rules-limit-label = Limit
smart-rules-limit-placeholder = All matches
smart-rules-problem-empty-group = The group holds no rules
smart-rules-problem-missing-field = The condition names no field
smart-rules-problem-range = A range takes a lower and an upper bound
smart-rules-problem-days = A day count takes a number
smart-rules-problem-playlist = A playlist rule takes a playlist
smart-rules-problem-truth = The rule takes a truth value
smart-rules-problem-compared = The value does not fit the rule
smart-rules-problem-boolean = A boolean field takes a truth value
smart-rules-problem-bad-order = The order is asc or desc

# playlist editor
playlist-name-placeholder = Playlist name
playlist-create-title = Create playlist
playlist-create-shelf = Save to
playlist-edit-title = Edit playlist
playlist-cover-choose = Choose picture
playlist-cover-remove = Remove cover
playlist-cover-unopenable = Sonora can't open this file
playlist-cover-too-large = This picture is larger than 40 MB
playlist-cover-not-a-picture = This file is not a JPEG, PNG or WebP picture

# cover viewer
playlist-delete-title = Delete playlist
playlist-again-title = Add it again?
playlist-again-confirm = This track is already in “{ $name }”. Add another copy?
playlist-again-add = Add again

# confirm

# cover viewer
cover-view = View cover
cover-zoom-in = Zoom in
cover-zoom-out = Zoom out
cover-zoom-fit = Fit to window

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
confirm-uninstall-widevine-title = Uninstall the Widevine module?
confirm-uninstall-widevine = Sonora deletes the copy it downloaded from Google. Apple Music tracks stop playing until it is downloaded again.
confirm-remove-soloist-title = Remove Soloist?
confirm-remove-soloist = Sonora deletes the Soloist program, your API key and its sign-in. Spotify goes back to the built-in player.
confirm-remove-downloads-title = Remove all downloads?
confirm-remove-downloads = Every song you downloaded is deleted from this computer, from every service. They stream again until you download them once more.
confirm-delete-named = Delete “{ $name }”? This cannot be undone.
