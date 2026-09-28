//! Reading the components out of a Vector configuration.
//!
//! A Vector config is four maps of named components — `sources`, `transforms`,
//! `sinks` and `enrichment_tables` — and every transform, sink and table
//! carries `inputs`, the IDs of the components feeding it. That is the whole
//! graph, stated outright, which is why it can be read rather than inferred.
//!
//! Both formats land in the same [`Component`] list, so everything downstream
//! is written once and neither knows nor cares which it came from. Each parser
//! only converts a component's body into one [`Value`] tree; what its fields
//! *mean* is [`crate::outputs`]'s, asked through the one [`Fields`]
//! implementation that tree has, so the two formats cannot drift apart.
//!
//! Before either parser sees a file, its environment variables are
//! interpolated, because that is what Vector does ([`crate::vars`]).
//!
//! Positions are kept from the start. A graph that cannot take you to the
//! component you clicked is half a feature, and spans cannot be retrofitted
//! through a serde round trip — which is why neither parser here is the serde
//! one.

use std::collections::HashMap;

use editor_text::{LineIndex, Range};
use saphyr::{LoadableYamlNode, MarkedYaml};

use crate::outputs::{self, Fields};
use crate::vars::{self, Interpolated, Syntax};

/// Which of the four maps a component came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Source,
    Transform,
    Sink,
    /// An `enrichment_tables` entry. Vector compiles one into a sink, and a
    /// `memory` table with a `source_config` into a source as well, under the
    /// separate name its `source_key` gives. Both halves are read as this
    /// role; which one a component is shows in whether it takes inputs or
    /// offers outputs.
    Table,
}

impl Role {
    /// The key this role lives under in a config.
    const fn section(self) -> &'static str {
        match self {
            Self::Source => "sources",
            Self::Transform => "transforms",
            Self::Sink => "sinks",
            Self::Table => ENRICHMENT_TABLES,
        }
    }

    const ALL: [Self; 4] = [Self::Source, Self::Transform, Self::Sink, Self::Table];

    /// Whether Vector requires this role to declare `inputs`.
    ///
    /// `check_shape` reports "has no inputs" for a transform or a sink, and
    /// only for those two: it never looks at the enrichment tables, so a table
    /// nothing writes into is perfectly legal — it is loaded from a file, or
    /// filled by VRL.
    pub(crate) const fn needs_inputs(self) -> bool {
        matches!(self, Self::Transform | Self::Sink)
    }
}

/// One entry of a component's `inputs`, as written.
///
/// Kept verbatim — wildcard, dotted output and all — because resolving it is a
/// separate job, and because what somebody typed is what has to be underlined
/// when it resolves to nothing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub text: String,
    pub range: Range,
    /// The file `range` is in. Usually the component's, but not always: a
    /// component split across files takes the `inputs` of every piece. See
    /// [`assemble`].
    pub file: usize,
}

/// A source, transform, sink or enrichment table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: String,
    pub role: Role,
    /// The `type` field: `file`, `remap`, `route`, `console`, `memory`.
    ///
    /// Empty when the config omits it. Vector would reject that, but it is no
    /// reason to refuse to draw what is there. It is also what decides the
    /// outputs, so a component without one gets the plain default output.
    #[serde(rename = "type")]
    pub component_type: String,
    pub inputs: Vec<Input>,
    /// Where the component is declared, for going to it from the graph.
    pub range: Range,
    /// The outputs this component offers besides its default one. See
    /// [`crate::outputs`].
    pub named_outputs: Vec<String>,
    /// Whether an input can name the component itself. `false` for a router,
    /// for an `opentelemetry` source, and for anything events only end at.
    /// See [`crate::outputs`].
    pub default_output: bool,
    /// Which of the files being read declares it, as an index into the list
    /// the caller gave. `range` is in that file.
    /// Always 0 when a single file is read.
    pub file: usize,
}

impl Component {
    /// Whether anything can read this component at all.
    #[must_use]
    pub fn has_outputs(&self) -> bool {
        self.default_output || !self.named_outputs.is_empty()
    }

    /// Every output an input can name, as Vector writes it: the bare ID for
    /// the default output, `id.port` for each named one.
    pub(crate) fn outputs(&self) -> impl Iterator<Item = (Option<&String>, String)> {
        let default = self.default_output.then(|| (None, self.id.clone()));
        let named = self
            .named_outputs
            .iter()
            .map(move |output| (Some(output), format!("{}.{output}", self.id)));
        default.into_iter().chain(named)
    }
}

