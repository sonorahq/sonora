# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Deezer is a sign-in option: browse as a guest or paste an ARL. Search, charts, albums and
  artists work; an ARL plays the full track.

## [0.31.0] - 2026-09-05

### Added

- Nix users can manage Sonora through Home Manager. The flake exposes `homeManagerModules.default`
  with a `programs.sonora` option whose `settings` are merged into `settings.json` on each launch.
- Sonora speaks Indonesian. Pick Bahasa Indonesia under Settings > General > Language, or leave the
  language on System and it follows an Indonesian desktop on its own.
- On Linux and FreeBSD, Sonora can switch between server-side and client-side window decorations
  from Appearance and shows its own window controls automatically with client-side decorations.

### Changed

- Window position, sidebar sizes, playback mode, table layouts, pins, listening history and local
  playlists now live in one `state.sqlite` file in the data directory, and `settings.json` keeps
  only preferences. Existing files are migrated on the first start.
- YouTube Music sign-in now opens your default browser from the cookie instructions dialog.
  Browser cookie extraction and automatic refresh from browser profiles have been removed.
- Each provider keeps its sign-in in its own `credentials.json` under the cache folder, readable
  only by you. Existing Spotify and YouTube Music sign-ins move over on the first launch.

### Fixed

- Playback no longer falls silent on PipeWire systems with a large graph quantum, such as a
  default Arch Linux install. The audio stream now keeps 50 ms of buffer regardless of the
  quantum, and a recovered underrun no longer restarts the player.
- Pressing play on an album, playlist or artist with shuffle on now opens with a random track
  instead of always the first one.

## [0.30.0] - 2026-09-04

### Added

- Closing the window no longer stops the music: Sonora stays in the system tray with play/pause,
  previous, next, show and quit at hand, and the Dock icon steps aside on macOS until the window
  is back. Turn it off under Settings → General → Window if you would rather it quit.
- Sonora speaks Spanish. Pick Español under Settings > General > Language, or leave the language on
  System and it follows a Spanish desktop on its own.
- Sonora speaks Japanese. Pick 日本語 under Settings > General > Language, or leave the language on
  System and it follows a Japanese desktop on its own.
- Lyrics have their own size, set separately for each surface. Settings > Playback > Lyrics size
  (panel) and Lyrics size (fullscreen) scale the lyrics text from 60% to 200% on top of the base
  font size.
- Fullscreen artwork now has a real-time spectrum visualizer, which can be switched off under
  Settings > Appearance > Visualizer.
- The playlist library can be filtered to show only playlists you own.
- Opus audio playback.
- Lyrics fetching for local files can be switched off under Settings > Playback.

### Changed

- The fullscreen title and artist names stay on screen while the player is idle. Only the heart
  beside the title fades, the way it already did in the narrow lyrics view.
- YouTube Music cookie onboarding now walks through the browser steps in a clearer checklist.

### Fixed

- Playback now skips deleted or unavailable playlist songs when moving forward or backward.
- Installing the standalone `.flatpak` bundle now registers the Sonora repository, so `flatpak update`
  keeps it current, and it can replace an install made from the repository.
- Emoji in playlist, track, and artist names now render instead of falling back to missing-glyph
  boxes when the UI font has no emoji glyphs.
- Changing the audio output restarts playback on the selected device.
- Artwork uses less memory while images load and remain cached.

## [0.29.0] - 2026-09-03

### Added

- Sonora ships as a Flatpak. Every release attaches a bundle for x86_64 and aarch64, and adding
  the Sonora repository once (`flatpak install --user https://nolight132.github.io/sonora/sonora.flatpakref`)
  keeps it current through `flatpak update`.
- Italian and Brazilian Portuguese translations.

### Changed

- Each section of Your Library appears as soon as it has loaded instead of waiting for the slowest
  one, and a section that fails no longer holds the others back.
- Quick Picks and Listen Again start the chosen song at once and fill the queue behind it with
  recommendations, instead of queuing the rest of the shelf.
- The fullscreen player's controls settle in and out on a spring when it wakes or goes idle.
- Karaoke lines that overlap in time each keep sweeping until they finish, background vocals sing
  through in a softer tone, and a finished line fades to gray instead of snapping.
- Windows uses Sonora's window controls without the native system control strip.
- The best match on the search page carries a play button whatever it is, so an album, artist or
  playlist starts from there just like a song. In the single-column layout it scrolls with the
  results instead of staying pinned above them.

### Fixed

- Large Spotify libraries load again: track and album metadata is fetched in smaller batches
  that stay within what the service accepts.
- Content no longer shows through an overlaid sidebar when window transparency is enabled.
- Building Sonora on Windows no longer requires a separately installed SQLite library.
- A sign-in failure in Settings now appears on the card of the service you were signing in to.
- Running Sonora through Nix on a system other than NixOS finds a Vulkan driver and the ALSA
  plugins it needs for audio.

## [0.28.1] - 2026-09-02

### Changed

- Sonora now uses the SQLite library your system provides instead of compiling in its own copy, so
  SQLite fixes reach it through a normal system update. Windows still carries its own.

## [0.28.0] - 2026-09-01

### Added

- Settings > Appearance can swap the interface icons between four sets: Lucide, Iconoir, Remix and
  Solar. Each entry in the picker previews a few of its own glyphs, and an icon a set does not
  carry falls back to Lucide.

## [0.27.0] - 2026-09-01

### Added

- Local Music stays in the sidebar before a folder is picked, and every one of its pages offers a
  Choose folder button so the library can be set up without opening Settings.
- A toast that names an album, artist, playlist or song turns that name into a link to its page.
- The sign-in screen carries a one-off checkbox that helps count how many people use Sonora. It is
  ticked by default and sends a single anonymous ping with your next action on that screen; untick
  it to send nothing. The checkbox never returns once you have signed in or answered it.

### Changed

- Pinned items keep a single order across providers, so a streaming pin and a local one can sit
  next to each other in the sidebar.
- An empty list or grid now shows a large icon above the message that was already there.
- Menus, dropdowns, dialogs and toasts now appear with the same short fade the page transition
  uses.
- Navigating again while a page is still fading in restarts the fade instead of cutting it short.
- Toasts block clicks on whatever sits under them, and stay up while the pointer is on them.
- The artwork, lyrics and queue pill in fullscreen stays visible while the pointer is on it.
- Episodes for Later no longer shows up among your YouTube Music playlists. Sonora does not play
  podcasts, so the container was never usable.

### Fixed

- Making a Spotify playlist public or private works again. The request was rejected, so the menu
  toggled the label without changing anything on the account.
- A text field no longer keeps its selection highlight after it loses focus.
- Page content no longer shifts by a pixel when a navigation transition finishes.

## [0.26.0] - 2026-08-31

### Added

- Local Music is its own sidebar entry that expands into Songs, Favorites, Albums, Artists and
  Playlists, and it reuses the Your Library screen.
- Imported songs can be hearted. Local favorites are their own section and are kept on this
  machine.
- Local playlists can be created, renamed, deleted and filled with imported songs; they are kept
  in a database beside the rest of the local library.
- Imported songs can be retagged from a dialog with Song, Album and Details tabs. Sonora writes
  the file and rescans the folder.
- An imported album picks up a cover.jpg sitting in the album folder, and an artist folder can
  carry a portrait: artist.jpg wins, then folder.jpg, then cover.jpg, in jpg, jpeg, png or webp.
- Imported albums can be shown as a list as well as a grid.
- Sorting an imported grid groups its cards under headings, the way the library grid does.
- Settings picks which entries the sidebar shows: Home, Search, Your Library, Local Music and
  History.

### Changed

- Imported music is called Local Music.
- Saved songs are called Favorites: the library tab, the page title and the Add to Favorites and
  Remove from Favorites menu items.
- An album card names the release year before the artist, on the artist page, in the library, in
  search and on Home. An imported album takes that year from the track tags or from a year at the
  start of the album folder name.
- A local playlist card counts its tracks where a streamed one names its owner.
- The YouTube Music guest button in Settings reads Use Guest mode, and the browser import button
  there drops the Firefox footnote that only applies on the sign-in screen.

