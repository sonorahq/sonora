//! Smart playlists: rule trees the provider evaluates into a track list on its own side,
//! instead of a list someone fills by hand.
//!
//! The wire shape follows Navidrome's smart-playlist criteria, the one provider that
//! answers the smart-playlist calls: a top-level `all` or `any` group holding conditions
//! such as `{"contains": {"title": "love"}}`, with a sort, an order, a limit and an offset
//! next to it. The model keeps every rule the server can send, including the ones Sonora
//! has no editor row for: an unknown expression parses into [`Rule::Unknown`] and writes
//! back verbatim, so editing the rest of a tree never discards part of it.

use serde_json::{Map, Number, Value};

/// What one playlist's rules look like on the provider, enough for an editor to open it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartPlaylist {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub public: bool,
    /// Who the provider says the playlist belongs to.
    pub owner: String,
    /// The signed-in account owns the playlist, so the provider takes its edits. Editing
    /// anyone else's is refused with a forbidden answer.
    pub owned: bool,
    /// The rules generating the track list, or `None` for an ordinary playlist: an editor
    /// opens one of those as a smart playlist only by adding rules on purpose.
    pub rules: Option<SmartRules>,
    /// The playlist syncs from an imported file, as an `.nsp` does, so the provider's next
    /// scan can overwrite an edit made here. An editor warns before saving one and never
    /// turns the sync off itself.
    pub synced: bool,
    pub track_count: u32,
}

/// The fields of a smart playlist a save changes. Only the `Some` fields go to the
/// provider, which leaves every field the request does not name untouched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartPlaylistUpdate {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    pub rules: Option<SmartRules>,
}

impl SmartPlaylistUpdate {
    /// Nothing to send.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.comment.is_none()
            && self.public.is_none()
            && self.rules.is_none()
    }

    /// The request body carrying exactly the fields set on the update.
    pub(crate) fn to_wire(&self) -> Value {
        let mut body = Map::new();
        if let Some(name) = &self.name {
            body.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(comment) = &self.comment {
            body.insert("comment".into(), Value::String(comment.clone()));
        }
        if let Some(public) = self.public {
            body.insert("public".into(), Value::Bool(public));
        }
        if let Some(rules) = &self.rules {
            body.insert("rules".into(), rules.to_wire());
        }
        Value::Object(body)
    }
}

/// A smart playlist's rules: the match group that picks the tracks, plus how the matches
/// are sorted and capped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartRules {
    /// The top-level group every track is matched against.
    pub group: RuleGroup,
    /// The field the matches sort by, such as `title`, `dateloved` or `random`.
    pub sort: Option<String>,
    /// `asc` or `desc`, kept as the server sent it.
    pub order: Option<String>,
    /// The most tracks the playlist holds. Wins over `limit_percent` when both are set.
    pub limit: Option<u32>,
    /// The share of the whole library the playlist holds, from 1 to 100, when no fixed
    /// `limit` applies.
    pub limit_percent: Option<u8>,
    /// How many of the first matches the playlist skips.
    pub offset: Option<u32>,
    /// How often the provider re-evaluates the rules, in the server's own duration spelling
    /// such as `1h30m`. `None` leaves the server's default in place.
    pub refresh_delay: Option<String>,
    /// Top-level keys a newer server sent that this model predates, written back as they
    /// came so a save loses nothing it never showed.
    pub extra: Map<String, Value>,
}

impl SmartRules {
    /// Rules matching every condition in one group, the shape a fresh editor starts from.
    pub fn new(group: RuleGroup) -> Self {
        Self {
            group,
            ..Self::default()
        }
    }

