//! A few sample lines of lyrics drawn by the lyrics sheet's own renderer the way Now playing
//! draws them, held still in the middle of a karaoke line so every lyrics setting shows the
//! moment it changes. The line being sung is in Japanese, so romanization has something to add.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, Pixels, Window, div};
use music::{LyricsLine, LyricsWord, RomanizedText, Voice, WritingSystem};
use state::RomanizationScripts;
use ui::{ActiveTheme as _, Theme};

use crate::chrome::{VerseLook, verse, verse_sizes};
use crate::screens::settings::{Layout, SettingsView};

/// How tall the preview stands for its width.
const ASPECT: f32 = 0.4;
/// How many lines show either side of the one being sung.
const AROUND: usize = 2;
/// How far down the preview the line being sung starts, as a share of its height.
const ANCHOR: f32 = 0.36;
/// The haze curve of the sheet, from a line's distance to the one being sung.
const HAZE: f32 = 0.45;
/// How long the sample's sung line takes, and how far into it the preview stands.
const SUNG: Duration = Duration::from_millis(3200);
const STILL: f32 = 0.55;
/// How far the sheet fades in from its top and bottom edges, in verse sizes, as the panel does.
const VERSE_FADE: f32 = 1.25;
/// How far apart the sample's lines start, and which of them is being sung.
const SPACING: Duration = Duration::from_secs(4);
const SUNG_LINE: usize = 2;
/// The sample's lines by their Fluent keys, and the one in Japanese with its romanization.
const SAMPLE: [&str; 5] = [
    "settings-lyrics-sample-first",
    "settings-lyrics-sample-second",
    "settings-lyrics-sample-japanese",
    "settings-lyrics-sample-fourth",
    "settings-lyrics-sample-fifth",
];
const ROMANIZED: &str = "settings-lyrics-sample-romanized";

/// What the Lyrics page's preview keeps between frames, which is nothing, since it never moves.
pub(in crate::screens::settings) struct LyricsPreview;

/// The lyrics settings the preview follows.
#[derive(Clone, Copy)]
struct Look {
    karaoke: bool,
    blur: bool,
    scripts: Option<RomanizationScripts>,
}

impl LyricsPreview {
    pub(in crate::screens::settings) fn new(_cx: &mut Context<SettingsView>) -> Self {
        Self
    }
}

/// The Lyrics page's preview: the sample lines as Now playing draws them at its text size, over
/// the ambient field while that is on.
pub(in crate::screens::settings) fn preview(
    _state: &LyricsPreview,
    view: &SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Option<AnyElement> {
    let height = (Layout::of(window, cx).width * ASPECT).round();
    let field = view.settings.read(cx).ambient().then(|| {
        let cover = view.previews.playing_or_picked(cx).large_cover(cx);
        view.previews.ambient(cover, cx)
    });
    // over the field the lines wear what fullscreen would, not the theme around them
    let theme = match &field {
        Some(field) => field.read(cx).theme(cx),
        None => *cx.theme(),
    };

    let settings = view.settings.read(cx);
    let look = Look {
        karaoke: settings.karaoke_lyrics(),
        blur: settings.blur_lyrics(),
        scripts: settings
            .romanized_lyrics()
            .then(|| settings.romanization_scripts()),
    };
    let sizes = verse_sizes(false, settings.fullscreen_lyrics_scale(), &theme);
    let sheet = div()
        .relative()
        .w_full()
        .h(height)
        .bg(theme.background)
        .children(field)
        .child(lines(look, sizes, height, theme));

    Some(super::framed(sheet, cx).into_any_element())
}

/// The sample lines around the one being sung, drawn the way the sheet draws them at the verse
/// and lane sizes in `sizes`, over the whole of a parent `height` tall. The one being sung starts
/// at `ANCHOR` of the height whatever the sizes, so a large text size never pushes it out of view.
fn lines(look: Look, sizes: (Pixels, Pixels), height: Pixels, theme: Theme) -> Div {
    let lines = sample();
    let (size, lane) = sizes;
    let position = lines[SUNG_LINE].start + SUNG.mul_f32(STILL);
    let verse_at = |index: usize| {
        let sung = index == SUNG_LINE;
        let distance = index.abs_diff(SUNG_LINE) as f32;
        let haze = match look.blur && !sung {
            true => (distance / AROUND as f32).min(1.).powf(HAZE),
            false => 0.,
        };
        verse(
            &lines[index],
            VerseLook {
                active: sung,
                size,
                lane,
                karaoke: look.karaoke,
                scripts: look.scripts,
                haze,
            },
            position,
            theme,
        )
    };
    let fade = size * VERSE_FADE;
    let anchor = height * ANCHOR;

    div()
        .absolute()
        .inset_0()
        .when(ui::effects(), |this| this.fade_edges(fade, fade))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(height - anchor)
                .pb_4()
                .px(theme.metrics.inset)
                .flex()
                .flex_col()
                .gap_4()
                .children((SUNG_LINE - AROUND..SUNG_LINE).map(verse_at)),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(anchor)
                .px(theme.metrics.inset)
                .flex()
                .flex_col()
                .gap_4()
                .children((SUNG_LINE..lines.len()).map(verse_at)),
        )
}

/// The sample lines, timed so the third is being sung and the ones before it are over.
fn sample() -> Vec<LyricsLine> {
    SAMPLE
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let text = i18n::lookup(key, None).to_string();
            let start = SPACING * index as u32;
            let romanized = (index == SUNG_LINE).then(|| RomanizedText {
                text: i18n::lookup(ROMANIZED, None).to_string(),
                writing_system: WritingSystem::Japanese,
            });
            LyricsLine {
                start,
                end: Some(start + SUNG),
                words: Some(timed(&text, start)),
                text,
                romanized,
                secondary: Vec::new(),
                voice: Voice::Lead,
            }
        })
        .collect()
}

/// Splits a line into words sung one after another across `SUNG`, by its spaces where it has
/// them and letter by letter where it has none, each word taking time for its length.
fn timed(text: &str, start: Duration) -> Vec<LyricsWord> {
    let words: Vec<String> = match text.contains(char::is_whitespace) {
        true => text.split_whitespace().map(str::to_owned).collect(),
        false => text.chars().map(String::from).collect(),
    };
    let letters = words
        .iter()
        .map(|word| word.chars().count())
        .sum::<usize>()
        .max(1);
    let mut at = start;
    words
        .into_iter()
        .map(|word| {
            let length = SUNG.mul_f32(word.chars().count() as f32 / letters as f32);
            let timed = LyricsWord {
                start: at,
                end: at + length,
                text: word,
            };
            at += length;
            timed
        })
        .collect()
}