### Fixed

- A dialog taller than the window scrolls instead of running off the edge.
- Pointing at the artist under a card no longer underlines the title above it. A title underlines
  when you point at the title itself.

## [0.25.0] - 2026-08-30

### Added

- YouTube Music listeners see their Listen Again history and personalized Quick Picks on Home.
- Shift-click or Shift-Up/Down selects a range of rows in a table. On songs, the context menu
  acts on all of them.
- Ctrl-click (Cmd-click on macOS) adds or removes individual rows from a table selection.
- Enter play/pauses the selected song when a table has exactly one row selected, or opens the
  selected album, artist or playlist.
- Delete or a Remove from library (or playlist, or history) action asks before it runs.
- Search results can be moved through with the arrow keys, including left and right between
  columns, and Enter play/pauses the selected song.
- Text fields have a Cut, Copy, Paste and Select all context menu.
- Search results for albums and playlists can be saved to the library from the context menu.

### Changed

- The list/grid toggle is labelled Grid instead of Cards.
- Clicking a song, album, artist or playlist in a table selects the row. Double-click or Enter
  still plays a song or opens an album, artist or playlist.
- Dialogs pack the title, copy and buttons more tightly, with a divider above the actions.

### Fixed

- Following and unfollowing an artist now updates the subscription in YouTube Music.
- YouTube Music Quick Picks paginate when the window cannot fit all three columns and never reuse
  recommendations from a previously connected Spotify account.
- Saving or removing a song, album or artist that the service refuses now says so instead of
  quietly flipping the control back.
- Double-clicking a word and dragging selects further words, instead of stopping at the first.
- Opening a settings file this build cannot read no longer replaces it with empty defaults, so pins
  stay on disk.
- Long titles on pinned sidebar items ellipsize instead of drawing past the panel.
- The left sidebar scrolls again when pinned items run past the player bar.

## [0.24.1] - 2026-08-29

### Added

- Hovering an album card's title shows the full name, so two releases that share a truncated title
  can be told apart.

### Fixed

- A track added to the playlist you have open appears in it right away, instead of only after you
  leave the page and come back.

## [0.24.0] - 2026-08-29

### Added

- Sonora keeps a listening history on this device: the last 500 tracks you played and when you
  played them. Open it from the sidebar, play anything back from it, drop a single play from its
  context menu, or clear the whole list.
- The explicit badge sits next to the track title in the player bar and on the fullscreen now
  playing screen, the same way it does in a table.
- Spotify listeners now have a page of their own, with their avatar, display name, follower and
  following counts and public playlists. Open it from a playlist owner, from the person who added a
  track, or from a `spotify:user:` link.
- A playlist on someone's profile answers a right click with the same menu as everywhere else.

### Changed

- The startup screen picker lists screens in sidebar order, with History at the end.
- Lyrics are set three pixels larger on the fullscreen now playing screen. The lyrics panel beside
  the app is unchanged.

### Fixed

- Autoplay of similar tracks is remembered between runs instead of turning itself off at every
  start.
- The fullscreen title holds still while the like button beside it fades in and out.
- Blur is drawn cleanly across a wide surface, so a translucent background no longer smears at its
  edges.
- A song that appears more than once in a list no longer lights up every copy while it plays. Only
  the first row is marked, and in the listening history that is the most recent play.
- A Blend lists who added each track in place of the date, which moves every day and says little.
  Other playlists with more than one contributor keep the date and put that person's avatar next to
  it, and a playlist you built alone looks exactly as it did.
- Imported songs no longer offer Add to playlist. A local file cannot go into a Spotify playlist,
  so the action never did anything.
- The left sidebar scrollbar is easier to hit. Its resize edge starts a few pixels further in, so
  dragging the scrollbar scrolls the list instead of resizing the panel.
- The typeface picker opens at once on a machine with many fonts installed: it previews only the
  names in view, a few families per frame, rather than loading every one up front.
- Enter picks the highlighted typeface, and filtering the list moves the highlight to the first
  match instead of leaving it on a row that is no longer there.
- On Windows, dragging Sonora to a screen edge snaps it, hovering the maximize button opens Snap
  Layouts, and clicking its taskbar icon minimizes the window.
- On Windows, a maximized Sonora no longer runs under the taskbar, so nothing is cut off at the
  bottom of the screen.
- The Windows installer offers to create a desktop shortcut with the box already ticked. Untick it
  before installing to go without.

## [0.23.0] - 2026-08-28

### Added

- A battery saving setting under Appearance caps the frame rate of animations while Sonora is not
  the focused window: light, medium or strong, for 90, 60 or 30 frames a second. It is off by
  default, and a change applies from the next launch.

### Changed

- The karaoke sweep advances on a clock of its own instead of following the display refresh rate,
  so a fast panel no longer spends frames on it, and battery saving paces it too while Sonora is in
  the background.

### Fixed

- Switches appear already on or off when a screen opens, instead of sliding into place after it.
- With the theme set to System, Sonora no longer opens as a light window and fades into the dark
  palette. It remembers what the system last reported and starts there.
- On macOS the lyrics sheet showed only the lines around the one being sung. The rows it blurs
  above and below render again.

## [0.22.0] - 2026-08-28

### Added

- Escape closes any open menu, not only the ones that hold a text field. It watches keystrokes
  rather than the Dismiss action, which the fullscreen and search-field handlers were swallowing
  before it could reach a menu.

### Changed

- The sign-in screen gives each service its own tab instead of standing them side by side, and
  guest sign-in is one button underneath rather than a row of them.
- The lyrics sheet travels on a spring, overshooting a little and settling, where it used to ease to
  a stop. The rows still trail behind it one at a time.
- On Windows the frame no longer carries the system window buttons, leaving only the ones Sonora
  draws.
- Context menu entries that queue something now read "Add to queue" instead of naming the album or
  the artist a second time. The row you right-clicked already says what it is.
- The queue writes "from" in lower case beside Now playing.
- Track titles no longer underline when the pointer is over them, in the queue, on the home page,
  in search results and on the library card grid. Clicking one plays it, it does not open a page,
  so the underline promised a link that was never there.
- The sidebar's nested tabs drop the small dash beside each row, and the vertical guide moves under
  the icon of the row that opened them.
- German, French, Polish, Russian and Ukrainian cover every string again. The font picker, the
  lyrics settings, the romanization list, the sign-in problems and the update prompts had been
  falling back to English.

### Fixed

- A submenu with no room to its right opens to the left of the menu instead of sliding over it.
  0.21.0 claimed this, but a submenu carried a second window-fitting wrapper of its own, and that
  inner wrapper both hid the submenu's size from the outer one, which then had nothing to shift by
  when it flipped, and re-anchored the panel to the top left and slid it back under the window
  edge, on top of the menu it hangs off. A submenu now renders its panel directly and only the
  outer wrapper places it.
- YouTube Music plays on an account without Premium again. Sonora asks for the guest audio streams
  rather than the ones such an account cannot be handed.
- Seeking far into a track scrolls the lyrics to the new verse. The sheet went blank for a moment and
  then dropped the verses in from off screen, because it moved to the destination at once and only
  presented the journey.
- A verse no longer loses its lower half while the sheet is in motion. Rows were drawn where they had
  been laid out and shifted afterwards, so whatever reached past the edge of the panel was cut away
  before it moved.
- A line keeps the colour it already had while it hands over to the next one. It used to flash white
  first, which was wrong for lines lit word by word and for anything on screen while paused.
- A wrapped lyric line no longer starts its second row with a space.

## [0.21.0] - 2026-08-27

### Added

- The queue now says where the music is coming from, next to Now playing, and the name opens that
  album, playlist or artist, your liked songs or the imported tab. Starting a track from a table row
  used to leave no source recorded at all, so there was nothing to show; every way of starting
  playback now carries one, and it is kept with the queue across a restart.
- Artist cards have the play button album and playlist cards already had.

### Changed

- The lyrics sheet drags its rows along when it scrolls itself, on a verse change or after clicking
  a verse. Each row is held back on a spring of its own, so they come to rest one after another
  instead of the sheet arriving all at once. Scrolling the lyrics yourself is left alone: the rows
  stay exactly where you put them.
