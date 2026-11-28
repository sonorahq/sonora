use gpui::{App, ClipboardItem};
use music::Lyrics;
use state::{Outcome, Toasts};

/// Copies the original text of the whole sheet, including any backing vocals.
pub(crate) fn copy_lyrics(lyrics: Lyrics, cx: &mut App) {
    let text = plain_text(lyrics);
    if text.trim().is_empty() {
        Toasts::show(Outcome::Info, "lyrics-missing", cx);
        return;
    }
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    Toasts::show(Outcome::Done, "toast-lyrics-copied", cx);
}

fn plain_text(lyrics: Lyrics) -> String {
    match lyrics {
        Lyrics::Plain { text, .. } => text,
        Lyrics::Synced { lines } => lines
            .iter()
            .flat_map(|line| {
                std::iter::once(line.text.as_str())
                    .chain(line.secondary.iter().map(|lane| lane.text.as_str()))
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