    /// Reads the criteria JSON the provider answers with. The top level must hold exactly
    /// one of `all` and `any`, the way the server reads it; anything else is an error, since
    /// guessing which group to keep would silently drop rules.
    pub fn from_wire(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "the rules are not a json object".to_owned())?;
        let has_all = object.contains_key("all");
        let has_any = object.contains_key("any");
        let group = match (has_all, has_any) {
            (true, true) => {
                return Err(
                    "the rules use both 'all' and 'any' at the top level; one must nest inside the other"
                        .to_owned(),
                );
            }
            (true, false) => RuleGroup::All(rules_from_wire(&object["all"])?),
            (false, true) => RuleGroup::Any(rules_from_wire(&object["any"])?),
            (false, false) => {
                return Err("the rules hold no 'all' or 'any' group".to_owned());
            }
        };
        let mut rules = SmartRules {
            group,
            ..Self::default()
        };
        let mut extra = Map::new();
        for (key, value) in object {
            match key.as_str() {
                "all" | "any" => {}
                "sort" => {
                    rules.sort = value
                        .as_str()
                        .filter(|sort| !sort.is_empty())
                        .map(str::to_owned)
                }
                "order" => {
                    rules.order = value
                        .as_str()
                        .filter(|order| !order.is_empty())
                        .map(str::to_owned)
                }
                "limit" => rules.limit = count(value),
                "limitPercent" => {
                    // The server clamps the share to 0..=100 as it reads it; a clamped 0 is
                    // no limit at all.
                    rules.limit_percent = count(value)
                        .map(|share| share.min(100) as u8)
                        .filter(|share| *share > 0)
                }
                "offset" => rules.offset = count(value),
                "refreshDelay" => {
                    rules.refresh_delay = value
                        .as_str()
                        .filter(|delay| !delay.is_empty())
                        .map(str::to_owned)
                }
                _ => {
                    extra.insert(key.clone(), value.clone());
                }
            }
        }
        rules.extra = extra;
        Ok(rules)
    }

    /// The criteria JSON the provider reads back, byte-compatible with what it answered:
    /// known keys take their typed values, unknown keys and rules come back as they came.
    pub fn to_wire(&self) -> Value {
        let mut object = Map::new();
        let (key, rules) = match &self.group {
            RuleGroup::All(rules) => ("all", rules),
            RuleGroup::Any(rules) => ("any", rules),
        };
        object.insert(
            key.to_owned(),
            Value::Array(rules.iter().map(Rule::to_wire).collect()),
        );
        if let Some(sort) = &self.sort {
            object.insert("sort".into(), Value::String(sort.clone()));
        }
        if let Some(order) = &self.order {
            object.insert("order".into(), Value::String(order.clone()));
        }
        if let Some(limit) = self.limit.filter(|limit| *limit > 0) {
            object.insert("limit".into(), Value::Number(limit.into()));
        }
        if let Some(share) = self.limit_percent.filter(|share| *share > 0) {
            object.insert("limitPercent".into(), Value::Number(share.into()));
        }
        if let Some(offset) = self.offset.filter(|offset| *offset > 0) {
            object.insert("offset".into(), Value::Number(offset.into()));
        }
        if let Some(delay) = &self.refresh_delay {
            object.insert("refreshDelay".into(), Value::String(delay.clone()));
        }
        for (key, value) in &self.extra {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Value::Object(object)
    }

    /// The ways the rules break, each naming where in the tree it sits. An empty list is a
    /// ruleset an editor may save; anything else would fail on the provider, which reads the
    /// same rules when it builds the track query.
    pub fn problems(&self) -> Vec<RuleProblem> {
        let mut problems = Vec::new();
        let (path, rules) = match &self.group {
            RuleGroup::All(rules) => ("all", rules),
            RuleGroup::Any(rules) => ("any", rules),
        };
        if rules.is_empty() {
            problems.push(RuleProblem {
                path: path.to_owned(),
                problem: Problem::EmptyGroup,
            });
        }
        for (index, rule) in rules.iter().enumerate() {
            rule.problems(&format!("{path}[{index}]"), &mut problems);
        }
        if let Some(order) = &self.order
            && !matches!(order.to_ascii_lowercase().as_str(), "asc" | "desc")
        {
            problems.push(RuleProblem {
                path: "order".to_owned(),
                problem: Problem::BadOrder,
            });
        }
        problems
    }
}

/// The top-level group of a ruleset: match every rule, or match any of them.
#[derive(Clone, Debug, PartialEq)]
pub enum RuleGroup {
    All(Vec<Rule>),
    Any(Vec<Rule>),
}