- Cover art is cached on disk, so images come back without fetching them again after a restart.
- The track title in fullscreen is set semibold, both in the large view and in the strip.
- Lines in the lyrics sheet sit a little further apart.

### Fixed

- Scrolling the "Add to playlist" list no longer closes it. Every row reported hover on its own
  account, so as the list glided under a still cursor the row being left behind could have the last
  word and start the close timer. The submenu now tracks hover for the panel as a whole, and it
  stays open while the pointer is anywhere over it, the gap beside it, its scrollbar, or the row it
  hangs off.
- Moving back from a submenu onto the row that opened it no longer closes the submenu and leaves it
  stuck shut until the pointer has been away and returned.
- A submenu that would run off the right of the window now opens to the left of the menu instead of
  sliding over it.
- Turning the left sidebar off no longer leaves its width behind as empty space. A cached view is
  laid out from the style the cache is given and never consults its own, so hiding itself was not
  enough to give the room back.
- Lyrics no longer jerk at the end of a verse change. Rows are drawn at the exact offset they are
  given rather than rounded to whole pixels, which forced a slowing row either to sit on its last
  pixel for several frames and then hop, or to stop while it was still visibly moving.
- A verse no longer shifts sideways the frame its growth or fade finishes. Both animations now end
  at the size the line is actually set in, so shaping it at one size and scaling it to another can
  no longer disagree by a pixel, which a right-aligned background verse showed as a jump.
- The lyrics blur keeps up with a long jump, such as scrubbing from the end of a track back to the
  start. It is worked out from where a row is drawn instead of where it was laid out, so it no
  longer goes missing at the edge the sheet is coming from.
- A background lane no longer jumps as it finishes opening. The room it needs is measured from the
  line height it inherits and counts the gaps between lanes, so nothing is left clipped until the
  last frame.
- Scrolling the lyrics with a trackpad stops the panel following the sung verse, the same as a mouse
  wheel already did.
- The wider gap above a change of voice no longer stretches that row.

## [0.20.0] - 2026-08-27

### Changed

- Lyrics settle once instead of improving in stages. Sonora asks every source at once, and used to
  put up each answer that beat the last, so the words could change under you two or three times in
  the first seconds of a song. It now shows the first answer that has timings, and swaps at most once
  more: the moment a word-by-word sheet arrives, since nothing beats one, or else when every source
  has answered and the best of them is known. That one change comes in through a blur and a fade
  rather than appearing abruptly. Lyrics without timings are not shown at all until the search is
  over, so a plain sheet no longer flashes up in place of the synced one that was still coming.

### Fixed

- Lyrics blur and dim smoothly as they move rather than in three visible steps. The depth of field
  was rounded to whole pixels so neighbouring lines could share a filter, which left a line jumping
  between a handful of levels instead of following the scroll.
- Scrubbing a track that is still loading no longer starts it from the beginning. The seek was sent
  to an engine that was not ready for it and lost, so playback began wherever the track had loaded
  and the position jumped back a moment later. It is now applied again as soon as playback starts,
  which matters most on YouTube, where a track can take seconds to come up.
- A verse growing into place no longer steps through a few sizes, and the sheet no longer settles
  with a jolt once it has finished. The growth is drawn at the size the verse ends up and scaled into
  place from its leading edge, so the text is measured and drawn once instead of once per frame, and
  every line reserves the room the sung one needs, so becoming the sung line moves nothing around it.
- The word-by-word highlight no longer slips backwards inside a word. Lyrics providers split a word
  where it is sung in two — "nothing" arrives as "no" and "thing" — and the soft trail the highlight
  carries hardened at every one of those boundaries and softened again on the next, which moved the
  visible edge back about half a character. The trail now runs the whole way across a line and only
  sharpens as the line fills.
- A verse growing into place no longer costs a dropped frame. It was animated by setting a slightly
  different text size on every frame, and both the shaped line and every rasterised glyph are keyed
  by size, so the whole verse was measured and redrawn from scratch each frame. The size now lands
  on the pixel grid, which asks for two or three sizes across the whole growth.
- Lyrics no longer hitch as a track loads or as verses scroll into view. Measuring a line asked the
  text system to shape every piece of it separately, which on Japanese, Chinese and Korean lyrics is
  one shaping call per character and, on the frame a sheet first appears, hundreds of them at once.
  A line is shaped once now, and the widths that come back also account for the kerning between one
  piece and the next, so the highlight sits exactly where the glyphs do.

## [0.19.1] - 2026-08-27

### Changed

- Sonora redraws far less of the window at once. A word-by-word lyric line, a melody break and a
  verse change each rebuilt the player bar, the sidebar and the toolbar on every frame they
  animated, and scrolling a busy screen rebuilt the same things again, so the two together could not
  keep pace. Scrolling Home or your songs while the lyrics move is smooth now.
- Sonora looks for a newer version on Linux and macOS too, where the setting for it was already
  offered but never did anything. It only tells you: installing in place stays a Windows-only path,
  because that is the only build Sonora ships an installer for. The setting starts off there, since
  a distribution or a tap can trail a release by weeks and there is nothing to act on until it
  catches up, and the notice says to update Sonora the way you installed it.

### Fixed

- Long descriptions in Settings wrap onto a second line instead of being cut off.
- The lyrics panel measures a line once per panel width rather than on every frame, and a verse well
  outside the panel holds its place without being laid out at all.

## [0.19.0] - 2026-08-27

### Added

- Right-clicking an artist now offers Play artist, Play next, Add artist to queue, Follow and Copy
  link, wherever the artist appears. It only ever offered Copy link before.
- Albums and playlists on Home and on a genre page answer a right-click with the same menu they
  have in Your Library, so a card and a row no longer disagree about what you can do with an item.
- The song page and the now-playing artwork in the player bar answer a right-click with the track
  menu. The song page had no context menu at all, and the artwork ignored the click even though the
  title beside it did not.
- Songs, albums, playlists and artists can be dropped straight onto the queue. Drop between two
  queued songs to slot them in there, or anywhere else in the pane to add them at the end.
- Sonora tells you when a newer version is out. On Windows it asks GitHub once at startup and, if
  a release is newer than the running build, floats a card in the top-left corner with the new
  version number, a link to what changed, and a choice between Later and Update. Update downloads
  the installer, checks it against the release checksums, runs it and starts Sonora again when it
  is done. The card appears once per launch and never nags mid-session, and the check can be turned
  off under Settings, About.
- Sonora reopens at the size and position it had when you closed it, and maximized if you left it
  that way. A saved position that no longer lands on a connected display is dropped, so unplugging
  a monitor cannot leave the window off screen. Wayland does not hand a window its position, so
  there only the size and the maximized state come back.
- Settings, Appearance can pick the interface font. The list is searchable and holds every font
  installed on the machine that can actually set the interface, so the script-only families a system
  carries do not clutter it, and Bundled keeps the Inter that ships with Sonora.

### Changed

- Previous restarts the track you are on once you are more than three seconds into it, and only
  steps back a track if you press it near the start. There is also nothing to step back to on the
  first track of a queue, so there it always restarts.
- New installs start with different defaults. Album art tints the theme, corners are rounded rather
  than subtle, volume normalisation is off, and lyrics are romanized for Japanese, Chinese and
  Korean only. Albums, playlists and artists open as cards, an artist's own page as a list, and both
  sidebars are a little narrower. Anything you have already set is left alone.
- The lyric line being sung is set a little larger than the rest, and grows into that size as it
  arrives instead of only brightening. Entering or leaving fullscreen no longer replays that growth
  for a line that arrived a while ago.
- Background vocals only show on the line being sung. They fade in as the line arrives and fade out
  as it leaves, instead of sitting under every line in the sheet, and the space they take opens and
  closes with them so the lines below do not jump.
- Lyrics blur and dim by how far a line sits from the one being sung on screen, not by how many
  lines away it is. The lines next to the sung one are already slightly soft, and every line further
  out is softer than the one before it, all the way to the edge of the panel. A line never fades so
  far that it cannot be read, and pointing at one still brings it back sharp. The panel also takes
  longer to carry the next line into place. Scrolling the lyrics by hand is unchanged.

