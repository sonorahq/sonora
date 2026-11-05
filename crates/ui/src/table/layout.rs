use std::collections::HashMap;

use gpui::{Pixels, SharedString, TextAlign, px};
use serde::{Deserialize, Serialize};

use crate::metrics::Metrics;

const SLACK: Pixels = px(2.);
const MIN_FLEX: Pixels = px(96.);
const MIN_RIGID: Pixels = px(48.);
const MIN_TABLE: Pixels = px(160.);
const COMFORT: Pixels = px(164.);

pub(super) const PADDING: Pixels = px(8.);
pub(super) const TRAIL: Pixels = px(4.);
pub(super) const SORT_ROOM: Pixels = px(16.);
const CUSHION: Pixels = px(1.);

pub mod rank {
    pub const SPARE: u8 = 10;
    pub const NICE: u8 = 30;
    pub const HANDY: u8 = 50;
    pub const USEFUL: u8 = 70;
    pub const ESSENTIAL: u8 = 90;
}

#[derive(Clone, Copy)]
pub enum Width {
    Fixed(Pixels),
    Fill(f32),
    Thumb,
}

impl Width {
    fn share(self) -> f32 {
        match self {
            Self::Fill(share) => share,
            _ => 0.,
        }
    }

    fn fills(self) -> bool {
        matches!(self, Self::Fill(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    Ascending,
    Descending,
}

impl Sort {
    /// The opposite order, which a second click on the same column sorts in.
    pub const fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sorting {
    pub column: String,
    pub order: Sort,
}

pub struct ColumnSpec<F: 'static> {
    pub field: F,
    pub key: &'static str,
    pub header: &'static str,
    pub align: TextAlign,
    pub width: Width,
    pub anchored: bool,
    pub sortable: bool,
    /// The order the first click sorts this column in; a date reads best newest first.
    pub first: Sort,
    pub rank: u8,
}

impl<F: Copy + 'static> ColumnSpec<F> {
    pub const fn ranked(&self, rank: u8) -> Self {
        Self { rank, ..*self }
    }

    pub const fn filling(field: F) -> Self {
        Self {
            field,
            key: "",
            header: "",
            align: TextAlign::Left,
            width: Width::Fill(1.),
            anchored: false,
            sortable: true,
            first: Sort::Ascending,
            rank: rank::ESSENTIAL,
        }
    }

    pub const fn numbering(field: F, width: Pixels) -> Self {
        Self {
            key: "index",
            header: "column-index",
            align: TextAlign::Center,
            width: Width::Fixed(width),
            anchored: true,
            sortable: false,
            ..Self::filling(field)
        }
    }

    pub const fn artwork(field: F) -> Self {
        Self {
            key: "cover",
            width: Width::Thumb,
            anchored: true,
            sortable: false,
            ..Self::filling(field)
        }
    }
}

impl<F: 'static> ColumnSpec<F> {
    pub fn label(&self) -> SharedString {
        match self.header.is_empty() {
            true => SharedString::default(),
            false => i18n::lookup(self.header, None),
        }
    }

    fn natural(&self, metrics: Metrics) -> Pixels {
        match self.width {
            Width::Fixed(width) => width,
            Width::Thumb => metrics.thumb + metrics.pad * 2.,
            Width::Fill(_) => Pixels::ZERO,
        }
    }

    fn chrome(&self) -> Pixels {
        let sorting = match self.sortable {
            true => SORT_ROOM,
            false => Pixels::ZERO,
        };

        PADDING * 2. + sorting + TRAIL + CUSHION
    }

    fn floor(&self, metrics: Metrics, head: Pixels) -> Pixels {
        let bare = match (self.anchored, self.width) {
            (true, _) | (_, Width::Thumb) => self.natural(metrics),
            (false, Width::Fixed(width)) => width.min(MIN_RIGID),
            (false, Width::Fill(_)) => MIN_FLEX,
        };
        match self.anchored {
            true => bare,
            false => bare.max(head + self.chrome()),
        }
    }

    fn comfort(&self, metrics: Metrics, head: Pixels) -> Pixels {
        let wish = match self.width {
            Width::Fill(_) => COMFORT,
            Width::Fixed(width) => width,
            Width::Thumb => self.natural(metrics),
        };

        wish.max(self.floor(metrics, head))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub widths: HashMap<String, f32>,
}

impl Layout {
    pub fn hides(&self, key: &str) -> bool {
        self.hidden.iter().any(|hidden| hidden == key)
    }

    fn rank(&self, key: &str) -> usize {
        self.order
            .iter()
            .position(|ordered| ordered == key)
            .unwrap_or(usize::MAX)
    }

    fn width(&self, key: &str) -> Option<Pixels> {
        self.widths
            .get(key)
            .copied()
            .filter(|width| width.is_finite() && *width > 0.)
            .map(px)
    }

    pub fn toggle(&mut self, key: &str) {
        match self.hidden.iter().position(|hidden| hidden == key) {
            Some(at) => {
                self.hidden.remove(at);
            }
            None => self.hidden.push(key.to_owned()),
        }
    }
}

pub struct Resolved<F: 'static> {
    pub spec: &'static ColumnSpec<F>,
    pub width: Pixels,
    pub floor: Pixels,
}

struct Track<F: 'static> {
    spec: &'static ColumnSpec<F>,
    width: Pixels,
    floor: Pixels,
    pinned: bool,
}