impl Default for RuleGroup {
    fn default() -> Self {
        Self::All(Vec::new())
    }
}

impl RuleGroup {
    pub fn rules(&self) -> &[Rule] {
        match self {
            Self::All(rules) | Self::Any(rules) => rules,
        }
    }

    pub fn rules_mut(&mut self) -> &mut Vec<Rule> {
        match self {
            Self::All(rules) | Self::Any(rules) => rules,
        }
    }

    /// The group holds no rule yet.
    pub fn is_empty(&self) -> bool {
        self.rules().is_empty()
    }
}

/// One rule in a tree: a nested group, a condition on a field, or an expression Sonora has
/// no row for.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Condition(Condition),
    /// An expression whose operator or value shape Sonora does not know, kept as it came
    /// and written back unchanged. An editor renders it read-only rather than dropping it.
    Unknown(Value),
}

impl Rule {
    fn from_wire_key(key: &str, value: &Value) -> Result<Self, String> {
        let lowered = key.to_ascii_lowercase();
        match lowered.as_str() {
            "all" => return Ok(Self::All(rules_from_wire(value)?)),
            "any" => return Ok(Self::Any(rules_from_wire(value)?)),
            _ => {}
        }
        let Some(operator) = Operator::from_wire_name(&lowered) else {
            return Ok(Self::Unknown(Value::Object(Map::from_iter([(
                key.to_owned(),
                value.clone(),
            )]))));
        };
        let Some(fields) = value.as_object() else {
            return Ok(Self::Unknown(Value::Object(Map::from_iter([(
                key.to_owned(),
                value.clone(),
            )]))));
        };
        // The server reads every entry of the value map as one condition on that field, so
        // a value naming two fields means two conditions. Sonora's model holds one field per
        // condition; a map with more than one entry stays whole and read-only instead of
        // being split or dropped.
        if fields.len() != 1 {
            return Ok(Self::Unknown(Value::Object(Map::from_iter([(
                key.to_owned(),
                value.clone(),
            )]))));
        }
        let (field, operand) = fields.iter().next().expect("one entry checked");
        match Operand::from_wire(operator, field, operand) {
            Some(value) => Ok(Self::Condition(Condition {
                operator,
                field: field.clone(),
                value,
            })),
            None => Ok(Self::Unknown(Value::Object(Map::from_iter([(
                key.to_owned(),
                value.clone(),
            )])))),
        }
    }

    fn to_wire(&self) -> Value {
        match self {
            Self::All(rules) => Value::Object(Map::from_iter([(
                "all".to_owned(),
                Value::Array(rules.iter().map(Rule::to_wire).collect()),
            )])),
            Self::Any(rules) => Value::Object(Map::from_iter([(
                "any".to_owned(),
                Value::Array(rules.iter().map(Rule::to_wire).collect()),
            )])),
            Self::Condition(condition) => condition.to_wire(),
            Self::Unknown(raw) => raw.clone(),
        }
    }

    fn problems(&self, path: &str, problems: &mut Vec<RuleProblem>) {
        match self {
            Self::All(rules) | Self::Any(rules) => {
                if rules.is_empty() {
                    problems.push(RuleProblem {
                        path: path.to_owned(),
                        problem: Problem::EmptyGroup,
                    });
                }
                for (index, rule) in rules.iter().enumerate() {
                    rule.problems(&format!("{path}[{index}]"), problems);
                }
            }
            Self::Condition(condition) => condition.problems(path, problems),
            Self::Unknown(_) => {}
        }
    }

    /// A condition that passes every check, or `None` for groups and unknown rules.
    pub fn condition(&self) -> Option<&Condition> {
        match self {
            Self::Condition(condition) => Some(condition),
            _ => None,
        }
    }

    pub fn condition_mut(&mut self) -> Option<&mut Condition> {
        match self {
            Self::Condition(condition) => Some(condition),
            _ => None,
        }
    }
}

/// One test on one field, such as `title contains "love"` or `playcount is greater than 10`.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub operator: Operator,
    /// The field under test. The server knows its own columns plus any tag a scan has seen,
    /// so a name this model does not list is still allowed through.
    pub field: String,
    pub value: Operand,
}