### Removed

- The "Karaoke motion" setting is gone. Every word-by-word highlight now travels on the Smooth
  curve, which is what the setting defaulted to, so anyone who left it alone sees no change.

### Fixed

- Lyrics no longer run past the edge of the panel. Japanese, Chinese and Korean lines had no place to
  break, because a line was only ever split where it had a space, so a whole verse was laid out as
  one unbreakable row. They now break between characters, and never before a closing bracket or a
  mark like a comma that has to stay with the character in front of it.
- Non-Latin lyrics are set in the same weight as the rest of the text. Inter carries no Japanese,
  Chinese or Korean glyphs, so those came from whatever the system offered and arrived at regular
  weight next to bold Latin.
- Turning Karaoke lyrics on no longer widens the gaps between words. The line being sung was laid
  out one box per word so it could be highlighted piece by piece, and each box was measured on its
  own; it is now a single run per line with the highlight clipped over it, which is what every other
  line already did.
- The word-by-word highlight keeps pace with Japanese, Chinese and Korean lines, where it used to
  stall and then jump ahead to catch up. It travels on an eased curve, which reads as a flourish
  across a word of Latin letters but not across a single wide character, and lyrics for those
  languages are timed one character at a time or, worse, a whole phrase at a time: the highlight
  raced most of the way over a character and then crawled the rest. Wide characters and phrases now
  fill at an even pace, and the soft edge the highlight carries no longer hardens on every character
  it crosses, which was dragging the visible edge back half a character at a time. A word followed
  by a rest also finishes where it is sung instead of drifting on through the silence.
- Japanese, Chinese and Korean lyrics keep their spacing when a line's word timings do not line up
  with its text. Sonora fell back to spacing the words out as if they were English, which put a gap
  between every character.
- Album covers no longer leave a thin line across the player bar as they scroll past it. Card grids
  and the shelves on Home were drawing one row beyond the edge of what you can see, and a single
  pixel of it fell outside the clip.
- The Songs tab in Your Library no longer stutters on a large collection. Its play button was
  building a copy of every song in the list on every frame, so the bigger the library the worse it
  scrolled.
- A word-by-word lyric line never breaks in the middle of a word. Each word is laid out on its own
  so it would be squeezed rather than moved down when it did not fit, splitting "upon" across two
  lines as "u" and "pon". A word that does not fit now moves to the next line whole. Punctuation
  timed as its own word goes with it, so a line no longer wraps to leave a lone "?" or "," on the
  last row.
- Turning volume normalisation or gapless playback on or off keeps the song you are on. Both
  settings can only take effect on a fresh player, so Sonora used to drop the track and send you
  back to nothing; it now rebuilds the player and puts the song back where it was. A song that was
  paused stays paused rather than starting itself.
- YouTube tracks no longer leave a short silence between them. The audio YouTube serves carries a
  fraction of a second of encoder padding at each end, which nothing in the decoding stack was
  removing, so it played as a gap however early the next track was fetched. Sonora now reads how
  much to drop from the file itself and trims it.
- Imported tracks run into one another with no gap. The next track was already decoded and waiting
  behind the current one, but Sonora tore it down and started it again the moment the song changed,
  so every boundary cost a stumble.
- Spotify playback on Windows no longer hisses in the background, which was loudest at low volume.
  Sonora asked the output device for a stream it could not give and settled for the poorest sample
  format on offer instead of the one the device already runs at, so every track was played through
  a needlessly coarse output. It now opens the device at its own format, the way YouTube and
  imported tracks always have.
- Rows in the list view of Albums, Playlists and Artists can be dragged by their name. The name
  column swallowed the press, so the only place a row could be picked up from was its cover.
- Dropping something in the space between two pinned items in the sidebar, or between two queued
  songs, puts it there instead of at the end.
- The minimize and close buttons work on Windows, where a click on either did nothing. Maximize is
  still handed to Windows itself, which is what lets it restore a maximized window and open the
  Snap Layouts flyout on hover.
- Double-clicking the title bar maximizes the window and restores it when it is already maximized.
  Nothing happened on macOS before, because the first click started a window drag that swallowed
  the second.
- A lyric sheet that only says the song is instrumental is read as instrumental instead of being
  shown as the song's words, whatever case it is written in.

## [0.18.0] - 2026-08-25

### Added

- Word-by-word lyrics now come from Apple Music's own sheets first, through a public catalogue that
  needs no Apple account. They bring background vocals on their own line, the songwriters, and, on a
  duet, which of the two singers holds each line.
- A duet now reads as one. Lines sung by the second voice sit against the right edge of the panel,
  the way Apple Music lays them out, and their background vocals follow to that side. "Shallow",
  "Under Pressure", "Summer Nights" and "Ain't No Mountain High Enough" all split this way.
- Musixmatch is asked second and covers songs Apple has no sheet for, such as YOASOBI's 夜に駆ける.
  When Musixmatch decides the network is asking too often it stops answering, so Sonora leaves it
  alone for ten minutes and the other sources carry on.

### Changed

- Lyrics now appear as soon as the first source answers instead of once the slowest one has. In a
  255-track library the words show up after about 50 ms rather than 840 ms, because Spotify and
  LrcLib answer in well under a tenth of a second while Kugou and NetEase take one to three.
- A sheet on screen is only ever replaced by a better one. Plain text gives way to line-by-line
  timing and line-by-line to word-by-word, so a slow karaoke sheet still takes over when it lands.
  Between two sheets of the same kind the better match wins, so Apple's words replace a karaoke
  sheet that arrived first.
- Lyrics are kept between runs, so a song you have played before shows its words with no wait at
  all. The lyrics for the next song in the queue are fetched while the current one is still
  playing, so skipping forward usually costs nothing either.
- A karaoke sheet now borrows its words from the line-synced sheet already on screen and keeps only
  the word timing, so the line breaks, capitalization, punctuation and spelling stay exactly as they
  were and the switch to word-by-word highlighting is barely visible. In a 255-track library 47 of
  the 48 karaoke sheets match up this way; one that cannot be matched is shown as its provider sent
  it. Background vocals stay on their own line under the main one, and a sheet is only reshaped when
  nearly every line of it finds a match, so a song whose two sheets disagree about whole verses keeps
  the timing its provider gave it. A sheet that carries its own background vocals or duet
  voices is left exactly as it came, since reshaping it would flatten them.
- Verses from every source are capitalized the same way, so a sheet that no other source can shape
  still reads consistently.
- A sheet that appears part-way into a song puts the verse being sung straight where it belongs
  instead of scrolling down to it from the top. Verse-to-verse scrolling while a song plays is
  unchanged.
- Verses now ease into and out of focus as the song moves on, rather than snapping between sharp and
  blurred on the frame the line changes. The notes marking an instrumental break blur along with the
  verses around them instead of staying sharp.
- Settings gained "Karaoke motion", which chooses how the word-by-word highlight travels across each
  word: Steady keeps the constant speed it has always used, while Gentle, Smooth, Snappy and Glide
  ease it over a longer stretch, each easing off as it reaches the next word. Words whose timings sit
  on top of each other used to flip instantly; every eased setting sweeps them over about 150 ms
  instead, and a word is still fully lit by the time the next one starts, however long it is held.
- A verse now stays lit until the next one begins, rather than dimming the moment its last word has
  been sung. There is always a current verse on screen, so the gap between two karaoke lines no
  longer leaves the panel with nothing highlighted. An instrumental break still shows its notes in
  place of a verse.
- A karaoke line fades out when it finishes the same way a line-timed one does. It used to drop
  straight to its dimmed colour the moment the word-by-word highlight stopped drawing, and the
  panel briefly treated the pause between two karaoke lines as though the song had gone back to the
  first verse.

### Fixed

- The last word of a karaoke line is fully lit by the time the line ends. Every eased karaoke
  motion stretched the last sweep past the word itself, so a line that handed over to the next one
  mid-sweep lit its remaining words in a single frame.