impl<F: 'static> Track<F> {
    fn elastic(&self) -> bool {
        !self.spec.anchored && !matches!(self.spec.width, Width::Thumb)
    }

    fn slack(&self) -> Pixels {
        match self.elastic() {
            true => (self.width - self.floor).max(Pixels::ZERO),
            false => Pixels::ZERO,
        }
    }
}

pub fn resolve<F: 'static>(
    specs: &'static [ColumnSpec<F>],
    room: Pixels,
    metrics: Metrics,
    layout: &Layout,
    heads: &[Pixels],
    blank: &[bool],
) -> Vec<Resolved<F>> {
    let available = (room - SLACK).max(MIN_TABLE);
    let head = |ix: usize| heads.get(ix).copied().unwrap_or_default();
    let mut visible = ordered(specs, layout)
        .into_iter()
        .filter(|(ix, spec)| !layout.hides(spec.key) && !blank.get(*ix).copied().unwrap_or(false))
        .collect::<Vec<_>>();
    if visible.is_empty() {
        visible.extend(specs.iter().enumerate().take(1));
    }
    crowd(&mut visible, available, metrics, &head);

    let mut tracks = seed(visible, available, metrics, layout, &head);
    fit(&mut tracks, available);

    tracks
        .into_iter()
        .map(|track| Resolved {
            spec: track.spec,
            width: track.width,
            floor: track.floor,
        })
        .collect()
}

type Seat<F> = (usize, &'static ColumnSpec<F>);

fn ordered<F: 'static>(specs: &'static [ColumnSpec<F>], layout: &Layout) -> Vec<Seat<F>> {
    let (anchored, movable): (Vec<_>, Vec<_>) = specs
        .iter()
        .enumerate()
        .partition(|(_, spec)| spec.anchored);
    let mut movable = movable;
    movable.sort_by_key(|(_, spec)| layout.rank(spec.key));

    anchored.into_iter().chain(movable).collect()
}

fn crowd<F: 'static>(
    visible: &mut Vec<Seat<F>>,
    available: Pixels,
    metrics: Metrics,
    head: &impl Fn(usize) -> Pixels,
) {
    while movable(visible) > 1 {
        let wanted = visible
            .iter()
            .map(|(ix, spec)| spec.comfort(metrics, head(*ix)))
            .fold(Pixels::ZERO, |total, comfort| total + comfort);
        if wanted <= available {
            return;
        }

        let evicted = visible
            .iter()
            .enumerate()
            .filter(|(_, (_, spec))| !spec.anchored)
            .min_by(|(left, (_, one)), (right, (_, other))| {
                one.rank.cmp(&other.rank).then(right.cmp(left))
            })
            .map(|(at, _)| at);
        let Some(evicted) = evicted else {
            return;
        };
        visible.remove(evicted);
    }
}

