//! The modal that edits a smart playlist's rules: a recursive tree of match groups and
//! conditions, edited in place and saved through the library. Expressions the model keeps
//! as `Rule::Unknown` render read-only and can only be removed whole, so a save loses
//! nothing it could not show.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, Global, Render, ScrollHandle, SharedString, Subscription,
    Window, div, px,
};
use i18n::t;
use music::{
    Condition, FieldKind, Operand, OperandShape, Operator, Playlist, Rule, RuleGroup,
    SmartPlaylistUpdate, SmartRules, field_kind, parse_number,
};
use state::{LibraryEvent, SessionEvent, Shelf, Sonora};
use ui::{
    ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Input, MenuItem, Modal, Notice, Picker,
    Popovers, Submit,
};

/// What the editor is doing: making a new smart playlist, or changing the rules of one the
/// provider already has.
pub(crate) enum SmartEdit {
    Create { shelf: Shelf },
    Rules { playlist: Playlist },
}

/// Which picker the open row is asking for. One picker at a time keeps the keys static.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pick {
    Field(String),
    Operator(String),
}

/// One flattened row of the rules tree. A path is the rule's index chain down from the root
/// group, spelled like `0-2` for keys.
enum Row {
    Group {
        path: Vec<usize>,
        any: bool,
        root: bool,
    },
    Condition {
        path: Vec<usize>,
    },
    Unknown {
        path: Vec<usize>,
        text: String,
    },
}

/// A condition row's value fields: `low` alone, or both for a range.
struct RowInputs {
    low: Entity<Input>,
    high: Entity<Input>,
}

pub(crate) struct SmartPlaylistEditor {
    edit: Option<SmartEdit>,
    name: Entity<Input>,
    /// The working tree; `None` while the provider's answer is still out.
    rules: Option<SmartRules>,
    /// The opened playlist syncs from a file, so the provider's next scan can overwrite a
    /// save. The editor warns and never turns the sync off itself.
    synced: bool,
    loading: bool,
    failed: Option<String>,
    picking: Option<Pick>,
    /// Each condition row's value fields by path key, remade from the tree after a
    /// structural change.
    values: HashMap<String, RowInputs>,
    sort: Entity<Input>,
    limit: Entity<Input>,
    /// The sort direction once set: `Some(true)` ascends, `Some(false)` descends, and `None`
    /// leaves the server's order alone.
    order_asc: Option<bool>,
    popovers: Popovers,
    scroll: ScrollHandle,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    _listen: Subscription,
    _listen_session: Subscription,
}

struct Installed(Entity<SmartPlaylistEditor>);

impl Global for Installed {}

const FIELD_PICKER: &str = "smart-rules-field";
const OPERATOR_PICKER: &str = "smart-rules-operator";

/// The fields the picker offers. A rule can still name any tag a scan has seen: the server
/// takes it, so an existing rule on one keeps it.
const FIELDS: &[&str] = &[
    "title",
    "album",
    "artist",
    "albumartist",
    "genre",
    "year",
    "playcount",
    "rating",
    "loved",
    "comment",
    "path",
    "dateadded",
    "dateloved",
    "lastplayed",
    "bpm",
    "compilation",
    "duration",
    "tracknumber",
    "discnumber",
    "bitrate",
    "size",
    "missing",
    "hascoverart",
];

/// The path's key spelling, `root` for the top group and `0-2` style chains under it.
fn key_of(path: &[usize]) -> String {
    match path.is_empty() {
        true => "root".to_owned(),
        false => path
            .iter()
            .map(|at| at.to_string())
            .collect::<Vec<_>>()
            .join("-"),
    }
}

/// The rules a path's group holds. Every step but the walk's start names a nested group.
fn rules_at<'a>(rules: &'a mut Vec<Rule>, path: &[usize]) -> &'a mut Vec<Rule> {
    let mut current = rules;
    for &at in path {
        current = match &mut current[at] {
            Rule::All(inner) | Rule::Any(inner) => inner,
            _ => unreachable!("a path only walks groups"),
        };
    }
    current
}

fn group_at<'a>(rules: &'a mut SmartRules, path: &[usize]) -> &'a mut Vec<Rule> {
    rules_at(rules.group.rules_mut(), path)
}

