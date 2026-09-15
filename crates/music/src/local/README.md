# Local music documentation

## How does local music library work
1. Multiple folders can be added; the library is built from every folder's tracks together.
2. `Track` identity is the file system path. `Artist` is a normalized string, `Album` a normalized
   `(artist, name)` pair.
3. Metadata fallback, in order: tag (`lofty`) -> `symphonia` -> `id3.rs` (hand-rolled, ID3v2 only,
   last resort) -> filesystem hint -> raw filename.
4. Cover art: embedded picture -> `.jpg`/`.png` etc. in the folder nearby.
5. Artist portraits: filesystem only, no tag equivalent. A scanned folder whose name normalizes to
   a resolved artist's name, with a cover-like image beside it, becomes that artist's portrait. No
   match, no portrait (UI falls back to initials).
6. `album_artist` (the real grouping key for albums, not the per-track `artist`) follows the same
   cascade: tag `AlbumArtist` -> `symphonia` -> the track's own resolved `artist` as default.
7. No incremental index — every scan (startup, add/remove folder, Rescan) rebuilds `Scanned` from
   scratch. Folders are merged *before* grouping, so an artist/album split across folders still
   merges into one entry.

## Playback

`wire::id3v2_end` skips a leading ID3v2 tag before handing the file to `rodio` — audio doesn't need
it, and it sidesteps taggers that write a frame neither `symphonia` nor `lofty` can parse (see
below). Used directly in `playback::load`, and retried inside `probe_symphonia` so duration can
still be recovered from the audio frames even when the tag itself is unreadable.

## Malformed ID3v2 tags (`id3.rs`)

Some taggers (gamerip tools, at least one seen: `#gamemp3s`) write a `WXXX` frame with no encoding
byte — a raw URL where the encoding + description should be. `lofty` and `symphonia` both abort the
*whole* tag over that one frame. `id3.rs` is a minimal, hand-rolled reader that trusts each frame's
declared size and skips anything it doesn't need, so one bad frame no longer costs every other,
well-formed one.

- Reads only `TIT2`/`TPE1`/`TALB`/`TPE2`/`TRCK`/`TPOS`/`TYER`/`TDRC`/`APIC`.
- ID3v2.3/2.4 only; no extended header, no unsynchronisation — none of the real files needed them.
- Wired in as a last resort in `track_from_file` and `tag_year`, only once `lofty` has already
  failed to open the tag.

Deliberately **not** extended to `tags.rs` (the tag editor's read/write): fixing a tag this broken
would mean writing a fresh one from scratch, discarding the old bytes entirely — real risk to a
user's file for a case nobody's actually hit. An external tool (Mp3tag, Picard) does that safer than
we can.

## A lying Xing frame count (`wire::has_lying_xing_frame_count`)

Some mp3s carry a Xing/Info VBR header with a declared frame count of `0` instead of omitting it —
seen on files an old `ffmpeg`/`libavformat` (`Lavf54.20.4`) remuxed from a DASH/YouTube source,
which never goes back to patch the real count into a non-seekable pipe output. `symphonia` trusts
that count for gapless trimming: a declared `0` trims every packet in the track down to nothing, so
the file loads with no error and then plays silence end to end even though the MPEG frames that
follow decode fine. `playback::load` checks for this specific lie and turns gapless off only for a
file that has it, so a well-formed file keeps its LAME encoder delay/padding trim.

## Embedded lyrics (`lyrics.rs`)

`LocalLyrics` is a `LyricsProvider` like the online services, listed as Local under Settings →
Playback → Lyrics providers. With it enabled it answers only for local tracks, reading the file's
tags through `tags::lyrics`:

1. An ID3v2 `SYLT` frame (MP3 only), with millisecond stamps. A frame typed as chords, events or
   trivia is skipped. A newline at either end of an entry breaks the line, so a frame timed by
   syllable keeps its words for karaoke.
2. Otherwise the lyrics tag (`USLT`, Vorbis `LYRICS`, …): timed when it parses as LRC, plain text
   when it doesn't.

- It competes in the ranking with the other providers. Its trust wins most ties against a sheet of
  the same kind, never against a better kind (timed over plain, word by word over timed).
- Prefer local lyrics, shown only while Local is enabled, reads the file first and asks no service
  when it has lyrics.
- Never cached: `lyrics.json` holds only the services' sheets, so a tag edit shows on the next play.
  A local track whose services already answered rereads just the file.
- Lyrics for local files holds back only the online services. Local still answers, since the file
  never leaves the computer.