/// A config file, read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub components: Vec<Component>,
    /// The global `wildcard_matching: relaxed`, which makes an input pattern
    /// matching nothing legal instead of fatal. See [`crate::graph`].
    pub relaxed_wildcards: bool,
}

/// Why a config could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub message: String,
    /// Absent when the parser could not say where the trouble was.
    pub range: Option<Range>,
}

/// The top-level key Vector reads enrichment tables from.
const ENRICHMENT_TABLES: &str = "enrichment_tables";

/// The global key that relaxes wildcard matching, and the value that does it.
const WILDCARD_MATCHING: &str = "wildcard_matching";
const RELAXED: &str = "relaxed";

/// One component as one file writes it, before the files are put together.
///
/// A component is not always written in one place. Vector reads the files at
/// the top of a `--config-dir` directory as one value, merging them key by key
/// (`load_from_dir` and `merge_into_map`, `src/config/loading/`), so a file can
/// add `[[transforms.split.routes]]` to a router another file declares. What a
/// component is can only be said once every piece of it is in; [`assemble`]
/// is where that happens.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    role: Role,
    id: String,
    body: Value,
    inputs: Vec<Input>,
    range: Range,
    file: usize,
    /// The directory of the file it is in, which bounds what Vector merges.
    directory: String,
}

impl Entry {
    /// Places the entry in the `file`th file of a pipeline, under `directory`.
    pub(crate) fn in_file(mut self, file: usize, directory: &str) -> Self {
        self.file = file;
        self.directory = directory.to_owned();
        for input in &mut self.inputs {
            input.file = file;
        }
        self
    }

    /// The name of an enrichment table, for the VRL compiler.
    fn table(&self) -> Option<&str> {
        (self.role == Role::Table).then_some(self.id.as_str())
    }
}

/// A config file read as far as its entries, not yet merged with anything.
#[derive(Debug, Clone, Default)]
pub(crate) struct Entries {
    pub(crate) entries: Vec<Entry>,
    pub(crate) relaxed_wildcards: bool,
}

impl Entries {
    fn into_document(self) -> Document {
        Document {
            components: assemble(self.entries),
            relaxed_wildcards: self.relaxed_wildcards,
        }
    }
}

/// A component's body, whichever format it was written in.
///
/// Both parsers convert into this and nothing else answers [`Fields`], so the
/// YAML and the TOML reader cannot disagree about what a field means — and a
/// component written in two files can be merged the way Vector merges it,
/// which needs the values, not the syntax trees of two different parsers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Bool(bool),
    Text(String),
    List(Vec<Value>),
    /// In the order the keys are written.
    Map(Vec<(String, Value)>),
    /// A number, a date, a null: nothing the graph reads.
    Other,
}

impl Value {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Map(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Vector's `merge_values` (`src/config/loading/representation.rs`):
    /// mappings merge key by key, lists are concatenated, and anything else is
    /// replaced by the later value. Vector refuses two values of different
    /// kinds at one key; here the later one wins, since the graph reads so
    /// little of a component that the conflict is rarely in anything it shows.
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Map(mut entries), Self::Map(other)) => {
                for (key, value) in other {
                    match entries.iter_mut().find(|(name, _)| *name == key) {
                        Some(slot) => {
                            let existing = std::mem::replace(&mut slot.1, Self::Other);
                            slot.1 = existing.merge(value);
                        }
                        None => entries.push((key, value)),
                    }
                }
                Self::Map(entries)
            }
            (Self::List(mut items), Self::List(other)) => {
                items.extend(other);
                Self::List(items)
            }
            (_, other) => other,
        }
    }
}

/// Reads a YAML Vector configuration.
///
/// JSON is read by this too: Vector accepts `.json` configs, and YAML is a
/// superset of JSON, so the same parser handles both and keeps the spans.
///
/// # Errors
/// When the document is not YAML.
pub fn read_yaml(source: &str) -> Result<Document, ConfigError> {
    yaml_entries(source).map(Entries::into_document)
}

/// Reads a TOML Vector configuration.
///
/// # Errors
/// When the document is not TOML.
pub fn read_toml(source: &str) -> Result<Document, ConfigError> {
    toml_entries(source).map(Entries::into_document)
}

