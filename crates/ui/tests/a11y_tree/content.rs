//! Content primitives: the track table, the range scrubber, the response graph, a shelf with its
//! paging arrows and return button, and inline links. Each has its own state, so each mounts its
//! own runtime.

use std::borrow::Cow;

use rune::{Cx, El, Env, Runtime, col};
use serde_json::Value;
use ui::{
    ColumnSpec, GraphEdit, InlineLink, InlineLinks, RangeScrubber, ResponseAxes, ResponseGraph,
    ResponseGraphState, ResponseNode, ResponsePoint, ScrollWatch, Shelf, Table, TableState,
    return_top,
};

use super::support::{find, said, theme, with_role};

/// `view` over `state`, laid out and settled, as a screen reader would get it.
fn tree<S: 'static>(state: S, view: fn(&S, &mut Cx<S>) -> El) -> Value {
    let mut rt = Runtime::new(state, view, (900., 700.), Env::new(true, 2.));
    rt.frame(0.);
    rt.settle_all();
    rt.frame(1. / 60.);
    rt.accessibility_json()
}

/// Role and name of the node keyed `key`, panicking with the tree when there is none.
fn spoken<'a>(tree: &'a Value, key: &str) -> (&'a str, &'a str) {
    let Some(node) = find(tree, key) else {
        panic!("no node keyed {key:?} in {tree}");
    };
    said(node)
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Index,
    Title,
}

static COLUMNS: [ColumnSpec<Field>; 2] = [
    ColumnSpec::numbering(Field::Index, 40.),
    ColumnSpec {
        key: "title",
        header: "column-title",
        ..ColumnSpec::filling(Field::Title)
    },
];

struct Songs {
    ids: Vec<String>,
    table: TableState,
}

fn songs(s: &Songs, _: &mut Cx<Songs>) -> El {
    let theme = theme();
    col().w_full().h_full().child(
        Table::new(
            "songs",
            &COLUMNS,
            |s: &Songs| &s.table,
            |s: &mut Songs| &mut s.table,
            |s: &Songs| s.ids.len(),
            |s: &Songs, i| Cow::Borrowed(s.ids[i].as_str()),
        )
        .width(900.)
        .label("Songs")
        .name(|s: &Songs, i| format!("Song {}", s.ids[i]))
        .cell(|s: &Songs, cell| {
            let ink = super::support::theme();
            cell.text(s.ids[cell.row].clone(), ink.foreground, &ink)
        })
        .build(s, &theme),
    )
}

#[test]
fn a_table_reads_as_a_grid_of_named_rows_under_its_headers() {
    let mut table = TableState::new();
    table.select("t1");
    let tree = tree(
        Songs {
            ids: (0..1000).map(|i| format!("t{i}")).collect(),
            table,
        },
        songs,
    );
    assert_eq!(spoken(&tree, "songs"), ("grid", "Songs"));
    assert_eq!(spoken(&tree, "head-title"), ("column-header", "Title"));
    assert_eq!(spoken(&tree, "head-index"), ("column-header", "#"));
    assert_eq!(spoken(&tree, "t0"), ("row", "Song t0"));
    let (role, name) = spoken(&tree, "grip-title");
    assert_eq!(role, "splitter");
    assert!(
        name.contains("Title"),
        "the grip is named for its column: {name:?}"
    );
    // Only the rows near the viewport are built, so a long list stays cheap.
    assert!(with_role(&tree, "row").len() < 100);
}

struct Range {
    value: (f32, f32),
}

fn range(s: &Range, _: &mut Cx<Range>) -> El {
    col().w(300.).child(
        RangeScrubber::new("years", s.value)
            .name("Year")
            .values("1990", "2020")
            .on_change(|s: &mut Range, value, _| s.value = value)
            .build(&theme()),
    )
}

#[test]
fn a_range_scrubber_is_a_named_group_of_two_sliders() {
    let tree = tree(Range { value: (0.2, 0.8) }, range);
    assert_eq!(spoken(&tree, "years"), ("group", "Year"));
    assert_eq!(spoken(&tree, "years-low"), ("slider", "From"));
    assert_eq!(spoken(&tree, "years-high"), ("slider", "To"));
}

struct Graph {
    state: ResponseGraphState,
    edits: usize,
}

fn graph(s: &Graph, _: &mut Cx<Graph>) -> El {
    let axes = ResponseAxes::new(20.0..=20000., -12.0..=12.).expect("valid axes");
    let node = ResponseNode {
        id: 1,
        position: ResponsePoint {
            frequency_hz: 1000.,
            gain_db: 3.,
        },
        readout_db: 3.,
        gain_adjustable: true,
        selected: true,
        enabled: true,
    };
    col().child(
        ResponseGraph::new("eq", &s.state, axes, 600.)
            .response((0..64).map(|i| ResponsePoint {
                frequency_hz: 20. * 1000f32.powf(i as f32 / 63.),
                gain_db: 3. * (i as f32 / 10.).sin(),
            }))
            .nodes([node])
            .on_edit(|s: &mut Graph, _: GraphEdit, _| s.edits += 1)
            .build(&theme()),
    )
}

#[test]
fn a_response_graph_node_reads_its_frequency_and_gain() {
    let tree = tree(
        Graph {
            state: ResponseGraphState::new(),
            edits: 0,
        },
        graph,
    );
    let (role, name) = spoken(&tree, "eq-node-1");
    assert_eq!(role, "button");
    assert!(
        name.contains("kHz") && name.contains('3'),
        "node read as {name:?}"
    );
}

struct Page {
    rail: ScrollWatch,
    page: ScrollWatch,
}

fn page(s: &Page, _: &mut Cx<Page>) -> El {
    let theme = theme();
    let cards = (0..12).map(|i| {
        col()
            .key(format!("card-{i}"))
            .size(140., 140.)
            .role("button")
            .label(format!("Card {i}"))
    });
    col()
        .w_full()
        .h_full()
        .child(
            Shelf::new("rail", &s.rail)
                .items(cards)
                .build::<Page>(&theme),
        )
        .child(return_top::<Page>("page", &s.page, &theme))
}

#[test]
fn a_shelf_names_its_paging_arrows_and_a_page_its_return_button() {
    let tree = tree(
        Page {
            rail: ScrollWatch::new(),
            page: ScrollWatch::new(),
        },
        page,
    );
    assert_eq!(spoken(&tree, "rail-previous"), ("button", "Previous"));
    assert_eq!(spoken(&tree, "rail-next"), ("button", "Next"));
    assert_eq!(spoken(&tree, "page-return-top"), ("button", "Back to top"));
    assert_eq!(spoken(&tree, "card-3"), ("button", "Card 3"));
}

struct Artists {
    opened: Vec<String>,
}

fn artists(_: &Artists, _: &mut Cx<Artists>) -> El {
    let theme = theme();
    col().w(400.).child(
        InlineLinks::new(
            "artists",
            [
                InlineLink::new("Lunar Comets", Some("artist:1".into())),
                InlineLink::new("Dust Drift", Some("artist:2".into())),
            ],
            "Unknown artist",
            theme.muted_foreground,
        )
        .on_link(|s: &mut Artists, value: &str, _| s.opened.push(value.to_owned()))
        .build(&theme),
    )
}

#[test]
fn inline_links_read_as_named_links() {
    let tree = tree(Artists { opened: Vec::new() }, artists);
    let names: Vec<&str> = with_role(&tree, "link")
        .into_iter()
        .map(|link| said(link).1)
        .collect();
    assert_eq!(names, ["Lunar Comets", "Dust Drift"], "in {tree}");
}