impl Condition {
    fn to_wire(&self) -> Value {
        Value::Object(Map::from_iter([(
            self.operator.wire_name().to_owned(),
            Value::Object(Map::from_iter([(self.field.clone(), self.value.to_wire())])),
        )]))
    }

    fn problems(&self, path: &str, problems: &mut Vec<RuleProblem>) {
        if self.field.trim().is_empty() {
            problems.push(RuleProblem {
                path: path.to_owned(),
                problem: Problem::MissingField,
            });
        }
        let shape = self.operator.operand_shape();
        let fits = match (shape, &self.value) {
            (OperandShape::Range, Operand::Range(_, _)) => true,
            (OperandShape::Range, _) => false,
            (OperandShape::Days, Operand::Number(_)) => true,
            (OperandShape::Days, _) => false,
            (OperandShape::Playlist, Operand::Text(text)) => !text.is_empty(),
            (OperandShape::Playlist, _) => false,
            (OperandShape::Truth, Operand::Boolean(_)) => true,
            (OperandShape::Truth, _) => false,
            (OperandShape::Compared, _) => true,
        };
        if !fits {
            problems.push(RuleProblem {
                path: path.to_owned(),
                problem: Problem::WrongOperand(shape),
            });
            return;
        }
        // The server coerces boolean-looking values on boolean fields as it reads them, so a
        // boolean field tested with `is` takes only a truth value.
        if field_kind(&self.field) == FieldKind::Boolean
            && matches!(self.operator, Operator::Is | Operator::IsNot)
            && !matches!(self.value, Operand::Boolean(_))
        {
            problems.push(RuleProblem {
                path: path.to_owned(),
                problem: Problem::BooleanField,
            });
        }
    }
}

/// The test a condition runs on its field. The names are the server's own, so a ruleset
/// spelled for another client reads the same here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Is,
    IsNot,
    GreaterThan,
    LessThan,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    InTheRange,
    Before,
    After,
    InTheLast,
    NotInTheLast,
    InPlaylist,
    NotInPlaylist,
    IsMissing,
    IsPresent,
}

/// The operand shape an operator takes, which decides what an editor offers for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandShape {
    /// A single text, number or boolean the field is compared against.
    Compared,
    /// A lower and an upper bound, for `inTheRange`.
    Range,
    /// A count of days, for `inTheLast` and `notInTheLast`.
    Days,
    /// The id or path of another playlist, for `inPlaylist` and `notInPlaylist`.
    Playlist,
    /// A truth value, for `isMissing` and `isPresent`.
    Truth,
}

