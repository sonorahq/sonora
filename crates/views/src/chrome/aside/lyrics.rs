//! The Lyrics panel: a synced sheet that follows the song, the sung line grown and brightened on
//! a spring, the lines around it softened the further they are, and, where the source times
//! each word, a karaoke sweep across the sung line. Unsynced lyrics show as one block, and the
//! panel says when there are none.
//!
//! The view only runs when the sung line changes. `poll` finds that line from the playback
//! position, glides the sheet to it on the lyrics spring, and moves the karaoke sweep and the
//! instrumental notes through live values, so a playing song costs no view runs between lines.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i18n::t;
use music::{LyricsLane, LyricsLine, LyricsWord, Voice};
use rune::{
    Cursor, Cx, El, Live, ScrollSpec, TextStyle, Transition, Tx, WheelEvent, col, el, icon, row,
    text,
};
use state::{LyricsState, RomanizationScripts, SideTab};
use ui::{Button, Springs, Text, Theme, Vacancy, Weight};
use unicode_bidi::{BidiClass, Direction};

use crate::{Ui, act};

/// The sheet's scroll viewport and its content, which the follow glides and reads.
const SHEET: &str = "lyric-sheet";
const CONTENT: &str = "lyric-sheet-content";
/// The foreground at this opacity is the grey of unsung lyrics, which takes on what is behind it.
const QUIET: f32 = 0.56;
/// How far lines that were sung and lines still to come fade from that grey.
const PAST: f32 = 0.4;
const AHEAD: f32 = 0.6;
/// The blank room above and below the lines while the viewport's height is unknown.
const REST: f32 = 72.;
/// Where the sung line rests, as a share of the viewport from its top.
const PIN: f32 = 0.3;
/// How soft the furthest lines get, in verse sizes, how much they fade, and the curve from the
/// sung line out to them.
const BLUR: f32 = 0.13;
const VEIL: f32 = 0.3;
const HAZE: f32 = 0.45;
/// The step a line's blur is rounded to, so lines near each other share a blur.
const HAZE_STEP: f32 = 0.25;
/// Below this a blur is not worth drawing.
const HAZE_LEAST: f32 = 0.05;
/// The height of the fade along the sheet's edges, in verse sizes.
const VERSE_FADE: f32 = 1.25;
/// How much larger than the rest the sung line stands.
const ACTIVE_VERSE_GROWTH: f32 = 2.;
/// Space between phrases, room around each and beside each, relative to the verse size.
const VERSE_GAP: f32 = 0.6;
const VERSE_PADDING: f32 = 0.15;
const VERSE_SIDE_PADDING: f32 = 0.45;
/// How much larger than a lane unsynced lyrics are set.
const PLAIN_LYRICS_GROWTH: f32 = 1.15;
/// How far a held verse sinks.
const PRESSED: f32 = 0.955;
/// How a new sheet comes in: blurred by this share of a verse.
const RESOLVE_BLUR: f32 = 0.2;
/// How long the reader keeps the sheet after scrolling it before it follows the song again.
const SETTLE: Duration = Duration::from_secs(4);
/// A pause this long before a line gets a row of notes of its own.
const INSTRUMENTAL_BREAK: Duration = Duration::from_secs(5);
/// The shortest a word's sweep takes, how much longer than the word it runs, and how near the
/// end it counts as swept.
const SWEEP_LEAST: Duration = Duration::from_millis(180);
const SWEEP_STRETCH: f32 = 1.4;
const SWEPT: f32 = 0.98;
/// The soft edge of the sweep, in verse sizes.
const REVEAL: f32 = 0.6;
/// How often the sweep and the notes move: 45 times a second.
const KARAOKE_FRAME: Duration = Duration::from_nanos(1_000_000_000 / 45);
/// The soonest a poll is asked for again, so a line about to start is never polled in a spin.
const SOONEST: Duration = Duration::from_millis(8);
/// How many wrapped rows of the sung line sweep; any past them light up whole.
const SWEEP_ROWS: usize = 8;
/// The gap between the notes of an instrumental break.
const NOTE_GAP: f32 = 8.;

/// The Lyrics panel's own view state.
pub struct State {
    /// Whether the sheet follows the song, and when the reader last scrolled it themselves.
    pinned: bool,
    nudged: Option<Instant>,
    /// Whether the next aim jumps rather than glides, as a sheet comes into view.
    placing: bool,
    /// The offset the sheet was last sent to, and the one the view found for the sung line.
    aimed: Option<f32>,
    goal: Rc<Cell<Option<f32>>>,
    /// The sung line and the line whose instrumental break plays, as of the last change.
    active: Option<usize>,
    instrumental: Option<usize>,
    /// The playback position then, which says which lines were sung.
    position: Duration,
    /// Which answer the sheet shows: the track it follows and the lyrics' revision.
    take: Option<(Option<String>, u64)>,
    /// Each line broken into the rows it shows on at the sheet's width.
    plans: RefCell<Plans>,
    sweeps: Vec<Sweep>,
    notes: [Live<f32>; 3],
}

impl Default for State {
    fn default() -> Self {
        Self {
            pinned: true,
            nudged: None,
            placing: true,
            aimed: None,
            goal: Rc::new(Cell::new(None)),
            active: None,
            instrumental: None,
            position: Duration::ZERO,
            take: None,
            plans: RefCell::new(Plans::default()),
            sweeps: (0..SWEEP_ROWS).map(|_| Sweep::default()).collect(),
            notes: [Live::new(0.), Live::new(0.), Live::new(0.)],
        }
    }
}

impl State {
    /// Puts the sheet back on the sung line, at once, as the panel comes into view.
    pub(crate) fn anchor(&mut self) {
        self.pin();
        self.placing = true;
    }

    /// Follows the song again.
    fn pin(&mut self) {
        self.pinned = true;
        self.nudged = None;
        self.aimed = None;
    }
}

/// The two live offsets that sweep one wrapped row: the clip slides to the edge of what has been
/// sung, and the bright copy of the row inside it slides back by as much, so it stays in place.
struct Sweep {
    clip: Live<[f32; 2]>,
    words: Live<[f32; 2]>,
}