/// The condition a path names, when it still names one.
fn condition_at<'a>(rules: &'a SmartRules, path: &[usize]) -> Option<&'a Condition> {
    let mut current = rules.group.rules();
    for (depth, &at) in path.iter().enumerate() {
        let rule = current.get(at)?;
        match (depth == path.len() - 1, rule) {
            (true, Rule::Condition(condition)) => return Some(condition),
            (false, Rule::All(inner)) | (false, Rule::Any(inner)) => current = inner,
            _ => return None,
        }
    }
    None
}

fn condition_at_mut<'a>(rules: &'a mut SmartRules, path: &[usize]) -> Option<&'a mut Condition> {
    let last = *path.last()?;
    let parent = group_at(rules, &path[..path.len() - 1]);
    parent.get_mut(last)?.condition_mut()
}

/// The rows of the whole tree in reading order, deepest paths included.
fn flatten(rules: &SmartRules) -> Vec<Row> {
    fn walk(rules: &[Rule], path: &mut Vec<usize>, rows: &mut Vec<Row>) {
        for (at, rule) in rules.iter().enumerate() {
            path.push(at);
            match rule {
                Rule::All(inner) | Rule::Any(inner) => {
                    rows.push(Row::Group {
                        path: path.clone(),
                        any: matches!(rule, Rule::Any(_)),
                        root: false,
                    });
                    walk(inner, path, rows);
                }
                Rule::Condition(_) => rows.push(Row::Condition { path: path.clone() }),
                Rule::Unknown(value) => rows.push(Row::Unknown {
                    path: path.clone(),
                    text: value.to_string(),
                }),
            }
            path.pop();
        }
    }
    let mut rows = vec![Row::Group {
        path: Vec::new(),
        any: matches!(rules.group, RuleGroup::Any(_)),
        root: true,
    }];
    walk(rules.group.rules(), &mut Vec::new(), &mut rows);
    rows
}

/// The default condition a new row starts from.
fn blank_condition() -> Rule {
    Rule::Condition(Condition {
        operator: Operator::Is,
        field: "title".to_owned(),
        value: Operand::Text(String::new()),
    })
}

/// The operand's spelling for a fresh text field, or its pair for a range.
fn operand_texts(value: &Operand) -> (String, String) {
    match value {
        Operand::Range(low, high) => (low.to_string(), high.to_string()),
        Operand::Number(number) => (number.to_string(), String::new()),
        Operand::Text(text) => (text.clone(), String::new()),
        Operand::Boolean(truth) => (truth.to_string(), String::new()),
    }
}

/// What a row's text fields make of its operand. Strict parsing rejects text that is no
/// number where the operator or the field needs one; lenient keeps the raw text as a
/// `Text` operand so a structural edit loses nothing, and the shape check flags it on save.
/// A range is the one shape raw text cannot spell, so an invalid range fails either way.
fn collect_operand(
    shape: OperandShape,
    number_field: bool,
    boolean_compared: bool,
    current: &Operand,
    low: &str,
    high: &str,
    strict: bool,
) -> Result<Operand, ()> {
    match shape {
        OperandShape::Truth => Ok(current.clone()),
        OperandShape::Range => match (parse_number(low), parse_number(high)) {
            (Some(low), Some(high)) => Ok(Operand::Range(low, high)),
            _ => Err(()),
        },
        OperandShape::Days => match parse_number(low) {
            Some(days) => Ok(Operand::Number(days)),
            None if strict => Err(()),
            None => Ok(Operand::Text(low.trim().to_owned())),
        },
        OperandShape::Playlist => Ok(Operand::Text(low.trim().to_owned())),
        // Boolean fields with `is` take the toggle's value, never the text.
        OperandShape::Compared if boolean_compared => Ok(current.clone()),
        OperandShape::Compared if number_field => match parse_number(low) {
            Some(number) => Ok(Operand::Number(number)),
            None if strict => Err(()),
            None => Ok(Operand::Text(low.trim().to_owned())),
        },
        OperandShape::Compared => Ok(Operand::Text(low.trim().to_owned())),
    }
}

