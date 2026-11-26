//! Carries a chain stage's samples to an analyzer on another thread. The format travels in band
//! with the index of the first sample it applies to, and frames go in whole or not at all, so a
//! full ring drops time but never shifts a channel.

use rtrb::{Consumer, Producer, RingBuffer};

/// How many format changes can wait unread. A track brings one at most.
const MARKS: usize = 16;

/// The rate and channel count of the samples that follow a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub rate: u32,
    pub channels: u16,
}

/// A format and the index of the first sample it applies to.
#[derive(Clone, Copy)]
struct Mark {
    at: u64,
    layout: Layout,
}

/// The stage's end, on the audio thread. It never blocks, allocates or frees.
pub(crate) struct Feed {
    samples: Producer<f32>,
    marks: Producer<Mark>,
    /// Samples pushed so far, which is the index the next one gets.
    pushed: u64,
    layout: Option<Layout>,
    /// Whether the analyzer has been sent `layout`.
    announced: bool,
    /// How many samples of the frame under way are still to come, zero when it was not let in.
    owed: u16,
}

/// The analyzer's end. It reads format changes and whole frames in the order the stage sent
/// them.
pub(crate) struct Listener {
    samples: Consumer<f32>,
    marks: Consumer<Mark>,
    /// Samples popped so far, which is the index the next one has.
    popped: u64,
    layout: Option<Layout>,
    frame: Vec<f32>,
}

/// What a listener reads next.
pub(crate) enum Heard<'a> {
    /// The frames that follow are in this format.
    Format(Layout),
    /// One sample per channel.
    Frame(&'a [f32]),
}

/// A feed and its listener over a ring of `capacity` samples.
pub(crate) fn channel(capacity: usize) -> (Feed, Listener) {
    let (samples, sample_rx) = RingBuffer::new(capacity);
    let (marks, mark_rx) = RingBuffer::new(MARKS);
    let feed = Feed {
        samples,
        marks,
        pushed: 0,
        layout: None,
        announced: false,
        owed: 0,
    };
    let listener = Listener {
        samples: sample_rx,
        marks: mark_rx,
        popped: 0,
        layout: None,
        frame: Vec::new(),
    };
    (feed, listener)
}

impl Feed {
    /// Declares the format of the frames from the next one on, finishing the frame under way
    /// with silence. Its room was taken when it was let in.
    pub(crate) fn format(&mut self, rate: u32, channels: u16) {
        while self.owed > 0 && self.samples.push(0.).is_ok() {
            self.pushed += 1;
            self.owed -= 1;
        }
        self.owed = 0;
        self.layout = Some(Layout { rate, channels });
        self.announced = false;
    }

    /// Has the format sent again ahead of the next frame, for a listener that starts over.
    pub(crate) fn reannounce(&mut self) {
        self.announced = false;
    }

    /// Takes one sample, `channel` being its place in its frame. A frame's first sample decides
    /// whether the whole frame fits.
    #[inline]
    pub(crate) fn push(&mut self, channel: u16, sample: f32) {
        if channel == 0 {
            self.owed = self.admit();
        }
        if self.owed > 0 && self.samples.push(sample).is_ok() {
            self.pushed += 1;
            self.owed -= 1;
        }
    }

    /// The samples a frame starting now may send: all its channels, or none when they do not
    /// fit or there is no format to send them in.
    fn admit(&mut self) -> u16 {
        let Some(layout) = self.layout else {
            return 0;
        };
        if self.samples.slots() < usize::from(layout.channels) {
            return 0;
        }
        if !self.announced {
            let mark = Mark {
                at: self.pushed,
                layout,
            };
            if self.marks.push(mark).is_err() {
                return 0;
            }
            self.announced = true;
        }
        layout.channels
    }
}

impl Listener {
    /// The next format change or whole frame, or `None` until more arrives. Samples from before
    /// the first format are dropped.
    pub(crate) fn next(&mut self) -> Option<Heard<'_>> {
        if self
            .layout
            .is_some_and(|layout| self.frame.len() == usize::from(layout.channels))
        {
            self.frame.clear();
        }
        loop {
            // The stage pushes a mark before the samples it covers, so once a sample is in, the
            // mark for it is too. A mark only ever falls between frames.
            let sample = *self.samples.peek().ok()?;
            if self.frame.is_empty()
                && let Ok(mark) = self.marks.peek()
                && mark.at <= self.popped
            {
                let layout = mark.layout;
                self.marks.pop().ok();
                self.layout = Some(layout);
                return Some(Heard::Format(layout));
            }
            self.samples.pop().ok();
            self.popped += 1;
            let Some(layout) = self.layout else {
                continue;
            };
            self.frame.push(sample);
            if self.frame.len() == usize::from(layout.channels) {
                return Some(Heard::Frame(&self.frame));
            }
        }
    }

    /// Drops everything waiting and the format with it, so nothing is read until the stage
    /// sends a format again.
    pub(crate) fn forget(&mut self) {
        while self.samples.pop().is_ok() {
            self.popped += 1;
        }
        while self.marks.peek().is_ok_and(|mark| mark.at <= self.popped) {
            self.marks.pop().ok();
        }
        self.layout = None;
        self.frame.clear();
    }

    /// Whether the stage is gone. What it sent and is still unread is not worth keeping.
    pub(crate) fn abandoned(&self) -> bool {
        self.samples.is_abandoned()
    }
}