impl Default for Sweep {
    fn default() -> Self {
        Self {
            clip: Live::new([-1e4, 0.]),
            words: Live::new([1e4, 0.]),
        }
    }
}

/// The wrap of every line, for the answer, width, size and fonts it was measured at.
#[derive(Default)]
struct Plans {
    key: Option<PlanKey>,
    lines: HashMap<usize, Wrapped>,
}

#[derive(Clone, PartialEq)]
struct PlanKey {
    take: (Option<String>, u64),
    width: u32,
    size: u32,
    fonts: u64,
}

/// One line, split where it may wrap and into the words its timing names, and broken into rows.
#[derive(Clone, Default)]
struct Wrapped {
    fragments: Vec<String>,
    /// The word each fragment belongs to.
    spoken: Vec<usize>,
    rows: Vec<Range<usize>>,
    widths: Vec<f32>,
    /// Where each fragment starts within its word, and how wide the whole word is.
    spans: Vec<(f32, f32)>,
    /// Whether each word fills at an even pace rather than on the sweep's curve.
    evenly: Vec<bool>,
    text: Vec<String>,
    row_widths: Vec<f32>,
    /// Whether the line reads right to left, which sets the side its sweep starts from.
    rtl: bool,
}

/// What every line of a sheet is drawn with.
#[derive(Clone, Copy)]
struct Look {
    verse: f32,
    lane: f32,
    phrase_gap: f32,
    verse_pad: f32,
    karaoke: bool,
    scripts: Option<RomanizationScripts>,
    mirrored: bool,
    /// How many lines fit above and below the sung one, for the haze; zero for none.
    above: f32,
    below: f32,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let state = &ui.chrome.aside.lyrics;
    let header = super::header(SideTab::Lyrics, None, theme);
    let lyrics = &ui.store.lyrics;
    let settings = &ui.store.settings;
    let (verse, lane) = verse_sizes(settings.panel_lyrics_scale(), theme);
    let hit = lyrics.current();
    let lines = match (lyrics.state(), hit.map(|hit| &hit.lyrics)) {
        (LyricsState::Ready, Some(music::Lyrics::Synced { lines })) => Some(lines),
        _ => None,
    };
    let viewport = cx
        .layout_with(SHEET, |rect| {
            [rect[1].round(), rect[2].round(), rect[3].round()]
        })
        .unwrap_or([0.; 3]);
    let mirrored = hit.is_some_and(|hit| reads_right_to_left(&hit.lyrics));
    let pitch = active_verse_size(verse) * ui::LEADING + verse * (VERSE_PADDING * 2. + VERSE_GAP);
    let hazing = ui::effects() && settings.blur_lyrics() && state.pinned && viewport[2] > 0.;
    let look = Look {
        verse,
        lane,
        phrase_gap: verse * VERSE_GAP,
        verse_pad: verse * VERSE_SIDE_PADDING,
        karaoke: settings.karaoke_lyrics() && ui::effects(),
        scripts: settings
            .romanized_lyrics()
            .then(|| settings.romanization_scripts()),
        mirrored,
        above: match hazing {
            true => (viewport[2] * PIN / pitch).max(1.),
            false => 0.,
        },
        below: match hazing {
            true => (viewport[2] * (1. - PIN) / pitch).max(1.),
            false => 0.,
        },
    };
    let take = (lyrics.following().map(str::to_owned), lyrics.revision());

    let body = match (lines, lyrics.state()) {
        (Some(lines), _) => sheet(ui, cx, lines, look, &take, viewport, theme),
        (None, LyricsState::Ready) => match hit.map(|hit| &hit.lyrics) {
            Some(music::Lyrics::Plain { text, romanized }) => {
                let plain = plain(text, romanized.as_ref(), look, theme);
                scroller(ui, plain, look, &take, None, theme)
            }
            _ => wordless("lyrics-missing", "icons/mic-off.svg", theme),
        },
        (None, LyricsState::Idle) => empty("lyrics-idle", theme),
        (None, LyricsState::Loading) => empty("lyrics-loading", theme),
        (None, LyricsState::Instrumental) => {
            wordless("lyrics-instrumental", "icons/guitar.svg", theme)
        }
        (None, LyricsState::Missing) => wordless("lyrics-missing", "icons/mic-off.svg", theme),
        (None, LyricsState::Failed(reason)) => {
            match ui.store.network.lost() || music::trouble::offline(reason) {
                true => wordless("trouble-offline", "icons/wifi-off.svg", theme),
                false => empty("lyrics-failed", theme),
            }
        }
    };
    let follow = (lines.is_some() && !state.pinned).then(|| {
        ui::perched(
            Button::new("resume-pin")
                .secondary()
                .icon("icons/undo-2.svg")
                .tooltip(t!("lyrics-follow").to_string())
                .on_press(|ui: &mut Ui| ui.chrome.aside.lyrics.pin())
                .build(theme),
        )
        .key("lyrics-follow")
        .transition(Transition::opacity().combined(Transition::offset(0., 8.)))
    });

    col()
        .w_full()
        .h_full()
        .min_h(0.)
        .min_w(0.)
        .child(header)
        .child(
            col()
                .key("lyrics-body")
                .grow(1.)
                .min_h(0.)
                .w_full()
                .child(body)
                .children(follow),
        )
}