impl SmartPlaylistEditor {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let editor = cx.new(|cx| {
                let library = Sonora::global(cx).library.clone();
                let listen = cx.subscribe(&library, |this: &mut Self, _, event, cx| {
                    if let LibraryEvent::SmartPlaylistReady(_) = event {
                        this.ready(cx);
                    }
                });
                // A sign-out or a new account closes the editor, creating included: the
                // working rules belong to the account that was signed in when they were
                // read or started, and a save under the next one would go to its library.
                let session = Sonora::global(cx).session.clone();
                let listen_session = cx.subscribe(&session, |this: &mut Self, _, event, cx| {
                    if matches!(event, SessionEvent::SignedIn | SessionEvent::SignedOut) {
                        this.reset(cx);
                    }
                });
                Self {
                    edit: None,
                    name: cx.new(|cx| Input::new(t!("smart-rules-name-placeholder"), cx)),
                    rules: None,
                    synced: false,
                    loading: false,
                    failed: None,
                    picking: None,
                    values: HashMap::new(),
                    sort: cx.new(|cx| Input::new(t!("smart-rules-sort-placeholder"), cx).compact()),
                    limit: cx
                        .new(|cx| Input::new(t!("smart-rules-limit-placeholder"), cx).compact()),
                    order_asc: None,
                    popovers: Popovers::default(),
                    scroll: ScrollHandle::new(),
                    focus: cx.focus_handle(),
                    restore: None,
                    _listen: listen,
                    _listen_session: listen_session,
                }
            });
            cx.set_global(Installed(editor));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn open(edit: SmartEdit, window: &mut Window, cx: &mut App) {
        let editor = Self::entity(cx);
        editor.update(cx, |this, cx| this.show(edit, window, cx));
    }

