//! A few sample lines of lyrics drawn the way Now playing draws them, held still in the middle
//! of a karaoke line so every lyrics setting shows the moment it changes. The line being sung is
//! in Japanese, so romanization has something to add.

use music::WritingSystem;
use rune::{Cx, El, Span, TextStyle, Transition, col, el, rich, text};
use state::RomanizationScripts;
use ui::{LEADING, Text, Theme, Weight};

use super::{DIMMED, framed, playing_or_picked};
use crate::Ui;
use crate::screens::settings::Layout;

/// How tall the preview stands for its width.
const ASPECT: f32 = 0.4;
/// How many lines show either side of the one being sung.
const AROUND: usize = 2;
/// How far down the preview the line being sung starts, as a share of its height.
const ANCHOR: f32 = 0.36;
/// The haze curve of the sheet, from a line's distance to the one being sung.
const HAZE: f32 = 0.45;
/// How far into the sung line the preview stands, as a share of its words.
const STILL: f32 = 0.55;
/// How far the sheet fades in from its top and bottom edges, in verse sizes, as the panel does.
const VERSE_FADE: f32 = 1.25;
/// Which of the sample's lines is being sung.
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
/// How much larger Now playing sets its verses than the title step, and the line being sung
/// over the rest.
const FULLSCREEN_GROWTH: f32 = 3.;
const ACTIVE_GROWTH: f32 = 2.;
/// The gap between verses, and a verse's padding, as shares of the verse size.
const VERSE_GAP: f32 = 0.6;
const VERSE_PADDING: f32 = 0.15;
const VERSE_SIDE_PADDING: f32 = 0.45;
/// How strongly unsung text shows: the quiet ink, and its share for lines past and ahead.
const QUIET: f32 = 0.56;
const PAST: f32 = 0.4;
const AHEAD: f32 = 0.6;
/// How far a hazy verse fades and blurs at the sheet's edge, and the least blur worth drawing.
const VEIL: f32 = 0.3;
const BLUR: f32 = 0.13;
const BLUR_LEAST: f32 = 0.05;

/// The lyrics settings the preview follows.
#[derive(Clone, Copy)]
struct Look {
    karaoke: bool,
    blur: bool,
    scripts: Option<RomanizationScripts>,
}

/// The Lyrics page's preview: the sample lines as Now playing draws them at its text size, over
/// the ambient field while that is on. Every setting springs the lines to their new look.
pub(in crate::screens::settings) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layout: &Layout) -> El {
    let width = layout.width;
    let height = (width * ASPECT).round();
    let settings = &ui.store.settings;
    // over the field the lines wear what fullscreen would, not the theme around them
    let (theme, field) = match settings.ambient() {
        true => {
            let cover = playing_or_picked(ui).large_cover(&ui.store);
            super::field(ui, cover.as_deref(), width)
        }
        false => (
            ui.theme,
            el().key("field")
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .fill(ui.theme.background.color()),
        ),
    };
    let look = Look {
        karaoke: settings.karaoke_lyrics(),
        blur: settings.blur_lyrics(),
        scripts: settings
            .romanized_lyrics()
            .then(|| settings.romanization_scripts()),
    };
    let scale = settings.fullscreen_lyrics_scale();
    let size = (theme.text(Text::Title) + FULLSCREEN_GROWTH) * scale;
    let lane = theme.text(Text::Body) * scale;
    let on = settings.lyrics_enabled();
    let sheet = el()
        .key("lyrics-sheet")
        .w_full()
        .h(height)
        .opacity(match on {
            true => 1.,
            false => DIMMED,
        })
        .motion(ui::Springs::BASE)
        .a11y_hidden()
        .child(field)
        .child(lines(look, size, lane, height, &theme));
    framed("lyrics-preview", sheet, &ui.theme)
}

