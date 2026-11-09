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

mod more {
    use std::sync::{Arc, Mutex};
    use rune::{Cx, El, Env, Runtime, col};
    use ui::*;

    fn png() -> Vec<u8> {
        let img = image::RgbaImage::from_fn(300, 200, |x, _| image::Rgba([200, (x % 255) as u8, 40, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    struct A { art: Artwork }
    fn aview(s: &A, _: &mut Cx<A>) -> El {
        let t = Theme::dark();
        col().child(cover(&s.art, Some("u1"), 100., &t).key("c1")).child(avatar(&s.art, Some("bad"), 40., &t).key("c2"))
    }

    #[test]
    fn artwork_loads() {
        let bytes = png();
        let woke = Arc::new(Mutex::new(0));
        let w2 = woke.clone();
        let art = Artwork::new(
            Arc::new(move |url: String| { let b = bytes.clone(); Box::pin(async move { if url == "bad" { anyhow::bail!("nope") } Ok(b) }) }),
            Arc::new(|f| { std::thread::spawn(move || futures::executor::block_on(f)); }),
            Arc::new(move || *w2.lock().unwrap() += 1),
        );
        let mut rt = Runtime::new(A { art: art.clone() }, aview, (400., 400.), Env::new(true, 2.));
        rt.frame(0.);
        std::thread::sleep(std::time::Duration::from_millis(500));
        rt.frame(0.1); rt.frame(0.5);
        println!("usage {:?} palette {:?}", art.usage(), art.cover_palette("u1").map(|p| p.primary));
        println!("palette sample {:?}", art.palette("u1").is_some());
        for e in rt.elements().iter().filter(|e| e.rect[2] > 0.) { println!("{:?} {:?} op {}", e.key, e.rect, e.opacity); }
    }

    struct V { vis: Visualizer }
    fn vview(s: &V, _: &mut Cx<V>) -> El {
        let t = Theme::dark();
        col().child(s.vis.view(800., 160.).style(VisualizerStyle::Both).corner_radius(12.).build(&t))
    }
    #[test]
    fn visualizer_pushes() {
        let vis = Visualizer::new(32);
        let mut rt = Runtime::new(V { vis: vis.clone() }, vview, (800., 400.), Env::new(true, 2.));
        rt.frame(0.);
        let lv = Levels { left: (0..32).map(|i| (i as f32 / 32.)).collect(), right: (0..32).map(|i| 1. - i as f32 / 32.).collect() };
        let t = std::time::Instant::now();
        for _ in 0..10 { vis.push(&lv); }
        println!("10 pushes {:?}", t.elapsed());
        rt.frame(0.016);
        println!("ok");
    }

    struct R { v: (f32, f32) }
    fn rview(s: &R, _: &mut Cx<R>) -> El {
        let t = Theme::dark();
        col().w(400.).child(RangeScrubber::new("yr", s.v).on_change(|s: &mut R, v, _| s.v = v).build(&t))
    }
    #[test]
    fn scrubber_drags() {
        let mut rt = Runtime::new(R { v: (0.2, 0.8) }, rview, (400., 100.), Env::new(true, 2.));
        rt.frame(0.);
        for e in rt.elements().iter().filter(|e| e.key.is_some()) { println!("{:?} {:?}", e.key, e.rect); }
        let hi = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("yr-high")).unwrap().rect;
        let (x, y) = (hi[0] + hi[2] / 2., hi[1] + hi[3] / 2.);
        rt.pointer_down(x, y); rt.frame(0.016);
        rt.pointer_move(x - 100., y); rt.frame(0.016);
        rt.pointer_up(x - 100., y); rt.frame(0.016);
        println!("value {:?}", rt.state.v);
        rt.frame(0.5);
        let hi2 = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("yr-high")).unwrap().rect;
        println!("hi moved {:?} -> {:?}", hi, hi2);
    }

    struct G { st: ResponseGraphState, pos: ResponsePoint, sel: bool, log: Vec<String> }
    fn gview(s: &G, _: &mut Cx<G>) -> El {
        let t = Theme::dark();
        let axes = ResponseAxes::new(20.0..=20000., -12.0..=12.).unwrap();
        col().child(ResponseGraph::new("eq", &s.st, axes, 600.).reference(2., "Preamp")
            .response((0..64).map(|i| ResponsePoint { frequency_hz: 20. * 1000f32.powf(i as f32 / 63.), gain_db: 6. * (i as f32 / 10.).sin() }))
            .spectrum((0..64).map(|i| ResponsePoint { frequency_hz: 20. * 1000f32.powf(i as f32 / 63.), gain_db: -40. + 10. * (i as f32 / 7.).cos() }), -80.0..=0.)
            .nodes([ResponseNode { id: 1, position: s.pos, readout_db: s.pos.gain_db, gain_adjustable: true, selected: s.sel, enabled: true }])
            .on_edit(|s: &mut G, e: GraphEdit, _| { s.log.push(format!("{e:?}")); match e { GraphEdit::Move { position, .. } => s.pos = position, GraphEdit::Select(_) => s.sel = true, GraphEdit::Deselect => s.sel = false, _ => {} } })
            .build(&t))
    }
    #[test]
    fn graph_drags() {
        let mut rt = Runtime::new(G { st: ResponseGraphState::new(), pos: ResponsePoint { frequency_hz: 1000., gain_db: 0. }, sel: false, log: vec![] }, gview, (600., 400.), Env::new(true, 2.));
        rt.frame(0.);
        let n = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("eq-node-1")).unwrap().rect;
        let (x, y) = (n[0] + n[2] / 2., n[1] + n[3] / 2.);
        rt.pointer_down(x, y); rt.frame(0.016);
        rt.pointer_move(x + 60., y - 30.); rt.frame(0.016);
        rt.pointer_up(x + 60., y - 30.); rt.frame(0.016);
        println!("log {:?}", rt.state.log);
        // click empty plot
        rt.pointer_down(150., 250.); rt.frame(0.016); rt.pointer_up(150., 250.); rt.frame(0.016);
        rt.pointer_down(150., 250.); rt.frame(0.03); rt.pointer_up(150., 250.); rt.frame(0.016);
        println!("log2 {:?}", rt.state.log);
    }
}