fn movable<F: 'static>(visible: &[Seat<F>]) -> usize {
    visible.iter().filter(|(_, spec)| !spec.anchored).count()
}

fn seed<F: 'static>(
    visible: Vec<Seat<F>>,
    available: Pixels,
    metrics: Metrics,
    layout: &Layout,
    head: &impl Fn(usize) -> Pixels,
) -> Vec<Track<F>> {
    let rigid = visible
        .iter()
        .filter(|(_, spec)| !spec.width.fills() || layout.width(spec.key).is_some())
        .map(|(_, spec)| match layout.width(spec.key) {
            Some(width) if !spec.anchored => width,
            _ => spec.natural(metrics),
        })
        .fold(Pixels::ZERO, |total, width| total + width);

    let free: f32 = visible
        .iter()
        .filter(|(_, spec)| spec.width.fills() && layout.width(spec.key).is_none())
        .map(|(_, spec)| spec.width.share())
        .sum();
    let spare = (available - rigid).max(Pixels::ZERO);
    let loose = visible
        .iter()
        .filter(|(_, spec)| spec.width.fills() && layout.width(spec.key).is_none())
        .count();

    visible
        .into_iter()
        .map(|(ix, spec)| {
            let pinned = !spec.anchored && layout.width(spec.key).is_some();
            let width = match (layout.width(spec.key), spec.width) {
                (Some(width), _) if !spec.anchored => width,
                (_, Width::Fill(share)) if free > 0. => spare * (share / free),
                (_, Width::Fill(_)) if loose > 0 => spare / loose as f32,
                _ => spec.natural(metrics),
            };
            let floor = spec.floor(metrics, head(ix));

            Track {
                spec,
                width: width.max(floor),
                floor,
                pinned,
            }
        })
        .collect()
}

fn fit<F: 'static>(tracks: &mut [Track<F>], available: Pixels) {
    if tracks.is_empty() {
        return;
    }

    let total = tracks
        .iter()
        .map(|track| track.width)
        .fold(Pixels::ZERO, |total, width| total + width);

    match total > available {
        true => shrink(tracks, total - available),
        false => grow(tracks, available - total),
    }

    spread_leftover(tracks, available);
}

fn shrink<F: 'static>(tracks: &mut [Track<F>], excess: Pixels) {
    let mut excess = excess;
    for stage in [Stage::Fluid, Stage::Any] {
        if excess <= Pixels::ZERO {
            return;
        }
        let slack = tracks
            .iter()
            .filter(|track| stage.takes(track))
            .map(|track| track.slack())
            .fold(Pixels::ZERO, |total, slack| total + slack);
        if slack <= Pixels::ZERO {
            continue;
        }

        let take = excess.min(slack);
        for track in tracks.iter_mut().filter(|track| stage.takes(track)) {
            let share = track.slack() / slack;
            track.width = (track.width - take * share).max(track.floor);
        }
        excess -= take;
    }
}

fn grow<F: 'static>(tracks: &mut [Track<F>], leftover: Pixels) {
    if leftover <= Pixels::ZERO {
        return;
    }

    let shares: f32 = tracks
        .iter()
        .filter(|track| track.spec.width.fills() && !track.pinned)
        .map(|track| track.spec.width.share())
        .sum();
    if shares > 0. {
        for track in tracks
            .iter_mut()
            .filter(|track| track.spec.width.fills() && !track.pinned)
        {
            track.width += leftover * (track.spec.width.share() / shares);
        }
        return;
    }

    let fluid = tracks
        .iter()
        .filter(|track| track.spec.width.fills())
        .map(|track| track.width)
        .fold(Pixels::ZERO, |total, width| total + width);
    if fluid > Pixels::ZERO {
        for track in tracks.iter_mut().filter(|track| track.spec.width.fills()) {
            let share = track.width / fluid;
            track.width += leftover * share;
        }
        return;
    }

    if let Some(track) = tracks.iter_mut().rev().find(|track| track.elastic()) {
        track.width += leftover;
    }
}