- The soft edge trailing the karaoke highlight now narrows as the highlight lands on the end of a
  word, rather than disappearing in one frame once the word is fully lit.
- Credits are no longer shown as lyrics. Sheets that opened with a title and artist line, a version
  tag such as "Edited Version", a copyright notice, or a block of "Produced by", "Recorded by" and
  per-instrument credits now start at the first sung line.
- Right-clicking the song title in fullscreen opens the track menu when the window is too narrow to
  show the lyrics beside the artwork. Only the wider layout answered a right-click before.

## [0.17.1] - 2026-08-25

### Added

- The lyrics panel tells an instrumental track apart from one whose lyrics are simply missing: a
  guitar and "This song is instrumental" when a lyrics provider says the track has no words, and a
  crossed-out microphone and "No lyrics found, sorry!" when nobody has them.

### Changed

- Karaoke lyrics now come from Kugou as well as NetEase Cloud Music, and Spotify hands over its
  own lyrics for the track it is playing, matched by track rather than by title. Across a
  255-track library that lifts word-by-word coverage from 55% to 78% and leaves NetEase supplying
  a fifth of it rather than nearly all.
- The AMLL community database is no longer queried: it held a matching sheet for one track in a
  255-track library while costing a request for every track played.
- The lyric line being left behind fades out quicker than the incoming line fades in.

### Fixed

- Japanese romanized lyrics now read as words rather than as fragments: kanji keep their word
  readings (君 is "kimi", not "kun"), verb endings stay attached to their stem ("oshietekureta"
  instead of "oshie tekureta"), particles are romanized as they are spoken ("wa" for は, "o" for
  を), and small つ doubles the next consonant instead of appearing as "tsu".
- NetEase sheets no longer render their own metadata as lyrics: a leading "artist - title" line or
  a Title/Album/By block is dropped, and doubled spacing between words is collapsed.
- A sheet whose header names a different song is rejected, so a mismatched upload no longer
  replaces the right lyrics.
- Lyrics typed with lookalike Cyrillic letters are folded back to Latin, so they read as the words
  they imitate instead of picking up a spurious romanized line beneath them.
- Background vocal lines no longer show doubled spacing once they start being sung, and stray
  markup such as "<-3>" left in an upload is dropped instead of appearing mid-verse.

## [0.17.0] - 2026-08-24

### Added

- The theme picker offers a System option that follows the operating system's light or dark
  appearance, switching automatically when it changes.
- Lyrics now also come from the AMLL community database and NetEase Cloud Music alongside LrcLib,
  and the best match is chosen more reliably: the album is taken into account, truncated uploads
  fall behind, duplicate uploads collapse into one entry, and word-synced lyrics win over
  line-synced ones.
- The lyrics panel names its source and the songwriters beneath the verses.
- Word-synced lyrics light up word by word as they are sung, with a soft sweep across each word.
  Line-synced lyrics keep the existing line highlight.
- Background vocals in parentheses render as their own smaller line beneath the verse, with their
  own timing, and standalone echo lines attach to the verse they answer.
- Lyrics in Japanese, Chinese, Korean, Cyrillic, Greek or Arabic can show a romanized
  pronunciation line, switchable per writing system in Settings → Playback.
- Long instrumental breaks show notes in the lyrics panel that light up as the break plays out;
  clicking them seeks to the start of the break.
- Karaoke word highlighting can be turned off in Settings → Playback.
- Word-synced lyrics appear as soon as the first provider answers instead of waiting for the
  slowest one.
- Windows releases ship an installer alongside the portable binary.

### Changed

- The fullscreen panel switcher and volume popup are solid instead of translucent and frosted, so
  they read clearly over artwork.
- Lyric matching drops hits whose title, artist or length do not fit the playing track, along with
  sheets that are mostly stretched shouting, and section labels such as [Chorus] no longer render
  as verses.
- The playback clock eases toward corrections from the engine instead of snapping, so lyrics and
  the seek bar never jump backwards mid-word.
- The active lyric line brightens in and the previous one dims out with a short transition.

### Fixed

- Audio opens with the device's own output configuration, so devices that reject the previously
  assumed format play again.
- The volume bubble stays put while dragging, and fullscreen seeks preview the target position
  instead of waiting for the engine to catch up.

## [0.16.3] - 2026-08-24

### Fixed

- The About the artist dialog now shows the full biography in a scrollable pane that fades out at
  the bottom, instead of only the artist's name.
- Dialogs no longer spill past the window on narrow viewports; they always keep a margin from the
  edges.
- Synced lyrics hold the first and last line at the same follow position as every other line,
  instead of leaving them stuck at the top or the bottom of the panel.

## [0.16.2] - 2026-08-23

### Added

- Fullscreen controls fade away after a few seconds without mouse or keyboard activity, leaving
  only the cover, the track title and artists, and the lyrics or queue panel; the cover grows into
  the freed space. Any activity brings them back.

### Changed

- Track tables start out with roomier title, artist and album columns, so a fresh install needs
  less dragging before it reads well.
- In fullscreen, the cover column and the lyrics or queue panel now split the window evenly, with
  the same spacing between them as around them.

## [0.16.1] - 2026-08-22

### Added

- Playlists show when each track was added, in a sortable Date added column.

### Fixed

- The play overlay and cover darkening now really do appear on queue rows — the artwork was
  painted over them.
- Long single-artist names in a table are cut with an ellipsis instead of being sliced mid-letter.

## [0.16.0] - 2026-08-22

### Added

- Removing a track from a playlist shows a confirmation toast, mirroring the one for adding.

### Changed

- Track rows look and behave the same everywhere — queue, search results and quick picks share
  one entry: artwork with a play overlay on hover, a hover-underlined title, artist links, and in
  the queue a remove button on the right.
- Album, playlist and artist cards are built from one shared piece, so shelves, the library grid
  and search results no longer drift apart in typography or behaviour.

### Fixed

- The play overlay on track rows appears when you point at the row — including the queue, where it
  never showed — and its tooltip no longer pops over rows that are not being pointed at.
- Clicking the play control on a paused track resumes it from where it stopped; quick picks and
  search used to restart the track from the beginning.
- Queue rows no longer vanish abruptly inside the edge fade — they now dissolve with it — and the
  queue scrollbar is no longer dimmed by that fade.
- The library says when a section failed to load instead of presenting an empty table that looks
  like an empty library, and it says so per section — songs can arrive while artists fail.
- Removing a track from a playlist updates the playlist everywhere at once; the track count and
  the "Add to playlist" menu used to keep the old state until a full reload.
- A failed search no longer leaves its error on screen after signing out.
- Disabled buttons no longer show tooltips.
- Switches sit pixel-exact inside their track.
- YouTube Music plays again. YouTube stopped serving the clients Sonora asked for: the download of
  every track was refused part-way through, and the fallback it tried next was refused outright. A
  signed-in session now streams through the YouTube Music client itself, which also hands over a
  better stream — 256 kbps instead of 128. Without an account Sonora asks as a headset would, with a
  visitor id issued by YouTube rather than one it made up.
- Playing without a YouTube account says so once instead of failing track after track. YouTube now
  turns anonymous listeners away from most music, and Sonora used to work through the whole queue,
  waiting six seconds between refusals. It now stops at the first one and asks you to sign in.
- A signed-in YouTube session survives a restart. Sonora used to keep the cookies it copied from
  your browser the day you signed in, and YouTube rotates those every few hours, so your library
  quietly went missing. When you signed in through a browser, Sonora now re-reads its cookies each
  time it starts and keeps the copy fresh.
- Playing after the app had been open for a long time no longer says the track cannot be played.
  Suspending the machine, or any long enough network break, killed Sonora's connection to Spotify for
  good and every track after that failed to load. Sonora now notices the connection went stale and
  reconnects on its own, keeping the track you were on and resuming it where it stopped.

## [0.15.0] - 2026-08-16

### Added

- Appearance settings carry a Reduce motion choice — follow the system, always or never — so you can
  decide up front whether Sonora animates its interface.
- Appearance settings also carry an Animation speed choice — slow, standard or quick — that stretches
  or tightens every interface animation to taste.