/// Follows the song: finds the sung line, glides the sheet to it, sweeps its words and lights the
/// notes of a break, and asks for the next poll when something will next change. True when the
/// sung line changed, or the sheet starts following again, so the view runs.
pub(crate) fn poll(ui: &mut Ui, shown: bool, tx: &mut Tx) -> bool {
    if !shown {
        return false;
    }
    let (verse, _) = verse_sizes(ui.store.settings.panel_lyrics_scale(), &ui.theme);
    let karaoke = ui.store.settings.karaoke_lyrics() && ui::effects();
    let playback = &ui.store.playback;
    let (position, playing) = match playback.steered(&ui.store.devices) {
        Some(remote) => (remote.state.live_position(), remote.state.playing),
        None => (
            playback.live_position(),
            matches!(playback.state(), state::PlaybackState::Playing),
        ),
    };
    let lyrics = &ui.store.lyrics;
    let state = &mut ui.chrome.aside.lyrics;
    let mut changed = false;

    let take = (lyrics.following().map(str::to_owned), lyrics.revision());
    if state.take.as_ref() != Some(&take) {
        state.take = Some(take);
        state.anchor();
        state.goal.set(None);
        state.active = None;
        state.instrumental = None;
        changed = true;
    }
    if !state.pinned {
        match state.nudged.map(|at| at.elapsed()) {
            Some(still) if still < SETTLE => tx.wake_after(SETTLE - still),
            _ => {
                state.pin();
                changed = true;
            }
        }
    }
    let lines = match (lyrics.state(), lyrics.current().map(|hit| &hit.lyrics)) {
        (LyricsState::Ready, Some(music::Lyrics::Synced { lines })) => lines,
        _ => return changed,
    };

    let active = sung_line(lines, position);
    let instrumental = active_instrumental(lines, position);
    if (active, instrumental) != (state.active, state.instrumental) {
        state.active = active;
        state.instrumental = instrumental;
        state.position = position;
        changed = true;
    }

    if state.pinned
        && let Some(goal) = state.goal.get()
        && state.aimed.is_none_or(|aimed| (aimed - goal).abs() > 0.5)
    {
        match std::mem::take(&mut state.placing) {
            true => tx.scroll_to_instant(SHEET, 0., goal),
            false => tx.scroll_to_with(SHEET, 0., goal, Springs::LYRICS_SCROLL),
        }
        state.aimed = Some(goal);
    }

    let sweeping = active.is_some_and(|index| {
        let line = &lines[index];
        let words = line.words.as_deref().filter(|words| !words.is_empty());
        match (karaoke, words) {
            (true, Some(words)) => {
                let plans = state.plans.borrow();
                if let Some(plan) = plans.lines.get(&index) {
                    sweep(
                        &state.sweeps,
                        plan,
                        line.start,
                        words,
                        position,
                        verse * REVEAL,
                    );
                }
                line.sung_end().is_none_or(|end| position < end)
            }
            _ => false,
        }
    });
    let breaking = instrumental.is_some_and(|index| {
        let gap = instrumental_gap_before(lines, index);
        let start = lines[index].start.saturating_sub(gap);
        let progress = progress_between(start, lines[index].start, position);
        for (note, level) in state.notes.iter().enumerate() {
            level.set((progress * 3. - note as f32).clamp(0., 1.));
        }
        true
    });

    if playing {
        let next = lines
            .iter()
            .enumerate()
            .flat_map(|(index, line)| {
                let gap = instrumental_gap_before(lines, index);
                let notes = (gap >= INSTRUMENTAL_BREAK).then(|| line.start.saturating_sub(gap));
                [Some(line.start), notes]
            })
            .flatten()
            .filter(|at| *at > position)
            .min();
        match (sweeping || breaking, next) {
            (true, _) => tx.wake_after(KARAOKE_FRAME),
            (false, Some(next)) => tx.wake_after((next - position).max(SOONEST)),
            (false, None) => {}
        }
    }
    changed
}

/// The verse and lane sizes at `scale`.
fn verse_sizes(scale: f32, theme: &Theme) -> (f32, f32) {
    (
        theme.text(Text::Large) * scale,
        theme.text(Text::Body) * scale,
    )
}

fn active_verse_size(verse: f32) -> f32 {
    verse + ACTIVE_VERSE_GROWTH
}

/// The grey unsung lyrics are drawn in.
fn quiet(theme: &Theme) -> ui::Hsla {
    theme.foreground.opacity(QUIET)
}

/// The synced sheet: every line, a row of notes before each long pause, and the credit.
fn sheet(
    ui: &Ui,
    cx: &mut Cx<Ui>,
    lines: &[LyricsLine],
    look: Look,
    take: &(Option<String>, u64),
    viewport: [f32; 3],
    theme: &Theme,
) -> El {
    let state = &ui.chrome.aside.lyrics;
    let sheet_padding = theme.metrics.pad * 0.75;
    let active_size = active_verse_size(look.verse);
    let grown = active_size / look.verse;
    let wrap_width = (viewport[1] - sheet_padding * 2. - look.verse_pad * 2.) / grown;
    let style = TextStyle::new(look.verse, Weight::SEMIBOLD);
    if wrap_width > 0. {
        let key = PlanKey {
            take: take.clone(),
            width: wrap_width.round() as u32,
            size: look.verse.to_bits(),
            fonts: rune::text::font_epoch(),
        };
        let mut plans = state.plans.borrow_mut();
        if plans.key.as_ref() != Some(&key) {
            plans.key = Some(key);
            plans.lines.clear();
        }
        for (index, line) in lines.iter().enumerate() {
            if !plans.lines.contains_key(&index)
                && let Some(plan) = wrap(
                    &lyrics_parts(&line.text, line.words.as_deref()),
                    style,
                    wrap_width,
                )
            {
                plans.lines.insert(index, plan);
            }
        }
    }

    // The sung line's place in the sheet, as laid out, is where the follow glides to.
    let focus = state.active.or(state.instrumental);
    let goal = focus.and_then(|index| {
        let key = match state.active {
            Some(_) => format!("lyric-verse-{index}"),
            None => format!("lyric-break-{index}"),
        };
        let top = cx.layout_with(&key, |rect| rect[1].round())?;
        Some((top - viewport[0] - viewport[2] * PIN).max(0.))
    });
    state.goal.set(goal.filter(|_| viewport[2] > 0.));

    let plans = state.plans.borrow();
    let count = lines.len();
    let mut rows = Vec::with_capacity(count + 1);
    for (index, line) in lines.iter().enumerate() {
        let gap = instrumental_gap_before(lines, index);
        if gap >= INSTRUMENTAL_BREAK {
            rows.push(notes(
                ui,
                line.start.saturating_sub(gap),
                index,
                look,
                theme,
            ));
        }
        rows.push(verse_row(
            ui,
            lines,
            index,
            plans.lines.get(&index),
            look,
            theme,
        ));
    }
    drop(plans);
    let credit = ui
        .store
        .lyrics
        .current()
        .map(|hit| credit(hit.source, &hit.writers, look, theme));
    let tail = active_size * ui::LEADING + look.verse * VERSE_PADDING * 2.;
    let edges = match viewport[2] > 0. {
        true => Some((
            (viewport[2] * PIN).max(REST),
            (viewport[2] * (1. - PIN) - tail).max(REST),
        )),
        false => None,
    };
    let content = col()
        .gap(look.phrase_gap)
        .items_start()
        .children(rows)
        .children(credit);
    scroller(ui, content, look, take, edges, theme)
}