impl Operator {
    /// Every operator, in the order an editor offers them.
    pub const ALL: &'static [Operator] = &[
        Operator::Is,
        Operator::IsNot,
        Operator::GreaterThan,
        Operator::LessThan,
        Operator::Contains,
        Operator::NotContains,
        Operator::StartsWith,
        Operator::EndsWith,
        Operator::InTheRange,
        Operator::Before,
        Operator::After,
        Operator::InTheLast,
        Operator::NotInTheLast,
        Operator::InPlaylist,
        Operator::NotInPlaylist,
        Operator::IsMissing,
        Operator::IsPresent,
    ];

    /// The key the server writes for the operator.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::IsNot => "isNot",
            Self::GreaterThan => "gt",
            Self::LessThan => "lt",
            Self::Contains => "contains",
            Self::NotContains => "notContains",
            Self::StartsWith => "startsWith",
            Self::EndsWith => "endsWith",
            Self::InTheRange => "inTheRange",
            Self::Before => "before",
            Self::After => "after",
            Self::InTheLast => "inTheLast",
            Self::NotInTheLast => "notInTheLast",
            Self::InPlaylist => "inPlaylist",
            Self::NotInPlaylist => "notInPlaylist",
            Self::IsMissing => "isMissing",
            Self::IsPresent => "isPresent",
        }
    }

    /// Reads an operator key the way the server does, which lowercases it first.
    fn from_wire_name(lowered: &str) -> Option<Self> {
        Some(match lowered {
            "is" => Self::Is,
            "isnot" => Self::IsNot,
            "gt" => Self::GreaterThan,
            "lt" => Self::LessThan,
            "contains" => Self::Contains,
            "notcontains" => Self::NotContains,
            "startswith" => Self::StartsWith,
            "endswith" => Self::EndsWith,
            "intherange" => Self::InTheRange,
            "before" => Self::Before,
            "after" => Self::After,
            "inthelast" => Self::InTheLast,
            "notinthelast" => Self::NotInTheLast,
            "inplaylist" => Self::InPlaylist,
            "notinplaylist" => Self::NotInPlaylist,
            "ismissing" => Self::IsMissing,
            "ispresent" => Self::IsPresent,
            _ => return None,
        })
    }

    /// What the operator's value looks like.
    pub fn operand_shape(self) -> OperandShape {
        match self {
            Self::InTheRange => OperandShape::Range,
            Self::InTheLast | Self::NotInTheLast => OperandShape::Days,
            Self::InPlaylist | Self::NotInPlaylist => OperandShape::Playlist,
            Self::IsMissing | Self::IsPresent => OperandShape::Truth,
            _ => OperandShape::Compared,
        }
    }

    /// How the operator reads in a sentence, for menus and read-only rows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::IsNot => "is not",
            Self::GreaterThan => "is greater than",
            Self::LessThan => "is less than",
            Self::Contains => "contains",
            Self::NotContains => "does not contain",
            Self::StartsWith => "starts with",
            Self::EndsWith => "ends with",
            Self::InTheRange => "is in the range",
            Self::Before => "is before",
            Self::After => "is after",
            Self::InTheLast => "is in the last",
            Self::NotInTheLast => "is not in the last",
            Self::InPlaylist => "is in the playlist",
            Self::NotInPlaylist => "is not in the playlist",
            Self::IsMissing => "has no",
            Self::IsPresent => "has a",
        }
    }
}

/// A condition's value, typed the way the server reads it: a boolean is a boolean and a
/// number a number, never a string spelled like one.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Text(String),
    Number(Number),
    Boolean(bool),
    /// `inTheRange`'s lower and upper bounds.
    Range(Number, Number),
}

impl Operand {
    /// Reads the server's value into the typed operand the operator takes, coercing the
    /// boolean spellings (`true`, `"true"`, `1`) the server itself coerces on boolean
    /// fields. `None` marks a value whose shape fits no operand, which keeps the whole
    /// condition read-only rather than losing it.
    fn from_wire(operator: Operator, field: &str, value: &Value) -> Option<Self> {
        match operator.operand_shape() {
            OperandShape::Range => {
                let pair = value.as_array()?;
                let (low, high) = (pair.first()?.as_number()?, pair.get(1)?.as_number()?);
                Some(Self::Range(low.clone(), high.clone()))
            }
            OperandShape::Days => Some(Self::Number(value.as_number()?.clone())),
            OperandShape::Playlist => Some(Self::Text(value.as_str()?.to_owned())),
            OperandShape::Truth => Some(Self::Boolean(to_bool(value)?)),
            OperandShape::Compared => {
                if field_kind(field) == FieldKind::Boolean
                    && let Some(truth) = to_bool(value)
                {
                    return Some(Self::Boolean(truth));
                }
                match value {
                    Value::String(text) => Some(Self::Text(text.clone())),
                    Value::Number(number) => Some(Self::Number(number.clone())),
                    Value::Bool(truth) => Some(Self::Boolean(*truth)),
                    _ => None,
                }
            }
        }
    }

    fn to_wire(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Number(number) => Value::Number(number.clone()),
            Self::Boolean(truth) => Value::Bool(*truth),
            Self::Range(low, high) => Value::Array(vec![
                Value::Number(low.clone()),
                Value::Number(high.clone()),
            ]),
        }
    }

    /// The value as it reads on a read-only row.
    pub fn describe(&self) -> String {
        match self {
            Self::Text(text) => format!("\"{text}\""),
            Self::Number(number) => number.to_string(),
            Self::Boolean(truth) => truth.to_string(),
            Self::Range(low, high) => format!("{low} and {high}"),
        }
    }
}