/// Offsets in the interpolated text, turned into positions in the file as
/// written. See [`crate::vars`].
struct Positions<'a> {
    index: LineIndex<'a>,
    text: &'a Interpolated,
}

impl<'a> Positions<'a> {
    fn new(source: &'a str, text: &'a Interpolated) -> Self {
        Self {
            index: LineIndex::new(source),
            text,
        }
    }

    fn range(&self, span: std::ops::Range<usize>) -> Range {
        self.index
            .range(self.text.original(span.start)..self.text.original(span.end))
    }
}

/// [`read_yaml`], as far as the entries.
pub(crate) fn yaml_entries(source: &str) -> Result<Entries, ConfigError> {
    let text = vars::interpolate(source, Syntax::Yaml);
    let positions = Positions::new(source, &text);

    let documents = MarkedYaml::load_from_str(&text.text).map_err(|error| ConfigError {
        message: error.to_string(),
        range: None,
    })?;

    // An empty file is an empty config, not a failure. It is what every config
    // looks like for its first few seconds.
    let Some(root) = documents.first() else {
        return Ok(Entries::default());
    };

    let mut entries = Vec::new();
    for role in Role::ALL {
        let Some(section) = root.data.as_mapping_get(role.section()) else {
            continue;
        };
        let Some(section) = section.data.as_mapping() else {
            continue;
        };

        for (key, body) in section {
            let Some(id) = key.data.as_str() else { continue };
            entries.push(Entry {
                role,
                id: id.to_owned(),
                body: yaml_value(body),
                inputs: yaml_inputs(body, &positions),
                range: yaml_range(key, &positions),
                file: 0,
                directory: String::new(),
            });
        }
    }

    Ok(Entries {
        entries,
        relaxed_wildcards: root
            .data
            .as_mapping_get(WILDCARD_MATCHING)
            .and_then(|node| node.data.as_str())
            == Some(RELAXED),
    })
}

fn yaml_value(node: &MarkedYaml<'_>) -> Value {
    if let Some(flag) = node.data.as_bool() {
        Value::Bool(flag)
    } else if let Some(text) = node.data.as_str() {
        Value::Text(text.to_owned())
    } else if let Some(items) = node.data.as_sequence() {
        Value::List(items.iter().map(yaml_value).collect())
    } else if let Some(entries) = node.data.as_mapping() {
        Value::Map(
            entries
                .iter()
                .filter_map(|(key, value)| Some((key.data.as_str()?.to_owned(), yaml_value(value))))
                .collect(),
        )
    } else {
        Value::Other
    }
}

fn yaml_range(node: &MarkedYaml<'_>, positions: &Positions<'_>) -> Range {
    positions.range(node.span.start.index()..node.span.end.index())
}

fn yaml_inputs(body: &MarkedYaml<'_>, positions: &Positions<'_>) -> Vec<Input> {
    let Some(node) = body.data.as_mapping_get("inputs") else {
        return Vec::new();
    };

    // Vector wants a list. A bare string is read anyway: drawing the edge
    // somebody plainly meant beats refusing over a missing dash.
    if let Some(text) = node.data.as_str() {
        return vec![Input {
            text: text.to_owned(),
            range: yaml_range(node, positions),
            file: 0,
        }];
    }

    node.data.as_sequence().map_or_else(Vec::new, |entries| {
        entries
            .iter()
            .filter_map(|entry| {
                entry.data.as_str().map(|text| Input {
                    text: text.to_owned(),
                    range: yaml_range(entry, positions),
                    file: 0,
                })
            })
            .collect()
    })
}

/// [`read_toml`], as far as the entries.
pub(crate) fn toml_entries(source: &str) -> Result<Entries, ConfigError> {
    let text = vars::interpolate(source, Syntax::Toml);
    let positions = Positions::new(source, &text);

    // `Document`, not `DocumentMut`. The mutable document is the one built for
    // rewriting a file, and it drops every span on the way: ask it where a key
    // is and it answers `None`, which is a silent wrong answer rather than an
    // error. Nothing here edits anything, so the immutable parse is both the
    // honest choice and the only one that keeps the positions this crate
    // exists to carry.
    let document = toml_edit::Document::parse(text.text.as_str()).map_err(|error| ConfigError {
        message: error.message().to_owned(),
        range: error.span().map(|span| positions.range(span)),
    })?;

    let mut entries = Vec::new();
    for role in Role::ALL {
        let Some(section) = document
            .get(role.section())
            .and_then(toml_edit::Item::as_table)
        else {
            continue;
        };

        for (id, body) in section.iter() {
            let Some(table) = body.as_table_like() else {
                continue;
            };

            entries.push(Entry {
                role,
                id: id.to_owned(),
                body: toml_value(body),
                inputs: toml_inputs(table, &positions),
                range: section
                    .key(id)
                    .and_then(toml_edit::Key::span)
                    .map_or_else(Range::default, |span| positions.range(span)),
                file: 0,
                directory: String::new(),
            });
        }
    }

    Ok(Entries {
        entries,
        relaxed_wildcards: document
            .get(WILDCARD_MATCHING)
            .and_then(toml_edit::Item::as_str)
            == Some(RELAXED),
    })
}