    fn show(&mut self, edit: SmartEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.restore = window.focused(cx);
        self.picking = None;
        self.values.clear();
        self.failed = None;
        self.synced = false;
        self.order_asc = None;
        self.sort.update(cx, |input, cx| input.set_text("", cx));
        self.limit.update(cx, |input, cx| input.set_text("", cx));
        match &edit {
            SmartEdit::Create { .. } => {
                self.loading = false;
                self.rules = Some(SmartRules::new(RuleGroup::All(vec![blank_condition()])));
                self.name.update(cx, |input, cx| input.set_text("", cx));
            }
            SmartEdit::Rules { playlist } => {
                self.loading = true;
                self.rules = None;
                self.name
                    .update(cx, |input, cx| input.set_text(playlist.name.clone(), cx));
                let id = playlist.id.clone();
                let shelf = Shelf::of(&playlist.id);
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.open_smart_playlist(id, shelf, cx));
            }
        }
        self.name.update(cx, |input, cx| input.focus(window, cx));
        self.edit = Some(edit);
        cx.notify();
    }

    /// The provider answered the open: the tree and the sync warning fill from its view of
    /// the playlist. Ordinary playlists answer `None` rules and start from one blank group.
    fn ready(&mut self, cx: &mut Context<Self>) {
        let Some(SmartEdit::Rules { playlist }) = &self.edit else {
            return;
        };
        let Some(editing) = Sonora::global(cx).library.read(cx).smart_editing.clone() else {
            return;
        };
        if editing.id != playlist.id {
            return;
        }
        self.loading = false;
        match editing.outcome {
            Ok(smart) => {
                self.synced = smart.synced;
                let mut rules = smart.rules.unwrap_or_default();
                if rules.group.is_empty() {
                    rules.group = RuleGroup::All(vec![blank_condition()]);
                }
                self.order_asc = rules.order.as_deref().and_then(|order| match order {
                    "asc" => Some(true),
                    "desc" => Some(false),
                    _ => None,
                });
                self.sort.update(cx, |input, cx| {
                    input.set_text(rules.sort.clone().unwrap_or_default(), cx)
                });
                self.limit.update(cx, |input, cx| {
                    input.set_text(
                        rules
                            .limit
                            .map(|limit| limit.to_string())
                            .unwrap_or_default(),
                        cx,
                    )
                });
                self.values.clear();
                self.rules = Some(rules);
            }
            Err(problem) => self.failed = Some(problem),
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = Sonora::global(cx).library.clone();
        library.update(cx, |library, cx| library.close_smart_playlist(cx));
        let restore = self.restore.take();
        self.reset(cx);
        if let Some(focus) = restore {
            window.focus(&focus, cx);
        }
    }

    /// Drops the open edit and every working value. A session change calls this directly,
    /// so a stale editor never stays up or saves into the next account's library.
    fn reset(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        self.rules = None;
        self.synced = false;
        self.loading = false;
        self.failed = None;
        self.picking = None;
        self.values.clear();
        self.restore = None;
        cx.notify();
    }

    /// Writes every open text field back into the tree. Lenient keeps an unparseable number
    /// as text so a structural edit loses nothing; strict marks the field and fails the
    /// save. Answers false when a strict parse rejected a value.
    fn collect_values(
        &mut self,
        strict: bool,
        exempt: Option<&[usize]>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(rules) = &mut self.rules else {
            return true;
        };
        let mut sound = true;
        for row in flatten(rules) {
            let Row::Condition { path } = row else {
                continue;
            };
            if exempt == Some(path.as_slice()) {
                continue;
            }
            let key = key_of(&path);
            let Some(inputs) = self.values.get(&key) else {
                continue;
            };
            let low = inputs.low.read(cx).text().to_owned();
            let high = inputs.high.read(cx).text().to_owned();
            let Some(condition) = condition_at_mut(rules, &path) else {
                continue;
            };
            let shape = condition.operator.operand_shape();
            let boolean_compared = field_kind(&condition.field) == FieldKind::Boolean
                && matches!(condition.operator, Operator::Is | Operator::IsNot);
            let parsed = collect_operand(
                shape,
                field_kind(&condition.field) == FieldKind::Number,
                boolean_compared,
                &condition.value,
                &low,
                &high,
                strict,
            );
            match parsed {
                Ok(value) => condition.value = value,
                Err(()) => {
                    sound = false;
                    let bad = |text: &str| text.trim().is_empty() || parse_number(text).is_none();
                    inputs
                        .low
                        .update(cx, |input, cx| input.set_invalid(bad(&low), cx));
                    if shape == OperandShape::Range {
                        inputs
                            .high
                            .update(cx, |input, cx| input.set_invalid(bad(&high), cx));
                    }
                }
            }
        }
        sound
    }

    /// Remakes the rows' text fields from the tree, after the tree's shape changed.
    fn rebuild(&mut self) {
        self.values.clear();
        self.picking = None;
    }

    fn add_condition(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if !self.collect_values(false, None, cx) {
            return;
        }
        if let Some(rules) = &mut self.rules {
            group_at(rules, &path).push(blank_condition());
        }
        self.rebuild();
        cx.notify();
    }

    fn add_group(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if !self.collect_values(false, None, cx) {
            return;
        }
        if let Some(rules) = &mut self.rules {
            group_at(rules, &path).push(Rule::All(vec![blank_condition()]));
        }
        self.rebuild();
        cx.notify();
    }

    /// Removes the rule a path names: a condition, a whole group, or an unknown expression.
    fn remove_rule(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if !self.collect_values(false, Some(&path), cx) {
            return;
        }
        if let Some(rules) = &mut self.rules
            && let Some(last) = path.last().copied()
        {
            group_at(rules, &path[..path.len() - 1]).remove(last);
        }
        self.rebuild();
        cx.notify();
    }

    /// Switches a group between matching all of its rules and matching any of them.
    fn flip_group(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if !self.collect_values(false, None, cx) {
            return;
        }
        if let Some(rules) = &mut self.rules {
            if path.is_empty() {
                rules.group = match std::mem::take(&mut rules.group) {
                    RuleGroup::All(inner) => RuleGroup::Any(inner),
                    RuleGroup::Any(inner) => RuleGroup::All(inner),
                };
            } else if let Some(last) = path.last().copied() {
                let parent = group_at(rules, &path[..path.len() - 1]);
                if let Some(rule) = parent.get_mut(last) {
                    *rule = match std::mem::replace(rule, Rule::All(Vec::new())) {
                        Rule::All(inner) => Rule::Any(inner),
                        Rule::Any(inner) => Rule::All(inner),
                        other => other,
                    };
                }
            }
        }
        self.rebuild();
        cx.notify();
    }

    fn set_field(&mut self, path: Vec<usize>, field: &str, cx: &mut Context<Self>) {
        if !self.collect_values(false, None, cx) {
            return;
        }
        if let Some(rules) = &mut self.rules
            && let Some(condition) = condition_at_mut(rules, &path)
        {
            condition.field = field.to_owned();
            // A boolean field tested with `is` takes the truth toggle's operand.
            if field_kind(field) == FieldKind::Boolean
                && matches!(condition.operator, Operator::Is | Operator::IsNot)
                && !matches!(condition.value, Operand::Boolean(_))
            {
                condition.value = Operand::Boolean(true);
            }
        }
        self.rebuild();
        cx.notify();
    }

    fn set_operator(&mut self, path: Vec<usize>, operator: Operator, cx: &mut Context<Self>) {
        if !self.collect_values(false, None, cx) {
            return;
        }
        if let Some(rules) = &mut self.rules
            && let Some(condition) = condition_at_mut(rules, &path)
        {
            let collected = condition.value.clone();
            let reshaped = operator.operand_shape() != condition.operator.operand_shape();
            condition.operator = operator;
            // A shape change starts the value over; within one shape the typed value stays.
            condition.value = match reshaped {
                false => collected,
                true => match operator.operand_shape() {
                    OperandShape::Truth => Operand::Boolean(true),
                    OperandShape::Range => Operand::Range(0.into(), 100.into()),
                    OperandShape::Days => Operand::Number(30.into()),
                    OperandShape::Playlist => Operand::Text(String::new()),
                    OperandShape::Compared => match field_kind(&condition.field) {
                        FieldKind::Boolean
                            if matches!(operator, Operator::Is | Operator::IsNot) =>
                        {
                            Operand::Boolean(true)
                        }
                        FieldKind::Number => Operand::Number(0.into()),
                        _ => Operand::Text(String::new()),
                    },
                },
            };
        }
        self.rebuild();
        cx.notify();
    }

    /// The truth toggle's click on a boolean operand.
    fn flip_truth(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if let Some(rules) = &mut self.rules
            && let Some(condition) = condition_at_mut(rules, &path)
            && let Operand::Boolean(truth) = &mut condition.value
        {
            *truth = !*truth;
        }
        cx.notify();
    }

    fn flip_order(&mut self, cx: &mut Context<Self>) {
        self.order_asc = match self.order_asc {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        let name = self.name.read(cx).text().trim().to_owned();
        if name.is_empty() {
            self.name
                .update(cx, |input, cx| input.set_invalid(true, cx));
            self.edit = Some(edit);
            return;
        }
        if !self.collect_values(true, None, cx) {
            self.edit = Some(edit);
            return;
        }
        let Some(rules) = self.rules.take() else {
            return;
        };
        let problems = rules.problems();
        if !problems.is_empty() {
            self.rules = Some(rules);
            self.edit = Some(edit);
            return;
        }
        let sort = self.sort.read(cx).text().trim().to_owned();
        let limit_text = self.limit.read(cx).text().trim().to_owned();
        let limit = match limit_text.is_empty() {
            true => None,
            false => match limit_text.parse::<u32>() {
                Ok(limit) => Some(limit),
                Err(_) => {
                    self.limit
                        .update(cx, |input, cx| input.set_invalid(true, cx));
                    self.rules = Some(rules);
                    self.edit = Some(edit);
                    return;
                }
            },
        };
        let mut rules = rules;
        rules.sort = (!sort.is_empty()).then_some(sort);
        rules.limit = limit;
        rules.order = self
            .order_asc
            .map(|asc| if asc { "asc" } else { "desc" }.to_owned());
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        let library = Sonora::global(cx).library.clone();
        match edit {
            SmartEdit::Create { shelf } => library.update(cx, |library, cx| {
                library.create_smart_playlist(name, rules, shelf, cx);
            }),
            SmartEdit::Rules { playlist } => {
                let renamed = (name != playlist.name).then_some(name);
                library.update(cx, |library, cx| {
                    library.save_smart_playlist(
                        playlist.id,
                        SmartPlaylistUpdate {
                            name: renamed,
                            rules: Some(rules),
                            ..SmartPlaylistUpdate::default()
                        },
                        cx,
                    );
                })
            }
        }
        cx.notify();
    }
}

/// What a row needs to render, lifted out of the tree so the inputs map can be borrowed
/// alongside it.
enum RowData {
    Group {
        path: Vec<usize>,
        key: String,
        any: bool,
        root: bool,
        empty: bool,
    },
    Condition {
        path: Vec<usize>,
        key: String,
        field: String,
        operator: Operator,
        low: String,
        high: String,
        truth: Option<bool>,
    },
    Unknown {
        path: Vec<usize>,
        key: String,
        text: String,
    },
}

impl Render for SmartPlaylistEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(edit) = self.edit.as_ref() else {
            return div().into_any_element();
        };
        let creating = matches!(edit, SmartEdit::Create { .. });
        let theme = *cx.theme();

        let body = div().flex().flex_col().flex_1().min_h_0().gap_3();
        let body = body.child(div().flex().child(self.name.clone()));

        let body = match (&self.loading, &self.failed, &self.rules) {
            (_, Some(problem), _) => body.child(
                Notice::new(t!("smart-rules-failed-title"), problem.clone())
                    .failed()
                    .icon("icons/info.svg"),
            ),
            (true, _, _) => body.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("smart-rules-loading")),
            ),
            (_, _, Some(_)) => {
                let rows = self.row_data();
                let mut list = div().flex().flex_col().gap_1();
                for row in rows {
                    list = list.child(self.row(row, cx));
                }
                let problems = self
                    .rules
                    .as_ref()
                    .map(|rules| rules.problems())
                    .unwrap_or_default();
                let mut column = div()
                    .id("smart-rules-scroll")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(list);
                if self.synced {
                    column = column.child(
                        Notice::new(
                            t!("smart-rules-synced-title"),
                            t!("smart-rules-synced-message"),
                        )
                        .icon("icons/info.svg"),
                    );
                }
                for problem in problems {
                    column = column.child(
                        div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(format!("{}: {}", problem.path, problem.problem)),
                    );
                }
                body.child(column).child(self.footer(cx))
            }
            _ => body,
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.apply(window, cx);
            }))
            .child(
                Modal::new(
                    "smart-playlist-editor",
                    match creating {
                        true => t!("smart-rules-create-title"),
                        false => t!("smart-rules-edit-title"),
                    },
                )
                .w(theme.metrics.cover * 4.2)
                .child(body)
                .action(
                    Button::new("cancel-smart-rules")
                        .ghost()
                        .label(t!("common-cancel"))
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                )
                .action(
                    Button::new("apply-smart-rules")
                        .primary()
                        .label(t!("common-save"))
                        .disabled(self.loading || self.failed.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                )
                .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

impl SmartPlaylistEditor {
    /// The tree lifted into render-ready rows.
    fn row_data(&self) -> Vec<RowData> {
        let Some(rules) = &self.rules else {
            return Vec::new();
        };
        flatten(rules)
            .into_iter()
            .filter_map(|row| match row {
                Row::Group { path, any, root } => {
                    let children: &[Rule] = match path.is_empty() {
                        true => rules.group.rules(),
                        false => {
                            let mut current = rules.group.rules();
                            for &at in &path {
                                match current.get(at) {
                                    Some(Rule::All(inner)) | Some(Rule::Any(inner)) => {
                                        current = inner
                                    }
                                    _ => return None,
                                }
                            }
                            current
                        }
                    };
                    Some(RowData::Group {
                        key: key_of(&path),
                        path,
                        any,
                        root,
                        empty: children.is_empty(),
                    })
                }
                Row::Condition { path } => {
                    let condition = condition_at(rules, &path)?;
                    let (low, high) = operand_texts(&condition.value);
                    let truth = match &condition.value {
                        Operand::Boolean(truth) => Some(*truth),
                        _ => None,
                    };
                    Some(RowData::Condition {
                        key: key_of(&path),
                        path,
                        field: condition.field.clone(),
                        operator: condition.operator,
                        low,
                        high,
                        truth,
                    })
                }
                Row::Unknown { path, text } => Some(RowData::Unknown {
                    key: key_of(&path),
                    path,
                    text,
                }),
            })
            .collect()
    }

    fn row(&mut self, row: RowData, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = *cx.theme();
        match row {
            RowData::Group {
                path,
                key,
                any,
                root,
                empty,
            } => {
                let indent = path.len();
                let mut header = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(indent as f32 * 18.));
                header = header.child(
                    Button::new(gpui::ElementId::Name(SharedString::from(format!(
                        "smart-group-kind-{key}"
                    ))))
                    .small()
                    .outline()
                    .label(match any {
                        true => t!("smart-rules-match-any"),
                        false => t!("smart-rules-match-all"),
                    })
                    .tooltip("smart-rules-match-toggle")
                    .on_click({
                        let path = path.clone();
                        cx.listener(move |this, _, _, cx| this.flip_group(path.clone(), cx))
                    }),
                );
                header = header.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(t!("smart-rules-of-following")),
                );
                header = header.child(div().flex_1());
                for (id, icon, label, add_group) in [
                    (
                        "smart-add-condition",
                        "icons/plus.svg",
                        t!("smart-rules-add-condition"),
                        false,
                    ),
                    (
                        "smart-add-group",
                        "icons/list-plus.svg",
                        t!("smart-rules-add-group"),
                        true,
                    ),
                ] {
                    let path = path.clone();
                    header = header.child(
                        Button::new(gpui::ElementId::Name(SharedString::from(format!(
                            "{id}-{key}"
                        ))))
                        .ghost()
                        .small()
                        .icon(icon)
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| match add_group {
                            true => this.add_group(path.clone(), cx),
                            false => this.add_condition(path.clone(), cx),
                        })),
                    );
                }
                if !root {
                    let path = path.clone();
                    header = header.child(
                        Button::new(gpui::ElementId::Name(SharedString::from(format!(
                            "smart-remove-group-{key}"
                        ))))
                        .ghost()
                        .small()
                        .icon("icons/x.svg")
                        .tooltip("smart-rules-remove-group")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_rule(path.clone(), cx)),
                        ),
                    );
                }
                let mut column = div().flex().flex_col().gap_1().child(header);
                if empty {
                    column = column.child(
                        div()
                            .pl(px((indent + 1) as f32 * 18.))
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t!("smart-rules-empty-group")),
                    );
                }
                column.into_any_element()
            }
            RowData::Condition {
                path,
                key,
                field,
                operator,
                low,
                high,
                truth,
            } => self.condition_row(path, key, field, operator, low, high, truth, cx),
            RowData::Unknown { path, key, text } => {
                let indent = path.len().saturating_sub(1);
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(indent as f32 * 18.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(text),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("smart-rules-unknown-note")),
                    );
                row = row.child(
                    Button::new(gpui::ElementId::Name(SharedString::from(format!(
                        "smart-remove-unknown-{key}"
                    ))))
                    .ghost()
                    .small()
                    .icon("icons/x.svg")
                    .tooltip("smart-rules-remove-condition")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.remove_rule(path.clone(), cx)),
                    ),
                );
                row.into_any_element()
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn condition_row(
        &mut self,
        path: Vec<usize>,
        key: String,
        field: String,
        operator: Operator,
        low: String,
        high: String,
        truth: Option<bool>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = *cx.theme();
        let indent = path.len().saturating_sub(1);
        let view = cx.entity().downgrade();

        // The field picker, or the button that opens it. Only one row's picker renders at a
        // time, so the two static keys never collide.
        let field_control = match &self.picking {
            Some(Pick::Field(open)) if *open == key => {
                let items = FIELDS.iter().map(|&name| {
                    let path = path.clone();
                    let view = view.clone();
                    MenuItem::new(
                        gpui::SharedString::from(format!("smart-field-{name}")),
                        name,
                    )
                    .checked(name == field)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| this.set_field(path.clone(), name, cx))
                            .ok();
                    })
                });
                Picker::new(FIELD_PICKER, &self.popovers, field.clone())
                    .name(t!("smart-rules-field"))
                    .ghost()
                    .w(Picker::REGULAR)
                    .items(items)
                    .into_any_element()
            }
            _ => Button::new(gpui::ElementId::Name(SharedString::from(format!(
                "smart-field-{key}"
            ))))
            .ghost()
            .small()
            .label(field.clone())
            .tooltip("smart-rules-field")
            .on_click({
                let key = key.clone();
                cx.listener(move |this, _, _, cx| {
                    this.picking = Some(Pick::Field(key.clone()));
                    this.popovers.open(FIELD_PICKER);
                    cx.notify();
                })
            })
            .into_any_element(),
        };

        let operator_control = match &self.picking {
            Some(Pick::Operator(open)) if *open == key => {
                let items = Operator::ALL.iter().map(|&option| {
                    let path = path.clone();
                    let view = view.clone();
                    MenuItem::new(
                        gpui::SharedString::from(format!("smart-operator-{}", option.wire_name())),
                        option.label(),
                    )
                    .checked(option == operator)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| this.set_operator(path.clone(), option, cx))
                            .ok();
                    })
                });
                Picker::new(OPERATOR_PICKER, &self.popovers, operator.label())
                    .name(t!("smart-rules-operator"))
                    .ghost()
                    .w(Picker::REGULAR)
                    .items(items)
                    .into_any_element()
            }
            _ => Button::new(gpui::ElementId::Name(SharedString::from(format!(
                "smart-operator-{key}"
            ))))
            .ghost()
            .small()
            .label(operator.label())
            .tooltip("smart-rules-operator")
            .on_click({
                let key = key.clone();
                cx.listener(move |this, _, _, cx| {
                    this.picking = Some(Pick::Operator(key.clone()));
                    this.popovers.open(OPERATOR_PICKER);
                    cx.notify();
                })
            })
            .into_any_element(),
        };

        // The operand: a truth toggle where the server reads a boolean, text fields
        // everywhere else, two of them for a range.
        let shape = operator.operand_shape();
        let boolean_compared = field_kind(&field) == FieldKind::Boolean
            && matches!(operator, Operator::Is | Operator::IsNot);
        let value_control = match (shape, boolean_compared, truth) {
            (OperandShape::Truth, _, Some(set)) | (OperandShape::Compared, true, Some(set)) => {
                Button::new(gpui::ElementId::Name(SharedString::from(format!(
                    "smart-truth-{key}"
                ))))
                .small()
                .outline()
                .label(match set {
                    true => t!("smart-rules-true"),
                    false => t!("smart-rules-false"),
                })
                .on_click({
                    let path = path.clone();
                    cx.listener(move |this, _, _, cx| this.flip_truth(path.clone(), cx))
                })
                .into_any_element()
            }
            _ => {
                let inputs = self.values.entry(key.clone()).or_insert_with(|| RowInputs {
                    low: cx.new(|cx| {
                        let mut input =
                            Input::new(t!("smart-rules-value-placeholder"), cx).compact();
                        input.set_text(low.clone(), cx);
                        input
                    }),
                    high: cx.new(|cx| {
                        let mut input =
                            Input::new(t!("smart-rules-value-placeholder"), cx).compact();
                        input.set_text(high.clone(), cx);
                        input
                    }),
                });
                let mut fields = div().flex().items_center().gap_1();
                fields = fields.child(div().w(px(140.)).child(inputs.low.clone()));
                if shape == OperandShape::Range {
                    fields = fields.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t!("smart-rules-range-and")),
                    );
                    fields = fields.child(div().w(px(140.)).child(inputs.high.clone()));
                }
                fields.into_any_element()
            }
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(indent as f32 * 18.))
            .child(field_control)
            .child(operator_control)
            .child(value_control)
            .child(div().flex_1())
            .child(
                Button::new(gpui::ElementId::Name(SharedString::from(format!(
                    "smart-remove-condition-{key}"
                ))))
                .ghost()
                .small()
                .icon("icons/x.svg")
                .tooltip("smart-rules-remove-condition")
                .on_click(cx.listener(move |this, _, _, cx| this.remove_rule(path.clone(), cx))),
            )
            .into_any_element()
    }

    /// The sort, direction and limit row under the tree.
    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("smart-rules-sort-label")),
            )
            .child(div().w(px(140.)).child(self.sort.clone()))
            .child(
                Button::new("smart-order")
                    .ghost()
                    .small()
                    .label(match self.order_asc {
                        None => t!("smart-rules-order-none"),
                        Some(true) => t!("smart-rules-order-asc"),
                        Some(false) => t!("smart-rules-order-desc"),
                    })
                    .tooltip("smart-rules-order-toggle")
                    .on_click(cx.listener(|this, _, _, cx| this.flip_order(cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("smart-rules-limit-label")),
            )
            .child(div().w(px(96.)).child(self.limit.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_keeps_raw_text_on_a_number_field() {
        let collected = collect_operand(
            OperandShape::Compared,
            true,
            false,
            &Operand::Number(5.into()),
            "abc",
            "",
            false,
        );
        assert_eq!(collected, Ok(Operand::Text("abc".to_owned())));
    }

    #[test]
    fn strict_rejects_text_on_a_number_field() {
        let collected = collect_operand(
            OperandShape::Compared,
            true,
            false,
            &Operand::Number(5.into()),
            "abc",
            "",
            true,
        );
        assert_eq!(collected, Err(()));
    }

    #[test]
    fn lenient_keeps_raw_days_text() {
        let collected = collect_operand(
            OperandShape::Days,
            false,
            false,
            &Operand::Number(30.into()),
            "soon",
            "",
            false,
        );
        assert_eq!(collected, Ok(Operand::Text("soon".to_owned())));
    }

    #[test]
    fn an_invalid_range_fails_even_lenient() {
        let collected = collect_operand(
            OperandShape::Range,
            true,
            false,
            &Operand::Range(0.into(), 100.into()),
            "10",
            "lots",
            false,
        );
        assert_eq!(collected, Err(()));
    }

    #[test]
    fn boolean_compared_keeps_the_toggle() {
        let collected = collect_operand(
            OperandShape::Compared,
            false,
            true,
            &Operand::Boolean(false),
            "ignored",
            "",
            false,
        );
        assert_eq!(collected, Ok(Operand::Boolean(false)));
    }

    #[test]
    fn numbers_parse_whole_or_decimal() {
        assert_eq!(parse_number("42"), Some(42.into()));
        assert_eq!(parse_number("4.5").and_then(|n| n.as_f64()), Some(4.5));
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("x"), None);
    }
}