/// How a field's value reads, which decides what an editor offers for it and how the server
/// coerces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
}

/// The kind of the fields Navidrome v0.64.2 lists in model/criteria/fields.go, with its own
/// numeric and boolean flags. A name it does not list is a tag only a scan knows, which
/// reads as text here; the server still takes a rule on it.
pub fn field_kind(field: &str) -> FieldKind {
    match field.to_ascii_lowercase().as_str() {
        "hascoverart" | "compilation" | "missing" | "loved" | "albumloved" | "artistloved" => {
            FieldKind::Boolean
        }
        "bitdepth"
        | "bpm"
        | "averagerating"
        | "rgalbumgain"
        | "rgalbumpeak"
        | "rgtrackgain"
        | "rgtrackpeak"
        | "library_id"
        | "replaygain_album_gain"
        | "replaygain_album_peak"
        | "replaygain_track_gain"
        | "replaygain_track_peak" => FieldKind::Number,
        _ => FieldKind::Text,
    }
}

/// The number an editor's text field spells: whole numbers stay whole, and anything else
/// parses as a decimal. `None` marks text that is no number, which the field reports.
pub fn parse_number(text: &str) -> Option<Number> {
    let trimmed = text.trim();
    match trimmed.parse::<i64>() {
        Ok(whole) => Some(Number::from(whole)),
        Err(_) => trimmed.parse::<f64>().ok().and_then(Number::from_f64),
    }
}

/// The boolean a criteria value spells, coerced exactly the ways the server coerces one: a
/// real boolean, a parseable string, or exactly the number 0 or 1.
fn to_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(truth) => Some(*truth),
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" => Some(true),
            "false" | "f" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) => match number.as_f64() {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A way the rules break, with the path of the rule that breaks them, such as
/// `all[2].any[0]`, so an editor can mark the row instead of only naming the mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleProblem {
    pub path: String,
    pub problem: Problem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A group with nothing in it, which the server reads as missing rules.
    EmptyGroup,
    /// A condition with no field to test.
    MissingField,
    /// The operand's shape is not the one the operator takes.
    WrongOperand(OperandShape),
    /// A boolean field tested with `is` against a value that is not a truth.
    BooleanField,
    /// An order that is neither `asc` nor `desc`.
    BadOrder,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EmptyGroup => "the group holds no rules",
            Self::MissingField => "the condition names no field",
            Self::WrongOperand(OperandShape::Range) => "a range takes a lower and an upper bound",
            Self::WrongOperand(OperandShape::Days) => "a day count takes a number",
            Self::WrongOperand(OperandShape::Playlist) => "a playlist rule takes a playlist",
            Self::WrongOperand(OperandShape::Truth) => "the rule takes a truth value",
            Self::WrongOperand(OperandShape::Compared) => "the value does not fit the rule",
            Self::BooleanField => "a boolean field takes a truth value",
            Self::BadOrder => "the order is asc or desc",
        })
    }
}

/// A non-negative whole number out of a value that should hold one, `None` for anything
/// else, which the server's own zero-omitting write never produces.
fn count(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
        .filter(|count| *count > 0)
}