fn spread_leftover<F: 'static>(tracks: &mut [Track<F>], available: Pixels) {
    let total = tracks
        .iter()
        .map(|track| track.width)
        .fold(Pixels::ZERO, |total, width| total + width);
    let residual = available - total;
    if residual.abs() < px(0.01) {
        return;
    }

    let last = tracks
        .iter()
        .rposition(|track| track.elastic() && track.width + residual >= track.floor);
    if let Some(last) = last {
        tracks[last].width += residual;
    }
}

#[derive(Clone, Copy)]
enum Stage {
    Fluid,
    Any,
}

impl Stage {
    fn takes<F: 'static>(self, track: &Track<F>) -> bool {
        match self {
            Self::Fluid => track.elastic() && track.spec.width.fills(),
            Self::Any => track.elastic(),
        }
    }
}

pub fn stretch<F: 'static>(
    columns: &[Resolved<F>],
    at: usize,
    delta: Pixels,
) -> Option<Vec<(String, f32)>> {
    let mut widths: Vec<Pixels> = columns.iter().map(|column| column.width).collect();
    let floors: Vec<Pixels> = columns.iter().map(|column| column.floor).collect();
    let right: Vec<usize> = (at + 1..columns.len())
        .filter(|ix| !columns[*ix].spec.anchored)
        .filter(|ix| !matches!(columns[*ix].spec.width, Width::Thumb))
        .collect();
    if right.is_empty() || at >= columns.len() || columns[at].spec.anchored {
        return None;
    }

    let capacity = right
        .iter()
        .map(|ix| widths[*ix] - floors[*ix])
        .fold(Pixels::ZERO, |total, slack| total + slack);
    let delta = delta.clamp(floors[at] - widths[at], capacity);
    if delta.abs() < px(0.01) {
        return None;
    }

    widths[at] += delta;
    match delta > Pixels::ZERO {
        true => {
            let slack = capacity.max(px(0.01));
            for ix in &right {
                let share = (widths[*ix] - floors[*ix]) / slack;
                widths[*ix] = (widths[*ix] - delta * share).max(floors[*ix]);
            }
        }
        false => {
            let total = right
                .iter()
                .map(|ix| widths[*ix])
                .fold(Pixels::ZERO, |total, width| total + width)
                .max(px(0.01));
            for ix in &right {
                let share = widths[*ix] / total;
                widths[*ix] -= delta * share;
            }
        }
    }

    Some(
        columns
            .iter()
            .zip(widths)
            .filter(|(column, _)| !column.spec.anchored)
            .filter(|(column, _)| !matches!(column.spec.width, Width::Thumb))
            .map(|(column, width)| (column.spec.key.to_owned(), width / px(1.)))
            .collect(),
    )
}

pub fn shifted<F: 'static>(columns: &[Resolved<F>], from: usize, delta: Pixels) -> usize {
    let anchored = columns.iter().filter(|column| column.spec.anchored).count();
    let mut to = from;
    let mut travelled = Pixels::ZERO;

    match delta > Pixels::ZERO {
        true => {
            for (ix, column) in columns.iter().enumerate().skip(from + 1) {
                travelled += column.width;
                if delta < travelled - column.width / 2. {
                    break;
                }
                to = ix;
            }
        }
        false => {
            for ix in (anchored..from).rev() {
                travelled -= columns[ix].width;
                if delta > travelled + columns[ix].width / 2. {
                    break;
                }
                to = ix;
            }
        }
    }

    to.max(anchored)
}