/// The scroll viewport a sheet reads in, which a wheel takes out of following the song. A new
/// answer comes in out of a blur.
fn scroller(
    _ui: &Ui,
    content: El,
    look: Look,
    take: &(Option<String>, u64),
    edges: Option<(f32, f32)>,
    theme: &Theme,
) -> El {
    let (over, under) = edges.unwrap_or((REST, REST));
    let content = content
        .key(format!(
            "{CONTENT}-{}-{}",
            take.0.as_deref().unwrap_or(""),
            take.1
        ))
        .w_full()
        .px(theme.metrics.pad * 0.75)
        .pt(over)
        .pb(under)
        .transition(Transition::asymmetric(
            Transition::opacity().combined(Transition::blur(look.verse * RESOLVE_BLUR)),
            Transition::none(),
        ))
        .on_wheel(|ui: &mut Ui, _: WheelEvent, tx: &mut Tx| {
            let state = &mut ui.chrome.aside.lyrics;
            state.nudged = Some(Instant::now());
            match std::mem::replace(&mut state.pinned, false) {
                true => state.aimed = None,
                false => tx.live_only(),
            }
            tx.wake_after(SETTLE);
            false
        });
    let mut viewport = col()
        .key(SHEET)
        .grow(1.)
        .min_h(0.)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content);
    if ui::effects() {
        viewport = viewport.scroll_edge_fade(look.verse * VERSE_FADE);
    }
    viewport
}

/// One line: its words in rows, the sung one grown and brightened, its romanization and, while
/// it is sung, its backing vocals under it. Pressing it seeks to it.
fn verse_row(
    ui: &Ui,
    lines: &[LyricsLine],
    index: usize,
    plan: Option<&Wrapped>,
    look: Look,
    theme: &Theme,
) -> El {
    let state = &ui.chrome.aside.lyrics;
    let line = &lines[index];
    let active = state.active == Some(index);
    let ended =
        state.active.is_some_and(|sung| index < sung) || line_has_passed(line, state.position);
    let worded = look.karaoke && line.words.as_ref().is_some_and(|words| !words.is_empty());
    let tint = match (active, ended) {
        (true, _) if worded => quiet(theme),
        (true, _) => theme.foreground,
        (false, true) => quiet(theme).opacity(PAST),
        (false, false) => quiet(theme).opacity(AHEAD),
    };
    let hugs = hugs_end(line.voice, look.mirrored);
    let style = TextStyle::new(look.verse, Weight::SEMIBOLD);
    let line_height = active_verse_size(look.verse) * ui::LEADING;

    let mut content = col()
        .key(format!("lyric-words-{index}"))
        .gap(theme.metrics.pad / 2.)
        .min_w(0.)
        .px(look.verse_pad)
        .py(look.verse * VERSE_PADDING)
        .radius(theme.radius)
        .hover(theme.table_hover.color())
        .press_scale(PRESSED)
        .scale(match active {
            true => active_verse_size(look.verse) / look.verse,
            false => 1.,
        })
        .origin(
            match hugs {
                true => 1.,
                false => 0.,
            },
            0.5,
        )
        .scale_motion(Springs::BASE);
    if hugs {
        content = content.items_end();
    }
    match plan {
        Some(plan) => {
            if plan.text.len() > 1 {
                content = content.w_full();
            }
            for (at, words) in plan.text.iter().enumerate() {
                let sweep = (active && worded)
                    .then(|| state.sweeps.get(at))
                    .flatten()
                    .map(|sweep| (sweep, plan.row_widths[at], plan.rtl));
                content = content.child(verse_text(
                    words,
                    style,
                    tint,
                    sweep,
                    line_height,
                    look.verse * REVEAL,
                    theme,
                ));
            }
        }
        None => {
            content = content.child(
                text(line.text.clone())
                    .font(style)
                    .color(tint.color())
                    .wrap()
                    .a11y_hidden(),
            )
        }
    }
    if let Some(romanized) = selected_romanization(line.romanized.as_ref(), look.scripts) {
        content = content.child(romanized_lane(romanized, look.lane, theme));
    }
    if active && !line.secondary.is_empty() {
        content = content.child(
            col()
                .key(format!("lyric-lanes-{index}"))
                .gap(theme.metrics.pad / 2.)
                .min_w(0.)
                .transition(Transition::opacity())
                .children(
                    line.secondary
                        .iter()
                        .enumerate()
                        .map(|(at, lane)| secondary_lane(lane, at, look, hugs, theme)),
                ),
        );
    }

    let lanes_spoken = selected_romanization(line.romanized.as_ref(), look.scripts)
        .into_iter()
        .chain(line.secondary.iter().map(|lane| lane.text.clone()))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let traded = index
        .checked_sub(1)
        .is_some_and(|previous| lines[previous].voice != line.voice);
    let haze = match active {
        true => 0.,
        false => haze(index as f32, state, look),
    };
    let seek = line.start;
    let mut verse = row()
        .key(format!("lyric-verse-{index}"))
        .role("button")
        .label(line.text.clone())
        .position_in_set(index + 1, lines.len())
        .w_full()
        .min_w(0.)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .motion(Springs::BASE)
        .opacity(1. - VEIL * haze)
        .blur(stepped(look.verse * BLUR * haze))
        .on_press(act(move |ui: &mut Ui| seek_to(ui, seek)))
        .child(content);
    if !lanes_spoken.is_empty() {
        verse = verse.description(lanes_spoken);
    }
    if hugs {
        verse = verse.justify_end();
    }
    if traded {
        verse = verse.mt(look.phrase_gap * 0.5);
    }
    verse
}