/// `[[transforms.x.routes]]` tables and an inline array of
/// `{ name = ..., condition = ... }` both come out as a list of maps.
fn toml_value(item: &toml_edit::Item) -> Value {
    match item {
        toml_edit::Item::None => Value::Other,
        toml_edit::Item::Value(value) => toml_inline(value),
        toml_edit::Item::Table(table) => Value::Map(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), toml_value(value)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(tables) => Value::List(
            tables
                .iter()
                .map(|table| {
                    Value::Map(
                        table
                            .iter()
                            .map(|(key, value)| (key.to_owned(), toml_value(value)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn toml_inline(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(text) => Value::Text(text.value().clone()),
        toml_edit::Value::Boolean(flag) => Value::Bool(*flag.value()),
        toml_edit::Value::Array(items) => Value::List(items.iter().map(toml_inline).collect()),
        toml_edit::Value::InlineTable(table) => Value::Map(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), toml_inline(value)))
                .collect(),
        ),
        _ => Value::Other,
    }
}

fn toml_inputs(table: &dyn toml_edit::TableLike, positions: &Positions<'_>) -> Vec<Input> {
    let Some(item) = table.get("inputs") else {
        return Vec::new();
    };
    let range = |span: Option<std::ops::Range<usize>>| {
        span.map_or_else(Range::default, |span| positions.range(span))
    };

    if let Some(text) = item.as_str() {
        return vec![Input {
            text: text.to_owned(),
            range: range(item.span()),
            file: 0,
        }];
    }

    item.as_array().map_or_else(Vec::new, |entries| {
        entries
            .iter()
            .filter_map(|entry| {
                entry.as_str().map(|text| Input {
                    text: text.to_owned(),
                    range: range(entry.span()),
                    file: 0,
                })
            })
            .collect()
    })
}

/// Puts the entries of every file together into components.
///
/// Entries of one role and one name, in files of one directory, are one
/// component written in pieces — the case Vector's `--config-dir` exists for,
/// and the only one in which it accepts a piece that is not a whole component:
/// given with `--config`, every file must stand alone, and a piece without a
/// `type` fails to load. Beyond a directory nothing is merged, as in Vector,
/// which `append`s each `--config-dir` to the others and refuses a name they
/// share.
///
/// What is a piece is decided by `type`. One file declares the component and
/// says what it is; the others only add to it. Two that both say what it is
/// are two components with one name — which is what files from two unrelated
/// pipelines look like, and what [`crate::graph`] reports. Vector itself,
/// given them in one `--config-dir`, would quietly let the later one win; no
/// one writes a config meaning that, and drawing it as one component would
/// hide a mistake.
///
/// Pieces merge in the order the files were given, with Vector's
/// [`Value::merge`], and the component is placed where its declaration is.
#[must_use]
pub(crate) fn assemble(entries: Vec<Entry>) -> Vec<Component> {
    let mut groups: Vec<Vec<Entry>> = Vec::new();
    let mut index: HashMap<(Role, String, String), usize> = HashMap::new();
    for entry in entries {
        let key = (entry.role, entry.id.clone(), entry.directory.clone());
        match index.get(&key) {
            Some(&group) => groups[group].push(entry),
            None => {
                index.insert(key, groups.len());
                groups.push(vec![entry]);
            }
        }
    }

    let mut components = Vec::new();
    for group in groups {
        for entry in merge(group) {
            push(&mut components, entry);
        }
    }
    components
}

/// One group of [`assemble`]: every declaration stays itself, and the pieces
/// join the first of them.
fn merge(group: Vec<Entry>) -> Vec<Entry> {
    if group.len() == 1 {
        return group;
    }

    let (mut declared, pieces): (Vec<Entry>, Vec<Entry>) =
        group.into_iter().partition(|entry| entry.body.get("type").is_some());
    if pieces.is_empty() {
        return declared;
    }

    // Nothing says what it is: the pieces are still one component, without a
    // type, which is the finding to show rather than one per piece.
    let anchor = if declared.is_empty() {
        None
    } else {
        Some(declared.remove(0))
    };
    let (range, file) = anchor
        .as_ref()
        .or(pieces.first())
        .map(|entry| (entry.range, entry.file))
        .unwrap_or_default();

    let mut all: Vec<Entry> = anchor.into_iter().chain(pieces).collect();
    all.sort_by_key(|entry| entry.file);
    let mut all = all.into_iter();
    let Some(mut whole) = all.next() else {
        return declared;
    };
    for piece in all {
        whole.body = whole.body.merge(piece.body);
        whole.inputs.extend(piece.inputs);
    }
    whole.range = range;
    whole.file = file;

    let mut merged = vec![whole];
    merged.extend(declared);
    merged
}

/// Adds a component and, when it has one, the source half of an enrichment
/// table — a second component under its own name, which is the shape Vector
/// compiles it into.
///
/// Its range is the table's, because that is where the name is written and so
/// where clicking the node should land.
fn push(components: &mut Vec<Component>, entry: Entry) {
    let fields = Body(&entry.body);
    let component_type = fields.text("type").unwrap_or_default();
    let outputs = outputs::declared(entry.role, &component_type, &fields);

    if entry.role == Role::Table {
        if let Some((source_key, source)) = outputs::table_source(&component_type, &fields) {
            components.push(Component {
                id: source_key,
                role: entry.role,
                component_type: component_type.clone(),
                inputs: Vec::new(),
                range: entry.range,
                named_outputs: source.named,
                default_output: source.default,
                file: entry.file,
            });
        }
    }

    components.push(Component {
        id: entry.id,
        role: entry.role,
        component_type,
        inputs: entry.inputs,
        range: entry.range,
        named_outputs: outputs.named,
        default_output: outputs.default,
        file: entry.file,
    });
}

/// One component's fields, for [`crate::outputs`].
#[derive(Clone, Copy)]
struct Body<'a>(&'a Value);

impl Fields for Body<'_> {
    fn flag(&self, key: &str, default: bool) -> bool {
        match self.0.get(key) {
            Some(Value::Bool(flag)) => *flag,
            _ => default,
        }
    }

    fn text(&self, key: &str) -> Option<String> {
        match self.0.get(key)? {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        }
    }

    fn keys(&self, key: &str) -> Option<Vec<String>> {
        match self.0.get(key)? {
            Value::Map(entries) => Some(entries.iter().map(|(name, _)| name.clone()).collect()),
            _ => None,
        }
    }

    fn names(&self, key: &str) -> Option<Vec<String>> {
        match self.0.get(key)? {
            Value::List(items) => Some(
                items
                    .iter()
                    .filter_map(|item| match item.get("name") {
                        Some(Value::Text(name)) => Some(name.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }

    fn child(&self, key: &str) -> Option<Self> {
        self.0
            .get(key)
            .filter(|value| matches!(value, Value::Map(_)))
            .map(Body)
    }
}

/// The names a YAML config declares under `enrichment_tables`.
///
/// Only the names: they are what the VRL compiler checks an enrichment lookup
/// against. What each table holds lives in a file on the machine Vector runs
/// on, not in the config.
///
/// # Errors
/// When the document is not YAML.
pub fn read_yaml_enrichment_tables(source: &str) -> Result<Vec<String>, ConfigError> {
    Ok(tables(yaml_entries(source)?))
}

/// [`read_yaml_enrichment_tables`], for a TOML config.
///
/// # Errors
/// When the document is not TOML.
pub fn read_toml_enrichment_tables(source: &str) -> Result<Vec<String>, ConfigError> {
    Ok(tables(toml_entries(source)?))
}

fn tables(read: Entries) -> Vec<String> {
    read.entries
        .iter()
        .filter_map(Entry::table)
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{read_toml, read_yaml, Role};

    /// The two readers have to agree about everything, so they are given the
    /// same config twice and compared field by field.
    fn both(yaml: &str, toml: &str) -> Vec<(String, Role, bool, Vec<String>)> {
        let of = |components: Vec<super::Component>| {
            components
                .into_iter()
                .map(|c| (c.id, c.role, c.default_output, c.named_outputs))
                .collect::<Vec<_>>()
        };
        let from_yaml = of(read_yaml(yaml).expect("yaml parses").components);
        let from_toml = of(read_toml(toml).expect("toml parses").components);
        assert_eq!(from_yaml, from_toml, "the two readers disagree");
        from_yaml
    }

    #[test]
    fn a_router_is_read_the_same_from_both_formats() {
        let read = both(
            "transforms:\n  split:\n    type: route\n    inputs: [in]\n    route:\n      errors: 'true'\n      warns: 'true'\n",
            "[transforms.split]\ntype = \"route\"\ninputs = [\"in\"]\n[transforms.split.route]\nerrors = \"true\"\nwarns = \"true\"\n",
        );

        assert_eq!(read.len(), 1);
        assert!(!read[0].2, "a router has no default output");
        assert_eq!(read[0].3, ["errors", "warns", "_unmatched"]);
    }

    #[test]
    fn an_exclusive_route_is_read_the_same_from_both_formats() {
        let read = both(
            "transforms:\n  split:\n    type: exclusive_route\n    inputs: [in]\n    routes:\n      - name: a\n        condition: 'true'\n      - name: b\n        condition: 'true'\n",
            "[[transforms.split.routes]]\nname = \"a\"\ncondition = \"true\"\n\n[[transforms.split.routes]]\nname = \"b\"\ncondition = \"true\"\n\n[transforms.split]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n",
        );

        assert_eq!(read[0].3, ["a", "b", "_unmatched"]);
    }

    #[test]
    fn a_source_with_ports_is_read_the_same_from_both_formats() {
        let read = both(
            "sources:\n  otel:\n    type: opentelemetry\n",
            "[sources.otel]\ntype = \"opentelemetry\"\n",
        );

        assert!(!read[0].2);
        assert_eq!(read[0].3, ["logs", "metrics", "traces"]);
    }

    /// A `memory` table is two components: the table events are written into,
    /// and the source they are read back out of, under its own name.
    #[test]
    fn a_memory_table_is_read_as_both_halves() {
        let read = both(
            "enrichment_tables:\n  cache:\n    type: memory\n    inputs: [parse]\n    source_config:\n      source_key: cache_out\n      export_expired_items: true\n",
            "[enrichment_tables.cache]\ntype = \"memory\"\ninputs = [\"parse\"]\n[enrichment_tables.cache.source_config]\nsource_key = \"cache_out\"\nexport_expired_items = true\n",
        );

        assert_eq!(read.len(), 2);
        let source = read.iter().find(|(id, ..)| id == "cache_out").expect("the source half");
        assert_eq!(source.1, Role::Table);
        assert!(source.2);
        assert_eq!(source.3, ["expired"]);

        let table = read.iter().find(|(id, ..)| id == "cache").expect("the table");
        assert!(!table.2, "nothing reads the table itself");
    }

    #[test]
    fn a_plain_table_is_one_component_that_nothing_reads() {
        let read = both(
            "enrichment_tables:\n  hosts:\n    type: file\n",
            "[enrichment_tables.hosts]\ntype = \"file\"\n",
        );

        assert_eq!(read.len(), 1);
        assert!(!read[0].2);
        assert!(read[0].3.is_empty());
    }

    #[test]
    fn relaxed_wildcards_are_read_from_both_formats() {
        assert!(read_yaml("wildcard_matching: relaxed\nsources: {}\n").unwrap().relaxed_wildcards);
        assert!(read_toml("wildcard_matching = \"relaxed\"\n").unwrap().relaxed_wildcards);
        assert!(!read_yaml("wildcard_matching: strict\n").unwrap().relaxed_wildcards);
        assert!(!read_yaml("sources: {}\n").unwrap().relaxed_wildcards);
    }

    /// JSON is a Vector config format, and YAML is a superset of it.
    #[test]
    fn a_json_config_is_read_by_the_yaml_parser() {
        let read = read_yaml(
            r#"{"sources": {"app": {"type": "file"}}, "sinks": {"out": {"type": "console", "inputs": ["app"]}}}"#,
        )
        .expect("parses");

        let ids: Vec<&str> = read.components.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["app", "out"]);
        assert_eq!(read.components[1].inputs[0].text, "app");
    }
}