pub fn reordered<F: 'static>(columns: &[Resolved<F>], from: usize, to: usize) -> Vec<String> {
    let mut keys: Vec<String> = columns
        .iter()
        .filter(|column| !column.spec.anchored)
        .map(|column| column.spec.key.to_owned())
        .collect();
    let anchored = columns.iter().filter(|column| column.spec.anchored).count();
    let (Some(from), Some(to)) = (from.checked_sub(anchored), to.checked_sub(anchored)) else {
        return keys;
    };
    if from >= keys.len() || to >= keys.len() {
        return keys;
    }

    let key = keys.remove(from);
    keys.insert(to, key);
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        Index,
        Cover,
        Title,
        Artist,
        Added,
        Length,
    }

    static SPECS: &[ColumnSpec<Field>] = &[
        ColumnSpec {
            field: Field::Index,
            key: "index",
            header: "",
            align: TextAlign::Center,
            width: Width::Fixed(px(44.)),
            anchored: true,
            sortable: false,
            first: Sort::Ascending,
            rank: rank::ESSENTIAL,
        },
        ColumnSpec {
            field: Field::Cover,
            key: "cover",
            header: "",
            align: TextAlign::Left,
            width: Width::Thumb,
            anchored: true,
            sortable: false,
            first: Sort::Ascending,
            rank: rank::ESSENTIAL,
        },
        ColumnSpec {
            field: Field::Title,
            key: "title",
            header: "",
            align: TextAlign::Left,
            width: Width::Fill(0.5),
            anchored: false,
            sortable: true,
            first: Sort::Ascending,
            rank: rank::ESSENTIAL,
        },
        ColumnSpec {
            field: Field::Artist,
            key: "artist",
            header: "",
            align: TextAlign::Left,
            width: Width::Fill(0.5),
            anchored: false,
            sortable: true,
            first: Sort::Ascending,
            rank: rank::SPARE,
        },
        ColumnSpec {
            field: Field::Added,
            key: "added",
            header: "",
            align: TextAlign::Left,
            width: Width::Fixed(px(112.)),
            anchored: false,
            sortable: true,
            first: Sort::Ascending,
            rank: rank::NICE,
        },
        ColumnSpec {
            field: Field::Length,
            key: "length",
            header: "",
            align: TextAlign::Right,
            width: Width::Fixed(px(72.)),
            anchored: false,
            sortable: true,
            first: Sort::Ascending,
            rank: rank::USEFUL,
        },
    ];

    fn metrics() -> Metrics {
        Metrics::default()
    }

    fn laid(room: Pixels, layout: &Layout) -> Vec<Resolved<Field>> {
        resolve(SPECS, room, metrics(), layout, &[], &[])
    }

    fn headed(room: Pixels, heads: &[Pixels]) -> Vec<Resolved<Field>> {
        resolve(SPECS, room, metrics(), &Layout::default(), heads, &[])
    }

    fn head_of(key: &str, width: Pixels) -> Vec<Pixels> {
        SPECS
            .iter()
            .map(|spec| match spec.key == key {
                true => width,
                false => Pixels::ZERO,
            })
            .collect()
    }

    fn total<F: 'static>(columns: &[Resolved<F>]) -> Pixels {
        columns
            .iter()
            .map(|column| column.width)
            .fold(Pixels::ZERO, |total, width| total + width)
    }

    fn keys<F: 'static>(columns: &[Resolved<F>]) -> Vec<&'static str> {
        columns.iter().map(|column| column.spec.key).collect()
    }

    fn width<F: 'static>(columns: &[Resolved<F>], key: &str) -> Pixels {
        columns
            .iter()
            .find(|column| column.spec.key == key)
            .map(|column| column.width)
            .unwrap_or_default()
    }

    #[test]
    fn columns_exactly_fill_the_room() {
        for room in [px(400.), px(700.), px(1200.), px(2400.)] {
            let columns = laid(room, &Layout::default());
            let expected = (room - SLACK).max(MIN_TABLE);
            assert!(
                (total(&columns) - expected).abs() < px(0.5),
                "room {room:?} produced {:?}",
                total(&columns)
            );
        }
    }

    #[test]
    fn narrow_rooms_drop_columns_but_still_fill() {
        let columns = laid(px(400.), &Layout::default());
        assert!(!keys(&columns).contains(&"added"));
        assert!((total(&columns) - px(398.)).abs() < px(0.5));
    }

    #[test]
    fn fill_columns_split_by_share() {
        let columns = laid(px(1000.), &Layout::default());
        assert!((width(&columns, "title") - width(&columns, "artist")).abs() < px(0.5));
    }

    #[test]
    fn a_saved_order_moves_movable_columns_only() {
        let layout = Layout {
            order: vec!["length".to_owned(), "title".to_owned()],
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert_eq!(keys(&columns)[..2], ["index", "cover"]);
        assert_eq!(keys(&columns)[2], "length");
        assert_eq!(keys(&columns)[3], "title");
    }

    #[test]
    fn hidden_columns_are_dropped() {
        let layout = Layout {
            hidden: vec!["artist".to_owned()],
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert!(!keys(&columns).contains(&"artist"));
        assert!((total(&columns) - px(998.)).abs() < px(0.5));
    }

    #[test]
    fn hiding_everything_keeps_one_column() {
        let layout = Layout {
            hidden: SPECS.iter().map(|spec| spec.key.to_owned()).collect(),
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert_eq!(keys(&columns), ["index"]);
    }

    #[test]
    fn saved_widths_are_honoured() {
        let layout = Layout {
            widths: HashMap::from([("length".to_owned(), 140.)]),
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert!((width(&columns, "length") - px(140.)).abs() < px(0.5));
        assert!((total(&columns) - px(998.)).abs() < px(0.5));
    }

    #[test]
    fn saved_widths_never_break_the_floor() {
        let layout = Layout {
            widths: HashMap::from([("title".to_owned(), 1.)]),
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert!(width(&columns, "title") >= MIN_FLEX);
    }

    #[test]
    fn anchored_columns_keep_their_natural_width() {
        let layout = Layout {
            widths: HashMap::from([("index".to_owned(), 400.)]),
            ..Layout::default()
        };
        let columns = laid(px(1000.), &layout);

        assert!((width(&columns, "index") - px(44.)).abs() < px(0.5));
    }

    #[test]
    fn stretching_takes_room_from_the_right() {
        let columns = laid(px(1000.), &Layout::default());
        let before = width(&columns, "title");
        let widths = stretch(&columns, 2, px(60.)).expect("resizable");
        let layout = Layout {
            widths: widths.into_iter().collect(),
            ..Layout::default()
        };
        let after = laid(px(1000.), &layout);

        assert!((width(&after, "title") - (before + px(60.))).abs() < px(1.));
        assert!((total(&after) - px(998.)).abs() < px(0.5));
    }

    #[test]
    fn stretching_stops_at_the_floors_on_the_right() {
        let columns = laid(px(1000.), &Layout::default());
        let widths = stretch(&columns, 2, px(5000.)).expect("resizable");
        let layout = Layout {
            widths: widths.into_iter().collect(),
            ..Layout::default()
        };
        let after = laid(px(1000.), &layout);

        assert!(width(&after, "artist") >= MIN_FLEX - px(0.5));
        assert!((total(&after) - px(998.)).abs() < px(0.5));
    }

    #[test]
    fn the_last_column_has_no_room_to_take() {
        let columns = laid(px(1000.), &Layout::default());
        let last = columns.len() - 1;

        assert!(stretch(&columns, last, px(40.)).is_none());
    }

    #[test]
    fn anchored_columns_cannot_be_stretched() {
        let columns = laid(px(1000.), &Layout::default());

        assert!(stretch(&columns, 0, px(40.)).is_none());
    }

    #[test]
    fn dragging_past_half_a_neighbour_shifts_by_one() {
        let columns = laid(px(1000.), &Layout::default());
        let next = columns[3].width;

        assert_eq!(shifted(&columns, 2, next / 2. - px(1.)), 2);
        assert_eq!(shifted(&columns, 2, next / 2. + px(1.)), 3);
    }

    #[test]
    fn dragging_never_enters_the_anchored_prefix() {
        let columns = laid(px(1000.), &Layout::default());

        assert_eq!(shifted(&columns, 3, px(-5000.)), 2);
    }

    #[test]
    fn a_column_is_never_narrower_than_its_header() {
        let heads = head_of("length", px(120.));
        let columns = headed(px(1200.), &heads);
        let length = columns
            .iter()
            .find(|column| column.spec.key == "length")
            .expect("kept");

        assert!(length.width >= px(120.) + PADDING * 2. + SORT_ROOM + TRAIL);
        assert!((total(&columns) - px(1198.)).abs() < px(0.5));
    }

    #[test]
    fn a_header_that_cannot_fit_drops_its_column() {
        let heads = head_of("artist", px(900.));
        let columns = headed(px(700.), &heads);

        assert!(!keys(&columns).contains(&"artist"));
        assert!((total(&columns) - px(698.)).abs() < px(0.5));
    }

    #[test]
    fn crowding_evicts_the_least_important_column_first() {
        let heads: Vec<Pixels> = SPECS.iter().map(|_| px(150.)).collect();
        let columns = headed(px(700.), &heads);

        assert!(!keys(&columns).contains(&"artist"));
        assert!(keys(&columns).contains(&"title"));
    }

    #[test]
    fn columns_leave_in_order_of_rank() {
        let mut leaving: Vec<&str> = Vec::new();
        let mut wider = keys(&laid(px(2000.), &Layout::default()));
        for room in (200..2000).step_by(4).rev() {
            let present = keys(&laid(px(room as f32), &Layout::default()));
            leaving.extend(wider.iter().filter(|key| !present.contains(key)));
            wider = present;
        }

        assert_eq!(leaving, ["artist", "added", "length"]);
    }

    #[test]
    fn widening_never_takes_a_column_away() {
        let mut previous: Vec<&str> = Vec::new();
        for room in (200..2000).step_by(4) {
            let present = keys(&laid(px(room as f32), &Layout::default()));
            for key in &previous {
                assert!(
                    present.contains(key),
                    "{key} vanished when the room grew to {room}"
                );
            }
            previous = present;
        }
    }

    #[test]
    fn a_kept_column_has_room_to_read() {
        let heads: Vec<Pixels> = SPECS.iter().map(|_| px(40.)).collect();
        for room in [px(300.), px(560.), px(900.), px(1600.)] {
            let columns = headed(room, &heads);
            let last = columns.len() - 1;
            for (at, column) in columns.iter().enumerate() {
                if column.spec.anchored || !column.spec.width.fills() || at == last {
                    continue;
                }
                assert!(
                    column.width >= MIN_FLEX,
                    "{} is unreadable at room {room:?}",
                    column.spec.key
                );
            }
        }
    }

    #[test]
    fn every_surviving_header_fits_however_tight_the_room() {
        let heads: Vec<Pixels> = SPECS.iter().map(|_| px(130.)).collect();
        for room in [px(300.), px(500.), px(800.), px(1600.)] {
            let columns = headed(room, &heads);
            for column in &columns {
                if column.spec.anchored {
                    continue;
                }
                assert!(
                    column.width >= px(130.) + column.spec.chrome(),
                    "{} truncates at room {room:?}",
                    column.spec.key
                );
            }
        }
    }

    #[test]
    fn reordering_rewrites_the_movable_keys() {
        let columns = laid(px(1000.), &Layout::default());

        assert_eq!(
            reordered(&columns, 2, 4),
            ["artist", "added", "title", "length"]
        );
        assert_eq!(
            reordered(&columns, 5, 2),
            ["length", "title", "artist", "added"]
        );
    }
}