/// One wrapped row of a line. The sung row of a worded line sweeps: a bright copy of it inside a
/// clip that two live offsets slide in step, so the edge moves with no view run.
fn verse_text(
    words: &str,
    style: TextStyle,
    tint: ui::Hsla,
    sweep: Option<(&Sweep, f32, bool)>,
    line_height: f32,
    fade: f32,
    theme: &Theme,
) -> El {
    let base = text(words.to_owned())
        .font(style)
        .color(tint.color())
        .a11y_hidden();
    let Some((sweep, width, rtl)) = sweep else {
        return base;
    };
    let (left, right) = match rtl {
        true => (fade, 0.),
        false => (0., fade),
    };
    el().child(base).child(
        el().absolute(0., 0.)
            .size(width + fade, line_height * 1.5)
            .clip_fade(left, 0., right, 0.)
            .live_offset(&sweep.clip)
            .a11y_hidden()
            .child(
                text(words.to_owned())
                    .font(style)
                    .color(theme.foreground.color())
                    .live_offset(&sweep.words),
            ),
    )
}

/// The romanization under a line, in the lane size.
fn romanized_lane(romanized: String, size: f32, theme: &Theme) -> El {
    text(romanized)
        .font(TextStyle::new(size, Weight::SEMIBOLD))
        .color(quiet(theme).color())
        .wrap()
        .a11y_hidden()
}

/// A backing vocal under the sung line, with its own romanization.
fn secondary_lane(lane: &LyricsLane, at: usize, look: Look, hugs: bool, theme: &Theme) -> El {
    let tint = match look.karaoke && lane.worded() {
        true => quiet(theme),
        false => theme.foreground,
    };
    let mut group = col().key(format!("lyric-lane-{at}")).min_w(0.).child(
        text(lane.text.clone())
            .font(TextStyle::new(look.lane, Weight::SEMIBOLD))
            .color(tint.color())
            .wrap()
            .a11y_hidden(),
    );
    if let Some(romanized) = selected_romanization(lane.romanized.as_ref(), look.scripts) {
        group = group.child(romanized_lane(romanized, look.lane, theme));
    }
    if hugs {
        group = group.items_end();
    }
    group
}

/// The three notes of a long pause before line `index`, lighting one after another as it runs.
/// Pressing them seeks to where the pause starts.
fn notes(ui: &Ui, start: Duration, index: usize, look: Look, theme: &Theme) -> El {
    let state = &ui.chrome.aside.lyrics;
    let playing = state.instrumental == Some(index);
    let past = state.active.is_some_and(|sung| sung >= index)
        || (!playing && state.position >= start && state.active.is_none_or(|sung| sung >= index));
    let size = look.verse;
    let dim = match past {
        true => quiet(theme).opacity(PAST),
        false => quiet(theme).opacity(AHEAD),
    };
    let haze = match state.active.is_some() && !playing {
        true => haze(index as f32 - 0.5, state, look),
        false => 0.,
    };
    let glyphs = state.notes.iter().enumerate().map(|(at, level)| {
        let mut note = el()
            .key(format!("note-{at}"))
            .size(size, size)
            .child(icon(icons::get("icons/music-2.svg"), size).color(dim.color()));
        if playing {
            note = note.child(
                icon(icons::get("icons/music-2.svg"), size)
                    .color(theme.primary.color())
                    .absolute(0., 0.)
                    .live_opacity(level.smooth(Springs::QUICK)),
            );
        }
        note.a11y_hidden()
    });
    row()
        .key(format!("lyric-break-{index}"))
        .role("button")
        .label(t!("lyrics-instrumental-break").to_string())
        .center_y()
        .gap(NOTE_GAP)
        .px(look.verse_pad)
        .py(look.verse * 0.45)
        .radius(theme.radius)
        .hover(theme.table_hover.color())
        .press_scale(PRESSED)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .motion(Springs::BASE)
        .opacity(1. - VEIL * haze)
        .blur(stepped(look.verse * BLUR * haze))
        .on_press(act(move |ui: &mut Ui| seek_to(ui, start)))
        .children(glyphs)
}

/// Unsynced lyrics as one block, line by line with each line's romanization under it.
fn plain(text_: &str, romanized: Option<&music::RomanizedText>, look: Look, theme: &Theme) -> El {
    let size = (look.lane * PLAIN_LYRICS_GROWTH).round();
    let style = TextStyle::new(size, Weight::SEMIBOLD);
    let romanized = selected_romanization(romanized, look.scripts);
    let mut block = col()
        .key("plain-lyrics")
        .role("text")
        .label(text_.to_owned())
        .w_full()
        .px(look.verse_pad)
        .gap(theme.metrics.pad / 2.);
    if look.mirrored {
        block = block.items_end();
    }
    let align = match look.mirrored {
        true => rune::Align::End,
        false => rune::Align::Start,
    };
    match romanized {
        None => block.child(
            text(text_.to_owned())
                .font(style)
                .color(theme.foreground.color())
                .align(align)
                .wrap()
                .a11y_hidden(),
        ),
        Some(romanized) => {
            let romanized = romanized.lines().collect::<Vec<_>>();
            block.children(text_.lines().enumerate().map(|(index, line)| {
                let mut group = col().child(
                    text(line.to_owned())
                        .font(style)
                        .color(theme.foreground.color())
                        .align(align)
                        .wrap()
                        .a11y_hidden(),
                );
                if let Some(under) = romanized.get(index).filter(|line| !line.is_empty()) {
                    group = group.child(
                        text((*under).to_owned())
                            .font(TextStyle::new(look.lane, Weight::NORMAL))
                            .color(quiet(theme).color())
                            .align(align)
                            .wrap()
                            .a11y_hidden(),
                    );
                }
                group
            }))
        }
    }
}

/// Which source the sheet came from and, when it names them, who wrote the song.
fn credit(source: &str, writers: &[String], look: Look, theme: &Theme) -> El {
    let from = match source == music::lyrics::LOCAL {
        true => t!("lyrics-source-local").to_string(),
        false => t!("lyrics-source", source = source).to_string(),
    };
    let style = TextStyle::new(theme.text(Text::Small), Weight::NORMAL);
    let line = |key: &str, words: String| {
        text(words.clone())
            .key(key.to_owned())
            .role("text")
            .label(words)
            .font(style)
            .color(quiet(theme).color())
            .wrap()
    };
    let mut credit = col()
        .key("lyrics-credit")
        .w_full()
        .px(look.verse_pad)
        .pt(theme.metrics.pad)
        .child(line("lyrics-credit-source", from));
    if !writers.is_empty() {
        let writers = writers.join(", ");
        credit = credit.child(line(
            "lyrics-credit-writers",
            t!("lyrics-writers", writers = writers.as_str()).to_string(),
        ));
    }
    credit
}

