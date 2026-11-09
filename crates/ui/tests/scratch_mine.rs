use std::borrow::Cow;
use rune::{Cx, El, Env, Runtime, col};
use ui::{ColumnSpec, Table, TableState, Theme, TextAlign, Width};

#[derive(Clone, Copy, PartialEq)]
enum F { Index, Title, Artist }

static SPECS: [ColumnSpec<F>; 3] = [
    ColumnSpec::numbering(F::Index, 40.),
    ColumnSpec { key: "title", header: "column-title", ..ColumnSpec::filling(F::Title) },
    ColumnSpec { key: "artist", header: "column-artist", ..ColumnSpec::filling(F::Artist) },
];

struct App { ids: Vec<String>, table: TableState, opened: Vec<String>, sorted: u32 }

fn view(s: &App, _cx: &mut Cx<App>) -> El {
    let theme = Theme::dark();
    col().w_full().h_full().child(
        Table::new("tracks", &SPECS, |s: &App| &s.table, |s: &mut App| &mut s.table, |s: &App| s.ids.len(), |s: &App, i| Cow::Borrowed(s.ids[i].as_str()))
            .width(600.)
            .label("Tracks")
            .lead(rune::el().key("hero").w_full().h(200.))
            .name(|s: &App, i| format!("Track {}", s.ids[i]))
            .cell(|s: &App, c| { let t = Theme::dark(); match c.field { F::Index => c.text(format!("{}", c.row + 1), t.muted_foreground, &t), F::Title => c.text(s.ids[c.row].clone(), t.foreground, &t), F::Artist => c.text("Artist", t.muted_foreground, &t) } })
            .on_open(|s: &mut App, id: &str, _| s.opened.push(id.to_owned()))
            .on_sort(|s: &mut App, _, _| s.sorted += 1)
            .build(s, &theme),
    )
}

#[test]
fn table_works() {
    let _ = (TextAlign::Left, Width::Thumb);
    let ids = (0..100_000).map(|i| format!("t{i}")).collect();
    let mut rt = Runtime::new(App { ids, table: TableState::new().multi_select(true), opened: vec![], sorted: 0 }, view, (600.0, 500.0), Env::new(true, 2.0));
    let t = std::time::Instant::now();
    rt.frame(0.0);
    println!("first frame {:?}", t.elapsed());
    let els = rt.elements();
    let n = els.iter().filter(|e| e.role == "row").count();
    println!("rows mounted {n}");
    for e in els.iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("head") || k == "hero" || k == "t0" || k.starts_with("grip"))) { println!("{:?} {:?} {:?}", e.key, e.role, e.rect); }
    // click t1
    let r = els.iter().find(|e| e.key.as_deref() == Some("t1")).unwrap().rect;
    rt.pointer_down(r[0]+100., r[1]+10.); rt.frame(0.016); rt.pointer_up(r[0]+100., r[1]+10.); rt.frame(0.016);
    println!("selected {:?}", rt.state.table.selected().collect::<Vec<_>>());
    // scroll down
    for _ in 0..10 { rt.wheel(300., 300., -400.); rt.frame(0.016); }
    let t = std::time::Instant::now();
    for _ in 0..30 { rt.wheel(300., 300., -400.); rt.frame(0.016); }
    println!("30 scroll frames {:?}", t.elapsed());
    let els = rt.elements();
    for e in els.iter().filter(|e| e.key.as_deref().is_some_and(|k| k == "head" || k == "hero")) { println!("after scroll {:?} {:?} stuck", e.key, e.rect); }
    let first = els.iter().filter(|e| e.role == "row").map(|e| (e.key.clone(), e.rect[1])).take(3).collect::<Vec<_>>();
    println!("rows {first:?}");
    // header click sorts
    let h = els.iter().find(|e| e.key.as_deref() == Some("head-title")).unwrap().rect;
    rt.pointer_down(h[0]+20., h[1]+10.); rt.frame(0.016); rt.pointer_up(h[0]+20., h[1]+10.); rt.frame(0.016);
    println!("sorted {} {:?}", rt.state.sorted, rt.state.table.sorting());
    // header drag: does it press?
    rt.pointer_down(h[0]+20., h[1]+10.); rt.frame(0.016); rt.pointer_move(h[0]+200., h[1]+10.); rt.frame(0.016); rt.pointer_up(h[0]+200., h[1]+10.); rt.frame(0.016);
    println!("after drag sorted {} order {:?}", rt.state.sorted, rt.state.table.layout().order);
    let a = rt.accessibility_json().to_string();
    println!("a11y len {} grid {}", a.len(), a.contains("\"grid\""));
    println!("{}", &a[..a.len().min(1500)]);
}
