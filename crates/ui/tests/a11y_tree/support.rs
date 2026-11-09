//! What the cases share: the state every scene reads, a headless runtime, and queries over
//! `Runtime::accessibility_json`.

use rune::accesskit::Action;
use rune::{Cx, El, Env, Runtime, TextEdit};
use serde_json::Value;
use ui::{MenuSearch, Popovers, ScrubberState, Theme};

/// The state every scene reads and its handlers write.
#[derive(Default)]
pub struct App {
    pub presses: u32,
    pub on: bool,
    pub level: f32,
    pub edit: TextEdit,
    pub popovers: Popovers,
    pub search: MenuSearch,
    pub sub: bool,
    pub scrubber: ScrubberState,
    pub dismissed: bool,
}

pub type View = fn(&App, &mut Cx<App>) -> El;

pub fn theme() -> Theme {
    Theme::dark()
}

/// `view` over `app`, laid out and settled.
pub fn mount(app: App, view: View) -> Runtime<App> {
    let mut rt = Runtime::new(app, view, (900., 700.), Env::new(true, 2.));
    settle(&mut rt);
    rt
}

pub fn settle(rt: &mut Runtime<App>) {
    rt.frame(0.);
    rt.settle_all();
    rt.frame(1. / 60.);
    rt.frame(1. / 60.);
}

/// The node keyed `key`; panics with the whole tree when there is none.
pub fn node<'a>(tree: &'a Value, key: &str) -> &'a Value {
    find(tree, key).unwrap_or_else(|| {
        panic!(
            "no node keyed {key:?} in {}",
            serde_json::to_string_pretty(tree).unwrap()
        )
    })
}

pub fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if v["key"] == key {
        return Some(v);
    }
    v["children"].as_array()?.iter().find_map(|c| find(c, key))
}

/// Every node with `role`, in reading order.
pub fn with_role<'a>(v: &'a Value, role: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    walk(v, &mut |n| {
        if n["role"] == role {
            out.push(n)
        }
    });
    out
}

fn walk<'a>(v: &'a Value, f: &mut dyn FnMut(&'a Value)) {
    f(v);
    for c in v["children"].as_array().into_iter().flatten() {
        walk(c, f);
    }
}

pub fn has(v: &Value, state: &str) -> bool {
    v["states"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == state))
}

pub fn can(v: &Value, action: &str) -> bool {
    v["actions"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == action))
}

/// Role and name of `v`.
pub fn said(v: &Value) -> (&str, &str) {
    (
        v["role"].as_str().unwrap_or(""),
        v["name"].as_str().unwrap_or(""),
    )
}

/// What a screen reader asks of the node keyed `key`, run as its frame would run it.
pub fn act(rt: &mut Runtime<App>, key: &str, action: Action) -> bool {
    let ok = rt.accessibility_action(key, action, None);
    settle(rt);
    ok
}