- Fullscreen is a real player now: large artwork that swaps to the album's high-resolution cover as
  soon as it arrives, the track and its artists, a seek bar, transport controls, and a pill that
  puts lyrics or the queue beside the artwork — the same lyrics and queue as the side panel, with
  everything they can do. On a narrow window the chosen panel takes over the whole body instead.
  Press `f` to go fullscreen and Escape to come back to wherever you were.
- Lyrics read like a stage now: the lines around the one being sung are softly blurred, the line
  under the pointer sharpens as you reach for it, and the top and bottom of the list fade out
  instead of being cut off. Hovering a line still seeks to it.
- Appearance settings gained an Advanced group, starting with an adaptive context menu. Turned on,
  a track's menu leaves out what the row already shows — its album or its artist. It ships off.

### Changed

- Switches now slide and fade between on and off instead of snapping, at a fixed control speed that
  the animation speed setting deliberately leaves alone.
- Long lists, grids and the home shelves cost far less to draw, so scrolling them stays smooth
  where it used to stutter. Sorting and filtering a large library got cheaper too.
- A sign-in that fails now explains itself in plain words on a small card, on the login screen and in
  account settings — being outside your account country, an expired session, no connection, a
  cancelled browser approval — instead of showing the raw message from the streaming library. An
  unrecognised failure is trimmed down to one readable sentence too.

### Fixed

- Signing in with an account that has no Spotify Premium closed Sonora outright, and it kept closing
  on every launch until the cached session was deleted by hand. Sonora now stays open, forgets that
  session and explains on the login screen that streaming needs Premium.
- The volume percentage now sits above the slider handle instead of trailing the pointer, so it
  reads as a label on the handle you are dragging.
- A shelf on home — Your Mood Mixes — arrived from Spotify with no names and no covers and drew as a
  row of blank cards. Those mixes are now filled in from the playlists themselves, in the background,
  so the rest of home appears straight away.
- The window buttons on macOS sat in the wrong place.

## [0.14.0] - 2026-08-14

### Added

- Searching now reaches the whole catalogue for albums and playlists, not just the ones your library
  already knows about. The third column of results holds both, tagged so you can tell them apart,
  and a playlist behaves like one everywhere: open it, play it, pin it or right-click it for the
  usual menu. Albums and playlists you have saved still come first. Spotify and YouTube Music both
  answer these searches from their own catalogue.

### Changed

- The songs, artists and albums columns in search scroll smoothly however many matches come back,
  and each column still scrolls on its own.

### Fixed

- Sonora sat redrawing the window as fast as the screen allowed whenever the lyrics panel was open,
  heating the machine and draining the battery while nothing on screen was moving. It now rests when
  there is nothing to draw.
- When Spotify ships a new build of its web player, album, artist and search pages recover on the
  next attempt instead of failing for hours.

## [0.13.0] - 2026-08-14

### Added

- Sonora remembers what you were listening to. Reopen it and the track you left off on is waiting,
  paused where you stopped, with the rest of the queue and the tracks you already heard still in
  place. It is readied silently in the background, so press play and the music starts from that
  second at once, without the progress bar sliding into place first.
- Double-clicking a word in any text field selects it, and a third click selects everything in the
  field.
- The search field carries a clear button while it holds text, so one click empties it and brings
  the full list back.

### Changed

- The home feed, genre pages and the genre grid in search scroll and resize without stuttering,
  however many shelves the feed carries.
- The single-column search results scroll smoothly however many matches come back, because only the
  rows on screen are drawn.

### Fixed

- Switching music service, or signing out, now clears the queue, the history and the current track,
  so nothing from one service is left sitting in the player when you move to another. Music from
  your imported folder keeps playing.
- YouTube tracks that refused to play now do. When YouTube turns down the quick route, Sonora
  works out the stream signature itself and plays the track anyway, often at a higher bitrate.
- Pinned items now belong to the service they came from, so switching service shows that service's
  pins alongside your imported ones instead of dead rows that failed to open. Switching back brings
  the earlier pins straight back, and existing pins are cleared once on this upgrade.
- Search results no longer all look like the track that is playing: only the song actually playing
  is highlighted and shows a pause button.
- Song titles in search results are readable again instead of being cut after a few letters: the
  three columns only appear once there is real room for them, and a song's duration no longer takes
  space away from its title.
- Every search result answers a right-click now, not just songs. Artists and albums open the same
  context menu they have elsewhere in the app, in the combined list as well as the three columns.
- The genre grid on search scrolls all the way to its end, so the last row of plates no longer sits
  flush against the player bar with its bottom cut off.
- Album, playlist and artist lists in your library end with the same breathing room as every other
  page instead of stopping against the player bar.

## [0.12.1] - 2026-08-13

### Fixed

- The queue scrolls with the same glide as the rest of the app instead of jumping.
- The language picker is wide enough for its search field.

## [0.12.0] - 2026-08-13

### Added

- Search now opens on a grid of genres, and every genre leads to its own page of playlists, albums
  and sub-genres, in cards or in a compact list.
- The home page carries a feed from the service you are signed in to — daily and artist mixes,
  editorial playlists and discovery shelves — with placeholder shelves while it loads, so the page
  is useful before you like a single song.
- German and French translations.
- The language picker has a search field, so a language is one keystroke away instead of a scroll.

### Changed

- Scrolling glides to a stop instead of jumping, at the same speed on any refresh rate. Holding
  Shift over a shelf scrolls it sideways, and a shelf no longer steals a plain wheel scroll.
- Searching keeps what is already in your library above the catalogue results, so a match stops
  sinking the moment the service answers.
- Artist names in the narrow search layout are links again, and a table column stays hidden when
  no row carries a value for it.
- Sonora describes itself as a music streaming client rather than a Spotify client.

### Fixed

- Home, the genre pages and the genre grid only build what is on screen, which removes the stall
  on pages with many covers.
- Opening a genre on YouTube Music no longer crashes on the texture atlas.
- Genre pages that carry nothing playable, and podcast-only entries, disappear from the grid by
  themselves.
- The Nix package installs the released binary instead of wrapping the loader, so audio mixers and
  process lists show Sonora rather than ld-linux.

## [0.11.0] - 2026-08-13

### Added

- Importing a YouTube Music session, or pasting cookies, now asks which Google account to use when
  the session is signed in to more than one, and Sonora stays on the account that was picked.
- The "Paste cookies manually" dialog spells out where the value comes from — which request to open
  in the developer tools, which header to copy, and which cookies the value has to carry.

### Changed

- Importing a browser session is limited to Firefox-based browsers, which the login screen now says
  under the button. Chrome, Edge, Brave and the other Chromium browsers are no longer offered;
  paste the cookies manually to sign in from one of those.
- Connecting a service from the settings now happens there: the cookie dialog, the browser and
  account pickers and a cancel button all appear in Manage accounts, and a failed attempt reports
  the reason on that service's card.
- The queue and the lyrics are opened from a pair of buttons in the player bar instead of the
  floating pills inside the sidebar, and pressing the button of the panel already on screen closes
  the sidebar again.
- An artist page opens with the same card deck the home screen uses for quick picks: 30 popular
  songs across paged columns of five, each card playing the artist's popular list from that song and
  showing the track length. Names below a title appear only where the song is shared with someone
  else. A button in the toolbar switches the section back to the old table, and the choice is
  remembered.
- The card deck keeps one height whatever it holds — a short last page leaves empty rows instead of
  shrinking — and quick picks on the home screen now mixes 30 songs to fill it.
- The artist page reads as an overview: the table view lists five songs and expands to ten, releases
  show two rows and expand to the whole discography, and the artist's biography closes the page in
  the same card the song page uses.
- The "About the artist" card clamps the biography to three lines and opens the full text in a
  dialog; from a song page that dialog also offers a jump to the artist.
- The library lists what changed last first: saved albums carry the date they were saved, playlists
  the date they were last edited, and albums, playlists and followed artists all open sorted by that
  date until a different sort is chosen. Both dates are new sortable columns.
- A narrowing table now gives up one column at a time, in order of how much each column matters,
  instead of dropping three of them at one width and squeezing the rest. The artist of a song is the
  first to go, since the album column carries the same information.