/// The muted line of a sheet that is not there yet.
fn empty(key: &'static str, theme: &Theme) -> El {
    col().grow(1.).w_full().center().child(ui::spoken_vacant(
        key,
        i18n::lookup(key, None).to_string(),
        theme,
    ))
}

/// The picture and line of a song without words to show.
fn wordless(key: &'static str, glyph: &'static str, theme: &Theme) -> El {
    col().grow(1.).w_full().center().child(
        Vacancy::new(i18n::lookup(key, None).to_string())
            .spoken(key)
            .icon(glyph)
            .build(theme),
    )
}

/// Seeks to `position` and follows the song from there.
fn seek_to(ui: &mut Ui, position: Duration) {
    ui.chrome.aside.lyrics.pin();
    ui.store.seek_to(position);
}

/// How far out of focus the line at `place` sits: zero at the sung line, rising on a curve to
/// one a viewport's share away. Zero while the sheet is not hazed.
fn haze(place: f32, state: &State, look: Look) -> f32 {
    if look.above <= 0. {
        return 0.;
    }
    let focus = state.active.or(state.instrumental).unwrap_or(0) as f32;
    let travel = place - focus;
    let reach = match travel >= 0. {
        true => look.below,
        false => look.above,
    };
    (travel.abs() / reach).min(1.).powf(HAZE)
}

/// A blur rounded to the nearest `HAZE_STEP`, and none below `HAZE_LEAST`.
fn stepped(blur: f32) -> f32 {
    let blur = HAZE_STEP * (blur / HAZE_STEP).round();
    match blur > HAZE_LEAST {
        true => blur,
        false => 0.,
    }
}

/// Moves the sweep of each wrapped row of the sung line to where `position` has sung it to.
fn sweep(
    sweeps: &[Sweep],
    plan: &Wrapped,
    line_start: Duration,
    words: &[LyricsWord],
    position: Duration,
    fade: f32,
) {
    let windows = (0..words.len())
        .map(|word| {
            let (start, end) = karaoke_window(line_start, words, word);
            (start, end, word + 1 >= words.len())
        })
        .collect::<Vec<_>>();
    for (row, sweep) in sweeps.iter().enumerate().take(plan.rows.len()) {
        let front = revealed(plan, row, &windows, position);
        let width = plan.row_widths[row];
        let boxed = width + fade;
        // the soft edge runs past the row's end as it finishes, so the last letter lights whole
        let reach = match width > 0. {
            true => front + fade * (front / width).min(1.),
            false => 0.,
        };
        let (clip, words) = match plan.rtl {
            true => (boxed - reach, reach - boxed),
            false => (reach - boxed, boxed - reach),
        };
        sweep.clip.set([clip, 0.]);
        sweep.words.set([words, 0.]);
    }
}

/// How far into row `row` of `plan` the sweep has reached at `position`, in points.
fn revealed(
    plan: &Wrapped,
    row: usize,
    windows: &[(Duration, Duration, bool)],
    position: Duration,
) -> f32 {
    let mut front = 0f32;
    let mut offset = 0f32;
    for index in plan.rows[row].clone() {
        let mine = plan.widths.get(index).copied().unwrap_or(0.);
        let word = plan.spoken.get(index).copied().unwrap_or(index);
        let Some(&(start, end, last)) = windows.get(word) else {
            offset += mine;
            continue;
        };
        // a wide character or a phrase timed as one word fills at an even pace; the eased curve
        // only reads as a flourish across Latin letters
        let share = match plan.evenly.get(word).copied().unwrap_or(false) {
            true => progress_between(start, end, position),
            false => swept(start, end, position, last),
        };
        if share > 0. {
            // a word covering several fragments hands each its own slice of the sweep
            let (before, whole) = plan.spans.get(index).copied().unwrap_or((0., mine));
            let part = match mine > 0. {
                true => ((whole * share - before) / mine).clamp(0., 1.),
                false => 0.,
            };
            let reach = offset + mine * part;
            if part > 0. && reach > front {
                front = reach;
            }
        }
        offset += mine;
    }
    front
}

fn karaoke_window(
    line_start: Duration,
    words: &[LyricsWord],
    index: usize,
) -> (Duration, Duration) {
    let word = &words[index];
    let start = match index {
        0 => line_start.min(word.start),
        _ => word.start,
    };
    let sung = word.end.max(start);
    let end = match words.get(index + 1) {
        // a rest after a word ends its sweep there rather than dragging it along
        Some(next) => match sung > start {
            true => next.start.max(start).min(sung),
            false => next.start.max(start),
        },
        None => sung,
    };
    (start, end)
}

fn swept(start: Duration, end: Duration, position: Duration, tail: bool) -> f32 {
    let span = end.saturating_sub(start);
    let travel = match tail {
        true => span.max(SWEEP_LEAST),
        false => span.mul_f32(SWEEP_STRETCH).max(SWEEP_LEAST),
    };
    let eased = ui::motion::ease_out_cubic(progress_between(start, start + travel, position));
    match eased >= SWEPT {
        true => 1.,
        false => eased,
    }
}

fn progress_between(start: Duration, end: Duration, position: Duration) -> f32 {
    if position < start {
        return 0.;
    }
    if position >= end {
        return 1.;
    }
    ((position - start).as_secs_f32() / (end - start).as_secs_f32()).clamp(0., 1.)
}

fn instrumental_gap_before(lines: &[LyricsLine], index: usize) -> Duration {
    let start = lines[index].start;
    match index {
        0 => start,
        _ => {
            let previous = &lines[index - 1];
            start.saturating_sub(previous.sung_end().unwrap_or(previous.start))
        }
    }
}