/// The sample lines around the one being sung, at the verse and lane sizes, over the whole of a
/// parent `height` tall. The one being sung starts at `ANCHOR` of the height whatever the
/// sizes, so a large text size never pushes it out of view.
fn lines(look: Look, size: f32, lane: f32, height: f32, theme: &Theme) -> El {
    let gap = size * VERSE_GAP;
    let anchor = height * ANCHOR;
    let fade = size * VERSE_FADE;
    let verse_at = |index: usize| verse(index, look, size, lane, theme);
    let mut sheet = el().key("lines").absolute(0., 0.).w_full().h_full();
    if ui::effects() {
        sheet = sheet.clip_fade(0., fade, 0., fade);
    }
    sheet
        .child(
            col()
                .key("before")
                .pin(Some(0.), None, Some(0.), Some(height - anchor))
                .pb(gap)
                .px(theme.metrics.inset)
                .gap(gap)
                .children((SUNG_LINE - AROUND..SUNG_LINE).map(verse_at)),
        )
        .child(
            col()
                .key("after")
                .pin(Some(0.), Some(anchor), Some(0.), None)
                .px(theme.metrics.inset)
                .gap(gap)
                .children((SUNG_LINE..SAMPLE.len()).map(verse_at)),
        )
}

/// One sample verse the way the sheet draws it at rest: the line being sung larger, with its
/// karaoke sweep held at `STILL` of its words and its romanized lane, the others quieter for
/// being past or ahead, hazier the further they stand from it while blur is on.
fn verse(index: usize, look: Look, size: f32, lane: f32, theme: &Theme) -> El {
    let line = i18n::lookup(SAMPLE[index], None).into_owned();
    let active = index == SUNG_LINE;
    let quiet = theme.foreground.opacity(QUIET);
    let karaoke = look.karaoke && ui::effects();
    let tint = match (active, index < SUNG_LINE) {
        (true, _) if karaoke => quiet,
        (true, _) => theme.foreground,
        (false, true) => quiet.opacity(QUIET * PAST),
        (false, false) => quiet.opacity(QUIET * AHEAD),
    };
    let shown = match active {
        true => size + ACTIVE_GROWTH,
        false => size,
    };
    let spans = match active && karaoke {
        true => vec![Span::new(0, sung(&line)).color(theme.foreground.color())],
        false => Vec::new(),
    };
    let mut words = col().gap(4.).child(
        rich(line, spans)
            .font(TextStyle::new(shown, Weight::SEMIBOLD).lh(shown * LEADING))
            .color(tint.color())
            .wrap(),
    );
    let romanized = look
        .scripts
        .filter(|scripts| active && scripts.contains(WritingSystem::Japanese));
    if romanized.is_some() {
        words = words.child(
            text(i18n::lookup(ROMANIZED, None).into_owned())
                .font(TextStyle::new(lane, Weight::NORMAL))
                .color(quiet.color())
                .wrap()
                .transition(Transition::offset(0., -4.).combined(Transition::opacity())),
        );
    }
    let haze = match look.blur && !active && ui::effects() {
        true => (index.abs_diff(SUNG_LINE) as f32 / AROUND as f32)
            .min(1.)
            .powf(HAZE),
        false => 0.,
    };
    // GPUI blurred by a radius about twice the deviation Rune takes
    let soft = size * BLUR * haze / 2.;
    col()
        .key(format!("verse-{index}"))
        .w_full()
        .px(size * VERSE_SIDE_PADDING)
        .py(size * VERSE_PADDING)
        .opacity(1. - VEIL * haze)
        .blur(match soft > BLUR_LEAST {
            true => soft,
            false => 0.,
        })
        .motion(ui::Springs::BASE)
        .child(words)
}

/// Where the karaoke sweep stands in `line`, as a byte offset: `STILL` of its words sung, split
/// on spaces where it has them and letter by letter where it has none.
fn sung(line: &str) -> usize {
    let spaced = line.contains(char::is_whitespace);
    let units: Vec<(usize, usize)> = match spaced {
        true => line
            .split_whitespace()
            .map(|word| {
                let start = word.as_ptr() as usize - line.as_ptr() as usize;
                (start, start + word.len())
            })
            .collect(),
        false => line
            .char_indices()
            .map(|(at, letter)| (at, at + letter.len_utf8()))
            .collect(),
    };
    let reached = ((units.len() as f32) * STILL).round() as usize;
    match reached.checked_sub(1).and_then(|last| units.get(last)) {
        Some((_, end)) => *end,
        None => 0,
    }
}