### Fixed

- Connecting a service from the settings no longer throws you onto the login screen, and a failure
  no longer signs you out of the service you were already using.
- Waiting for a browser authorization can be cancelled instead of leaving the app stuck until it is
  restarted, and cancelling now shuts the callback server down. Signing in again used to fail with
  "Address already in use" until Sonora was restarted.
- Cancelling a sign-in no longer flashes the empty library behind the login screen.
- Artwork that does not arrive square — an artist portrait, a cover embedded in a local file — is
  cropped to its middle instead of spilling out of its frame, so a round portrait is round again.
- A pinned table header no longer trembles by a pixel while the page scrolls, and the wheel now
  works over the header and over a column edge instead of stopping there.
- An artist page lists the whole discography — every album, single, compilation and alternate
  edition such as a deluxe or anniversary release — instead of only the most recent releases.

## [0.10.0] - 2026-08-12

### Added

- Anything can be pinned to the sidebar. Drag an album, artist, playlist or song out of a grid, a
  table, a search result, the queue or a page header and drop it into the left sidebar; it stays
  there across restarts, reorders by dragging, and opens a context menu that matches what it is.
- Playlists that arrive without artwork get a cover of their own, stitched together from the first
  four tracks and kept on disk so it is built only once.
- Content cards carry a play control on their artwork — a button in the corner of a tile, a dimmed
  cover on a row — and it stays visible while that item is playing.
- The left sidebar scrolls once its contents outgrow the panel.
- Local music reads more formats, gains its own toolbar, and keeps the list header in place while
  scrolling.
- A playlist that already holds a track marks it, and adding a second copy asks first.

### Changed

- Playlist covers are fetched at the largest size the service offers rather than the smallest.
- The appearance settings put the opacity and adaptive theme switches above the font size.

### Fixed

- The edge used to resize a side panel no longer competes with the row sitting underneath it.
- A playlist that lists the same track more than once keeps every copy.
- The lyrics pane returns to the top when a new track starts, and its scrollbar stays asleep until
  it is needed.
- A menu item that carries only a tick no longer reserves room for an icon beside it.
- Rounded corners nest correctly inside the container that clips them.

## [0.9.0] - 2026-08-11

### Added

- Sonora can play the music already on this device: point it at a folder in the settings and its
  files show up as tracks, albums and artists next to the streaming services.
- The right sidebar shows the lyrics of the playing track next to the queue, switched with a pair of
  floating pills at its foot. Timed lyrics highlight the line being sung, scroll along with it, and
  jump playback to a line when it is clicked.

### Changed

- The right sidebar is opened from the toolbar rather than the player bar, and the player bar button
  in its place opens fullscreen instead.
- A narrow window drops the right sidebar entirely rather than letting it cover the page; lyrics and
  the queue live in fullscreen there.

## [0.8.0] - 2026-08-11

### Added

- Sonora can now sign in to YouTube Music as an alternative to Spotify. The login screen offers
  both services; YouTube Music can be browsed as a guest, connected by importing an existing
  browser session, or connected by pasting cookies, and the whole library — liked songs,
  playlists, albums, artists, search, and radio — works through the same interface.
- The general settings gain a "Manage accounts" section: every service is listed with its own sign
  out, switching to a service already connected takes effect immediately, and a service that is not
  connected yet offers its sign-in options right there — including importing from a named browser.
- The login screen puts each service in its own column with guest mode below them, and importing a
  YouTube Music session asks which browser to read it from — Firefox, Zen, LibreWolf, Floorp,
  Waterfox, Mullvad, Tor, Pale Moon, Basilisk, SeaMonkey, Chrome, Chromium, Brave, Edge, Vivaldi,
  Opera, Yandex, Arc, Thorium and Helium are recognised, including Flatpak and Snap installs.
- YouTube Music albums and playlists play the album audio of a song rather than its music video, so
  a track lasts as long as its listed length.
- Switching, pausing, resuming, and seeking a YouTube Music track fade in and out instead of
  clicking, the previous track stops the moment a new one is picked, and the transport stays
  responsive while the next track loads.

### Changed

- Release cards on an artist page show the release year instead of repeating the artist's name.

### Fixed

- A disabled button reads clearly instead of fading its label into its own background.
- Pasting YouTube Music cookies happens in a dialog that can be dismissed, so a sign-in started by
  mistake no longer leaves the login screen waiting for input.
- Signing out of one service switches to another service still connected, and only falls back to the
  login screen once nothing is connected.
- YouTube Music playlists you own are recognised as yours, so renaming, changing visibility and
  deleting are offered instead of an unsupported "remove from library" that always failed.
- Creating a YouTube Music playlist reports success instead of an error, and the new playlist
  appears straight away rather than after a refresh.
- Removing a track from a YouTube Music playlist works for tracks whose music video was swapped for
  its album audio.
- A YouTube Music playlist you own no longer lists its privacy setting as its owner.
- The player switches to the next YouTube Music track the moment the previous one ends, instead of
  lagging up to half a second behind the audio.
- A YouTube Music session whose saved cookies stopped working falls back to guest browsing or the
  login screen instead of failing with an error.

## [0.7.0] - 2026-08-10

### Added

- Sonora keeps a log file at `$XDG_STATE_HOME/sonora/sonora.log`, so a problem noticed after hours of
  use can still be diagnosed without having started the app from a terminal.
- Playback failures explain themselves: a track that cannot be played is named in a toast, and if
  Spotify refuses the account playback keys entirely, Sonora says so once and stops instead of
  failing through track after track.

### Fixed

- Sonora holds on to far less memory during long listening sessions: cover art it has not shown for
  a while is released instead of being kept until the app closes, and cover art whose page was left
  before the download finished no longer stays in memory for the rest of the session.
- The left sidebar keeps the width you gave it. A window too narrow to fit it now hides it and brings
  it back at that same width, instead of squeezing it down to its narrowest and forgetting the width
  you had; it can still be resized while it sits over the content.
- Card views stay responsive in large libraries: album, song and playlist grids now draw only the
  rows on screen, so resizing the window no longer stutters or reloads covers, and an artist's
  discography opens without a pause.

## [0.6.0] - 2026-08-10

### Added

- Sonora answers the system media controls: media keys, the desktop's now-playing widget and
  lock-screen controls can play, pause, skip, seek and set the volume, and they show the current
  track with its cover art.
- Spotify links open in Sonora: a `spotify:` link to a track, album, playlist or artist opens that
  page, handing it to the window already running instead of starting a second one.

### Changed

- The app presents itself as Sonora rather than sonora, in the window title, the application menu
  and the Windows file properties.

## [0.5.0] - 2026-08-09

### Added

- Albums and playlists can be added to and removed from the library, from their context menu and
  from a heart button beside Play on their page.
- A transparent window background, with an opacity slider under Appearance settings.
- Settings are split into General, Appearance, Playback and About tabs, reachable from the sidebar.
- Copy link actions for tracks, albums, playlists and artists.
- Monthly listeners on the artist page, and play counts beside an artist's popular tracks.
- Toasts confirming queue and playlist additions.
- The volume can be set by scrolling the mouse wheel over the volume control.
- Turning on radio appends what it will play to the queue and lists it under a Similar tracks
  section, picked from the last track you queued. Those tracks play, reorder and can be removed like
  any other queue entry.
- Tooltips on icon-only controls across the player, title bar, toolbar and queue.
- The next track is fetched before the current one ends, so it starts without a gap.
- Gapless playback, on by default and switchable under Playback settings: an album runs from one
  track into the next the way it was sequenced, instead of being cut at the track boundary.

### Changed

- Settings that are on or off are switches instead of text buttons.
- Context menu entries are grouped into sections separated by a rule.
- The sidebar always collapses when the window gets narrow; the setting that governed it is gone.
- Skipping quickly through several tracks only loads the one you stop on.
- The seek and volume sliders have a taller grab area, so they are easier to hit without looking
  any thicker.
- Library cards are virtualized, artist release artwork loads only once it is scrolled to, and the
  artwork image cache is bounded, so large libraries stay responsive.