/// The line a long pause plays before, while it plays.
fn active_instrumental(lines: &[LyricsLine], position: Duration) -> Option<usize> {
    let next = lines.iter().position(|line| line.start > position)?;
    let gap = instrumental_gap_before(lines, next);
    let start = lines[next].start.saturating_sub(gap);
    (gap >= INSTRUMENTAL_BREAK && position >= start).then_some(next)
}

/// The line sung at `position`, as the store tells it, or none while a long pause plays.
fn sung_line(lines: &[LyricsLine], position: Duration) -> Option<usize> {
    match active_instrumental(lines, position) {
        Some(_) => None,
        None => music::lyrics::active(lines, position),
    }
}

fn line_has_passed(line: &LyricsLine, position: Duration) -> bool {
    line.sung_end().is_some_and(|end| position >= end)
}

fn selected_romanization(
    romanized: Option<&music::RomanizedText>,
    scripts: Option<RomanizationScripts>,
) -> Option<String> {
    let romanized = romanized?;
    scripts?
        .contains(romanized.writing_system)
        .then(|| romanized.text.clone())
}

/// Breaks a line into fragments where it may wrap, measures them on one shaped line so kerning
/// across a boundary counts, and fills rows `width` wide.
fn wrap(parts: &[(String, usize)], style: TextStyle, width: f32) -> Option<Wrapped> {
    if width <= 0. || parts.is_empty() {
        return None;
    }
    let fragments = parts
        .iter()
        .map(|(text, _)| text.clone())
        .collect::<Vec<_>>();
    let spoken = parts.iter().map(|(_, word)| *word).collect::<Vec<_>>();
    let whole = fragments.concat();
    let rtl = right_to_left(&whole);
    let widths = match has_right_to_left(&whole) {
        // right-to-left text sits left of what follows it, so a prefix no longer measures a
        // fragment: each is shaped on its own instead
        true => fragments
            .iter()
            .map(|fragment| rune::text::advance(fragment, style))
            .collect::<Vec<_>>(),
        false => {
            let mut widths = Vec::with_capacity(fragments.len());
            let mut prefix = String::new();
            let mut left = 0.;
            for fragment in &fragments {
                prefix.push_str(fragment);
                let right = rune::text::advance(&prefix, style);
                widths.push(right - left);
                left = right;
            }
            widths
        }
    };
    let breaks = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| match index.checked_sub(1) {
            Some(previous) => separable(&fragments[previous], fragment),
            None => true,
        })
        .collect::<Vec<_>>();
    let rows = wrap_fragment_widths(&widths, &breaks, width);
    let text = rows
        .iter()
        .map(|row| fragments[row.clone()].concat())
        .collect::<Vec<_>>();
    let row_widths = rows
        .iter()
        .map(|row| widths[row.clone()].iter().sum::<f32>().ceil())
        .collect();
    Some(Wrapped {
        spans: word_spans(&spoken, &widths),
        evenly: evenly_filled(&fragments, &spoken),
        fragments,
        spoken,
        rows,
        widths,
        text,
        row_widths,
        rtl,
    })
}

fn wrap_fragment_widths(widths: &[f32], breaks: &[bool], width: f32) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0.;
    for (index, fragment) in widths.iter().copied().enumerate() {
        if index > start
            && used + fragment > width
            && let Some(split) = (start + 1..=index)
                .rev()
                .find(|at| breaks.get(*at).copied().unwrap_or(true))
        {
            rows.push(start..split);
            used = widths[split..index].iter().sum();
            start = split;
        }
        used += fragment;
    }
    if start < widths.len() {
        rows.push(start..widths.len());
    }
    rows
}

/// A fragment's own slice of the word it belongs to.
fn word_spans(spoken: &[usize], widths: &[f32]) -> Vec<(f32, f32)> {
    let words = spoken.iter().copied().max().map_or(0, |last| last + 1);
    let mut wholes = vec![0f32; words];
    for (index, word) in spoken.iter().enumerate() {
        wholes[*word] += widths.get(index).copied().unwrap_or(0.);
    }
    let mut befores = vec![0f32; words];
    spoken
        .iter()
        .enumerate()
        .map(|(index, word)| {
            let before = befores[*word];
            befores[*word] += widths.get(index).copied().unwrap_or(0.);
            (before, wholes[*word])
        })
        .collect()
}

fn evenly_filled(fragments: &[String], spoken: &[usize]) -> Vec<bool> {
    let words = spoken.iter().copied().max().map_or(0, |last| last + 1);
    let mut pieces = vec![0usize; words];
    let mut broad = vec![false; words];
    for (index, word) in spoken.iter().enumerate() {
        pieces[*word] += 1;
        broad[*word] |= fragments[index].chars().any(wide);
    }
    pieces
        .into_iter()
        .zip(broad)
        .map(|(pieces, broad)| pieces > 1 || broad)
        .collect()
}

fn lyrics_parts(line: &str, words: Option<&[LyricsWord]>) -> Vec<(String, usize)> {
    match words {
        Some(words) if !words.is_empty() => karaoke_parts(line, words),
        _ => plain_lyrics_fragments(line)
            .into_iter()
            .enumerate()
            .map(|(index, piece)| (piece, index))
            .collect(),
    }
}

/// The line's text cut at each timed word, keeping the spacing the line has between them, and
/// each piece tagged with its word. Words the line does not spell out are spaced as they come.
fn karaoke_parts(line: &str, words: &[LyricsWord]) -> Vec<(String, usize)> {
    let mut starts = Vec::with_capacity(words.len());
    let mut cursor = 0;
    for word in words {
        if word.text.is_empty() {
            return spaced_words(words);
        }
        let Some(remainder) = line.get(cursor..) else {
            return spaced_words(words);
        };
        let Some(relative) = remainder.find(&word.text) else {
            return spaced_words(words);
        };
        let start = cursor + relative;
        starts.push(start);
        cursor = start + word.text.len();
    }
    starts
        .iter()
        .enumerate()
        .flat_map(|(index, start)| {
            let start = match index {
                0 => 0,
                _ => *start,
            };
            let end = starts.get(index + 1).copied().unwrap_or(line.len());
            plain_lyrics_fragments(&line[start..end])
                .into_iter()
                .map(move |piece| (piece, index))
        })
        .collect()
}