/// Reads a group's rule array. Every element is an object whose keys are operators or
/// nested groups; the server reads a multi-keyed object as one rule per key, and so does
/// this, writing each rule back as its own element.
fn rules_from_wire(value: &Value) -> Result<Vec<Rule>, String> {
    let array = value
        .as_array()
        .ok_or_else(|| "a rule group is not a json array".to_owned())?;
    let mut rules = Vec::new();
    for element in array {
        let Some(object) = element.as_object() else {
            rules.push(Rule::Unknown(element.clone()));
            continue;
        };
        for (key, value) in object {
            rules.push(Rule::from_wire_key(key, value)?);
        }
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn roundtrip(value: Value) -> Value {
        SmartRules::from_wire(&value).unwrap().to_wire()
    }

    #[test]
    fn a_documented_ruleset_round_trips() {
        // The example from Navidrome's smart-playlist documentation.
        let wire = json!({
            "all": [
                {"contains": {"title": "love"}},
                {"is": {"loved": true}}
            ],
            "sort": "dateloved",
            "order": "desc",
            "limit": 100
        });
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn nested_groups_round_trip() {
        let wire = json!({
            "any": [
                {"all": [
                    {"gt": {"playcount": 10}},
                    {"is": {"genre": "jazz"}}
                ]},
                {"inTheRange": {"year": [1980, 1989]}}
            ],
            "offset": 20
        });
        let rules = SmartRules::from_wire(&wire).unwrap();
        let RuleGroup::Any(top) = &rules.group else {
            panic!("the top group reads as any");
        };
        assert_eq!(top.len(), 2);
        assert_eq!(rules.offset, Some(20));
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn operator_keys_read_without_case_and_write_canonical() {
        let rules = SmartRules::from_wire(&json!({
            "all": [{"ISNOT": {"artist": "beatles"}}]
        }))
        .unwrap();
        let condition = rules.group.rules()[0].condition().unwrap();
        assert_eq!(condition.operator, Operator::IsNot);
        assert_eq!(
            rules.to_wire(),
            json!({"all": [{"isNot": {"artist": "beatles"}}]})
        );
    }

    #[test]
    fn boolean_values_read_the_ways_the_server_coerces_them() {
        let rules = SmartRules::from_wire(&json!({
            "all": [
                {"is": {"loved": "true"}},
                {"is": {"compilation": 0}},
                {"isMissing": {"comment": "FALSE"}}
            ]
        }))
        .unwrap();
        let operands: Vec<&Operand> = rules
            .group
            .rules()
            .iter()
            .filter_map(|rule| rule.condition().map(|condition| &condition.value))
            .collect();
        assert_eq!(
            operands,
            [
                &Operand::Boolean(true),
                &Operand::Boolean(false),
                &Operand::Boolean(false)
            ]
        );
        assert_eq!(
            rules.to_wire(),
            json!({"all": [
                {"is": {"loved": true}},
                {"is": {"compilation": false}},
                {"isMissing": {"comment": false}}
            ]})
        );
    }

    #[test]
    fn an_unknown_operator_is_kept_verbatim() {
        let wire = json!({
            "all": [
                {"isRelated": {"artist": "bowie"}},
                {"is": {"title": "heroes"}}
            ]
        });
        let rules = SmartRules::from_wire(&wire).unwrap();
        assert!(matches!(rules.group.rules()[0], Rule::Unknown(_)));
        assert!(rules.group.rules()[1].condition().is_some());
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn a_condition_with_two_fields_stays_whole() {
        let wire = json!({
            "all": [{"is": {"title": "a", "artist": "b"}}]
        });
        let rules = SmartRules::from_wire(&wire).unwrap();
        assert!(matches!(rules.group.rules()[0], Rule::Unknown(_)));
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn both_top_level_groups_are_refused() {
        let error = SmartRules::from_wire(&json!({"all": [], "any": []})).unwrap_err();
        assert!(error.contains("both"));
        let error = SmartRules::from_wire(&json!({"sort": "title"})).unwrap_err();
        assert!(error.contains("no 'all' or 'any'"));
    }

    #[test]
    fn a_range_reads_and_writes_as_two_numbers() {
        let rules = SmartRules::from_wire(&json!({
            "all": [{"inTheRange": {"year": [1980, 1989.5]}}]
        }))
        .unwrap();
        let condition = rules.group.rules()[0].condition().unwrap();
        assert_eq!(
            condition.value,
            Operand::Range(Number::from(1980), Number::from_f64(1989.5).unwrap())
        );
        assert_eq!(condition.value.describe(), "1980 and 1989.5");
    }

    #[test]
    fn a_misshapen_range_stays_verbatim() {
        let wire = json!({"all": [{"inTheRange": {"year": "eighties"}}]});
        let rules = SmartRules::from_wire(&wire).unwrap();
        assert!(matches!(rules.group.rules()[0], Rule::Unknown(_)));
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn limits_read_the_way_the_server_clamps_and_omits_them() {
        let rules = SmartRules::from_wire(&json!({
            "all": [{"is": {"loved": true}}],
            "limit": 0,
            "limitPercent": 150
        }))
        .unwrap();
        assert_eq!(rules.limit, None);
        assert_eq!(rules.limit_percent, Some(100));
        assert_eq!(
            rules.to_wire(),
            json!({"all": [{"is": {"loved": true}}], "limitPercent": 100})
        );
    }

    #[test]
    fn unknown_top_level_keys_write_back() {
        let wire = json!({
            "all": [{"is": {"loved": true}}],
            "sort": "title",
            "futureOption": {"nested": true}
        });
        assert_eq!(roundtrip(wire.clone()), wire);
    }

    #[test]
    fn a_typed_sort_wins_over_a_kept_unknown_spelling() {
        let mut rules = SmartRules::from_wire(&json!({
            "all": [{"is": {"loved": true}}],
            "sort": "title"
        }))
        .unwrap();
        rules
            .extra
            .insert("sort".into(), json!("a stale duplicate"));
        assert_eq!(
            rules.to_wire(),
            json!({"all": [{"is": {"loved": true}}], "sort": "title"})
        );
    }

    #[test]
    fn problems_mark_the_breaking_rule() {
        let mut rules = SmartRules::new(RuleGroup::All(vec![
            Rule::Condition(Condition {
                operator: Operator::Is,
                field: String::new(),
                value: Operand::Text("x".into()),
            }),
            Rule::Any(Vec::new()),
            Rule::Condition(Condition {
                operator: Operator::InTheRange,
                field: "year".into(),
                value: Operand::Text("eighties".into()),
            }),
            Rule::Condition(Condition {
                operator: Operator::Is,
                field: "loved".into(),
                value: Operand::Text("a lot".into()),
            }),
        ]));
        rules.order = Some("sideways".into());
        let problems = rules.problems();
        let kinds: Vec<(&str, Problem)> = problems
            .iter()
            .map(|problem| (problem.path.as_str(), problem.problem))
            .collect();
        assert_eq!(
            kinds,
            [
                ("all[0]", Problem::MissingField),
                ("all[1]", Problem::EmptyGroup),
                ("all[2]", Problem::WrongOperand(OperandShape::Range)),
                ("all[3]", Problem::BooleanField),
                ("order", Problem::BadOrder),
            ]
        );
    }

    #[test]
    fn sound_rules_report_no_problems() {
        let rules = SmartRules::from_wire(&json!({
            "all": [
                {"contains": {"title": "love"}},
                {"gt": {"playcount": 10}},
                {"inTheLast": {"lastplayed": 30}},
                {"inPlaylist": {"id": "some-playlist-id"}},
                {"isMissing": {"comment": true}}
            ],
            "sort": "random",
            "order": "asc",
            "limit": 25,
            "refreshDelay": "1h30m"
        }))
        .unwrap();
        assert_eq!(rules.problems(), Vec::new());
        assert_eq!(rules.refresh_delay.as_deref(), Some("1h30m"));
    }

    #[test]
    fn an_update_sends_only_the_fields_it_names() {
        let update = SmartPlaylistUpdate::default();
        assert!(update.is_empty());
        assert_eq!(update.to_wire(), json!({}));

        let update = SmartPlaylistUpdate {
            name: Some("quiet".into()),
            rules: Some(SmartRules::new(RuleGroup::All(vec![Rule::Condition(
                Condition {
                    operator: Operator::Is,
                    field: "loved".into(),
                    value: Operand::Boolean(true),
                },
            )]))),
            ..SmartPlaylistUpdate::default()
        };
        assert_eq!(
            update.to_wire(),
            json!({
                "name": "quiet",
                "rules": {"all": [{"is": {"loved": true}}]}
            })
        );
    }

    #[test]
    fn field_kinds_follow_the_server_list() {
        assert_eq!(field_kind("loved"), FieldKind::Boolean);
        assert_eq!(field_kind("HASCoverArt"), FieldKind::Boolean);
        assert_eq!(field_kind("bpm"), FieldKind::Number);
        assert_eq!(field_kind("title"), FieldKind::Text);
        assert_eq!(field_kind("some-custom-tag"), FieldKind::Text);
    }
}