- Headings, quick pick titles and queue track names are lighter and no longer bold.
- Releases ship static Inter faces instead of the variable font.
- Song, artist and album pages load their details in fewer requests.

### Fixed

- Radio builds its set of similar tracks once, instead of replacing it on every track it plays.
- Durations in search results stay on one line.
- Library cards keep their scroll position when you come back to them.
- The settings menu in the sidebar folds away when you navigate off settings.
- The Play button on a page plays the list as it is shown, so filtering or sorting no longer starts
  a track that is not in view.
- An empty library section says so instead of showing a bare table, and a filter that matches
  nothing says that too.
- Quick picks asks you to like a few songs once the library has loaded, instead of pulsing
  placeholders forever.
- The no-matches note in search lines up with the results above it.
- Clicking a dropdown trigger below the title bar closes its menu instead of leaving it open.
- Releases on the artist page answer to a right-click with the album menu.
- The percentage and timecode bubbles no longer appear below a slider, where a click would not land.
- Rows in quick picks, the queue and search results are inset by the same amount on all four sides,
  instead of drifting a pixel between the top and the bottom.
- Timestamps an hour or longer carry an hours field instead of overflowing the minutes.
- Track menus no longer offer a link to the page that is already open.
- A right-click on a table row no longer also opens the menu behind it.
- Artist release artwork no longer disappears while the page scrolls.
- The library card grid is padded on both sides.
- Play counts of zero are treated as unknown rather than shown as zero.

## [0.4.1] - 2026-08-09

### Added

- An About page crediting the team.

### Changed

- Shuffle, repeat and whether the queue panel is open are remembered between sessions.

### Fixed

- Play next inserts after the current track instead of appending to the queue.
- Resizing one side panel no longer resizes the other.
- A panel responds only to its own drag grip.
- A button label truncates instead of overflowing its button.
- Long song metadata is contained rather than spilling out of its row.

## [0.4.0] - 2026-08-09

### Added

- Playlist management: create, rename, delete, change visibility, and add or remove tracks.
- Albums and playlists can be queued as a whole from their menus.
- Library cards are laid out on a grid that adapts its column count to the space available.
- Toasts above the player report playlist changes.

### Changed

- Item menus are built from one shared definition rather than per-screen copies.
- Track columns are composed from one shared set across every table.
- Playlist edits update the library in place instead of reloading it.
- The library Songs tab stays in list mode, where a card grid adds nothing.
- The text input moved into the design system, and side panels are built on one panel primitive.

### Fixed

- Radio started from a search result is seeded with the track that was picked.
- A newly created playlist is named on creation instead of appearing untitled.

## [0.3.0] - 2026-08-08

### Added

- Columns can be reordered by dragging a header and resized by dragging its edge. Widths, order,
  hidden columns and the active sort are remembered per table between sessions.
- Library sections switch between the table and a card grid through a new View control, remembered
  per section.
- A sort control in the toolbar, so sorting works in card mode as well as in the table.
- Card mode groups rows under first-letter or year headings when the active sort is groupable.
- Album and playlist artwork carries a Play button that starts, pauses and resumes in place.
- Go to album and Go to artist in the track context menu; the latter opens a submenu when a track
  has several artists.
- Queued tracks gained clickable artists, a remove button on hover and the full track menu.
- Liked tracks, with like icons shown on tracks inside albums and playlists.

### Changed

- Track, album and playlist cards are now one primitive rather than four parallel implementations.
- Right-click context menus are a separate element from button dropdowns.
- Every icon was refreshed from Lucide 1.30.0, and the sidebar toggle moved to the panel-left family
  so its divider and arrow match a left-hand panel.
- Album cards show the artist instead of the release type.

### Fixed

- Column headers no longer overlap each other; a column either shows its full heading or is dropped.
- Clicking a menu trigger a second time closes the menu instead of reopening it.
- Selecting an option no longer dismisses the filters, columns or sort dropdown, so several options
  can be picked and the duration slider can be dragged.
- Right-clicking another row moves the context menu in one click.
- Submenus no longer block clicks across the whole window.
- Ghost and outline buttons show a visible hover on the player bar, which is painted in the same
  colour their hover used to be.

## [0.2.0] - 2026-08-07

### Added

- Added an About tab in settings carrying the copyright, the warranty disclaimer and links to the
  license and the source.
- Added `THIRD-PARTY.md`, listing every bundled dependency and the full text of every license.
  Packages and release archives now ship it alongside the Inter and Lucide license files.
- Added a Nix package that installs the prebuilt release binary instead of compiling GPUI.

### Changed

- **Licensing.** Sonora is now released under the GNU General Public License version 3 or later.
  Earlier releases carried no license file at all, which left them undistributable; GPUI depends on
  `zlog` and `ztracing` from the Zed repository, both GPL-3.0-or-later, so every binary ever built
  from this tree was already covered by the GPL. Versions 0.1.0 and 0.1.1 are therefore to be read
  as GPL-3.0-or-later as well.

### Fixed

- Spaced the sidebar tabs apart and drew menus on the popover surface.

## [0.1.1] - 2026-08-07

### Added

- Localized the interface with Fluent, shipping English, Russian, Ukrainian and Polish.
- Added a language setting that follows the system locale by default.
- Added an application icon for macOS, Linux and the disk image.

### Changed

- Releases now ship bare executables for Linux and Windows and a universal disk image for macOS.
- Shuffle moved into the queue, so the queue panel shows the order that will actually play.

### Fixed

- Capped the volume taper at unity gain; the top of the slider no longer clips.
- Aligned the scrubber thumb with the pointer across the whole track.

## [0.1.0] - 2026-08-07

Initial release: a native Spotify client with playback, an interactive queue, the saved library,
search, album, playlist, artist and song pages, context menus and adaptive theming.

[unreleased]: https://github.com/nolight132/sonora/compare/v0.31.0...HEAD
[0.31.0]: https://github.com/nolight132/sonora/compare/v0.30.0...v0.31.0
[0.30.0]: https://github.com/nolight132/sonora/compare/v0.29.0...v0.30.0
[0.29.0]: https://github.com/nolight132/sonora/compare/v0.28.1...v0.29.0
[0.28.1]: https://github.com/nolight132/sonora/compare/v0.28.0...v0.28.1
[0.28.0]: https://github.com/nolight132/sonora/compare/v0.27.0...v0.28.0
[0.27.0]: https://github.com/nolight132/sonora/compare/v0.26.0...v0.27.0
[0.26.0]: https://github.com/nolight132/sonora/compare/v0.25.0...v0.26.0
[0.25.0]: https://github.com/nolight132/sonora/compare/v0.24.1...v0.25.0
[0.24.1]: https://github.com/nolight132/sonora/compare/v0.24.0...v0.24.1
[0.24.0]: https://github.com/nolight132/sonora/compare/v0.23.0...v0.24.0
[0.23.0]: https://github.com/nolight132/sonora/compare/v0.22.0...v0.23.0
[0.22.0]: https://github.com/nolight132/sonora/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/nolight132/sonora/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/nolight132/sonora/compare/v0.19.1...v0.20.0
[0.19.1]: https://github.com/nolight132/sonora/compare/v0.19.0...v0.19.1
[0.19.0]: https://github.com/nolight132/sonora/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/nolight132/sonora/compare/v0.17.1...v0.18.0
[0.17.1]: https://github.com/nolight132/sonora/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/nolight132/sonora/compare/v0.16.3...v0.17.0
[0.16.3]: https://github.com/nolight132/sonora/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/nolight132/sonora/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/nolight132/sonora/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/nolight132/sonora/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/nolight132/sonora/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/nolight132/sonora/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/nolight132/sonora/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/nolight132/sonora/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/nolight132/sonora/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/nolight132/sonora/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/nolight132/sonora/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/nolight132/sonora/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/nolight132/sonora/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/nolight132/sonora/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/nolight132/sonora/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/nolight132/sonora/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/nolight132/sonora/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/nolight132/sonora/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/nolight132/sonora/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/nolight132/sonora/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/nolight132/sonora/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/nolight132/sonora/releases/tag/v0.1.0