fn spaced_words(words: &[LyricsWord]) -> Vec<(String, usize)> {
    words
        .iter()
        .enumerate()
        .flat_map(|(index, word)| {
            let mut text = word.text.clone();
            if words
                .get(index + 1)
                .is_some_and(|next| needs_space(&word.text, &next.text))
            {
                text.push(' ');
            }
            plain_lyrics_fragments(&text)
                .into_iter()
                .map(move |piece| (piece, index))
        })
        .collect()
}

fn needs_space(left: &str, right: &str) -> bool {
    let Some(last) = left.chars().next_back() else {
        return false;
    };
    let Some(first) = right.chars().next() else {
        return false;
    };
    if last.is_whitespace() || first.is_whitespace() || wide(last) || wide(first) {
        return false;
    }
    !matches!(last, '(' | '[' | '{' | '\'' | '’' | '-' | '—')
        && !matches!(
            first,
            ')' | ']' | '}' | ',' | '.' | '!' | '?' | ';' | ':' | '%' | '\'' | '’' | '-' | '—'
        )
}

/// Splits a line after each run of spacing, and between wide characters that may part.
fn plain_lyrics_fragments(line: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut start = 0;
    let mut spacing = false;
    let mut previous = None;
    for (index, letter) in line.char_indices() {
        if letter.is_whitespace() {
            spacing = true;
        } else if spacing || previous.is_some_and(|previous| parts(previous, letter)) {
            fragments.push(line[start..index].to_owned());
            start = index;
            spacing = false;
        }
        previous = Some(letter);
    }
    if start < line.len() {
        fragments.push(line[start..].to_owned());
    }
    fragments
}

fn separable(left: &str, right: &str) -> bool {
    // spacing ends a row
    if right.starts_with(char::is_whitespace) {
        return false;
    }
    if left.ends_with(char::is_whitespace) {
        return true;
    }
    match (left.chars().next_back(), right.chars().next()) {
        (Some(left), Some(right)) => parts(left, right),
        _ => false,
    }
}

fn wide(letter: char) -> bool {
    matches!(letter,
        '\u{2E80}'..='\u{303E}'
        | '\u{3041}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A000}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FF60}'
    )
}

/// Whether a row may break between two wide characters: not before closing punctuation and the
/// marks that lean on what comes before, nor after an opening bracket.
fn parts(left: char, right: char) -> bool {
    if !wide(left) || !wide(right) {
        return false;
    }
    !matches!(
        right,
        '、' | '。'
            | '，'
            | '．'
            | '！'
            | '？'
            | '：'
            | '；'
            | '」'
            | '』'
            | '）'
            | '】'
            | '〉'
            | '》'
            | '〕'
            | '・'
            | 'ー'
            | '…'
            | '々'
            | 'ゝ'
            | 'ゞ'
            | 'っ'
            | 'ッ'
    ) && !matches!(left, '「' | '『' | '（' | '【' | '〈' | '《' | '〔')
}

/// Whether a line by `voice` sits against the right edge. The lead voice starts on the side its
/// script starts from, and a counter voice answers from the other.
fn hugs_end(voice: Voice, mirrored: bool) -> bool {
    voice.lead() == mirrored
}

/// Whether most lines of `lyrics` read right to left, so the lyrics sit against the right edge.
fn reads_right_to_left(lyrics: &music::Lyrics) -> bool {
    let lean = |text: &str| match unicode_bidi::get_base_direction(text) {
        Direction::Rtl => 1,
        Direction::Ltr => -1,
        Direction::Mixed => 0,
    };
    let leaning: i32 = match lyrics {
        music::Lyrics::Plain { text, .. } => text.lines().map(lean).sum(),
        music::Lyrics::Synced { lines } => lines.iter().map(|line| lean(&line.text)).sum(),
    };
    leaning > 0
}

/// Whether `text` starts right to left, going by its first letter that has a direction.
fn right_to_left(text: &str) -> bool {
    unicode_bidi::get_base_direction(text) == Direction::Rtl
}

/// Whether any letter of `text` reads right to left.
fn has_right_to_left(text: &str) -> bool {
    text.chars().any(|letter| {
        matches!(
            unicode_bidi::bidi_class(letter),
            BidiClass::R | BidiClass::AL
        )
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{LyricsLine, LyricsWord};

    use super::{
        active_instrumental, karaoke_parts, karaoke_window, plain_lyrics_fragments, sung_line,
        wrap_fragment_widths,
    };

    fn line(start: u64, end: u64, text: &str) -> LyricsLine {
        LyricsLine {
            start: Duration::from_secs(start),
            end: Some(Duration::from_secs(end)),
            text: text.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_long_pause_plays_before_its_line() {
        let lines = [line(2, 5, "first"), line(12, 15, "second")];
        assert_eq!(active_instrumental(&lines, Duration::from_secs(8)), Some(1));
        assert_eq!(sung_line(&lines, Duration::from_secs(8)), None);
        assert_eq!(sung_line(&lines, Duration::from_secs(13)), Some(1));
    }

    #[test]
    fn karaoke_keeps_the_spacing_of_the_whole_line() {
        let words = ["Hello", "world"].map(|text| LyricsWord {
            text: text.to_owned(),
            ..Default::default()
        });
        let parts = karaoke_parts("Hello, world", &words);
        assert_eq!(parts, [("Hello, ".to_owned(), 0), ("world".to_owned(), 1)]);
    }

    #[test]
    fn plain_lyrics_break_after_spacing() {
        assert_eq!(plain_lyrics_fragments("a bc  d"), ["a ", "bc  ", "d"]);
    }

    #[test]
    fn an_oversized_fragment_keeps_a_row_of_its_own() {
        let rows = wrap_fragment_widths(&[10., 50., 10.], &[true, true, true], 30.);
        assert_eq!(rows, [0..1, 1..2, 2..3]);
    }

    #[test]
    fn a_rest_after_a_word_ends_its_sweep() {
        let words = [(0, 1), (3, 4)].map(|(start, end)| LyricsWord {
            start: Duration::from_secs(start),
            end: Duration::from_secs(end),
            text: "la".to_owned(),
        });
        assert_eq!(
            karaoke_window(Duration::ZERO, &words, 0),
            (Duration::ZERO, Duration::from_secs(1))
        );
    }
}
