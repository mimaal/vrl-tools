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

use std::collections::{HashMap, HashSet};

use editor_text::{LineIndex, Range};
use saphyr::{LoadableYamlNode, MarkedYaml};

use crate::graph::{Finding, Severity};
use crate::lookups::{self, Found};
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
    /// Where each of [`Self::named_outputs`] comes from, in the same order: the
    /// file that added the route, which for a router written across files is
    /// not the file that declares it. A list beside the names rather than a
    /// list of pairs, so a reader of the JSON that knows only the names keeps
    /// working.
    pub output_origins: Vec<Origin>,
    /// Whether an input can name the component itself. `false` for a router,
    /// for an `opentelemetry` source, and for anything events only end at.
    /// See [`crate::outputs`].
    pub default_output: bool,
    /// Which of the files being read declares it, as an index into the list
    /// the caller gave. `range` is in that file.
    /// Always 0 when a single file is read.
    pub file: usize,
    /// Every entry the component was merged from, in merge order: just the
    /// declaration, unless files of a `--config-dir` add to it.
    pub pieces: Vec<Origin>,
    /// The enrichment tables its VRL looks up by a literal name: a `remap`'s
    /// `source`, a condition. What a program kept in a file looks up is added
    /// once somebody hands the file over. See [`crate::lookups`].
    pub lookups: Vec<String>,
    /// Whether some lookup names its table in a way only the compiler can
    /// follow, so the list above may be short.
    pub opaque_lookups: bool,
    /// The VRL programs a `remap` reads from files, as written: its `file`
    /// and its `files`. Relative to the directory Vector is started in, which
    /// the config does not say.
    pub programs: Vec<String>,
    /// For an enrichment table read from a file, the path of its data: a
    /// `file` table's `file.path`, a `geoip` or `mmdb` table's `path`.
    pub path: Option<String>,
    /// Whether the first line of a `file` table's CSV is its header rather
    /// than a row: `file.encoding.include_headers`, which defaults to `true`.
    /// Needed by whoever counts the rows; `true` for everything else.
    pub csv_headers: bool,
}

/// A place in one of the files being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub file: usize,
    pub range: Range,
}

/// One entry of a merged component, with the body it wrote.
#[derive(Debug, Clone)]
struct Part {
    origin: Origin,
    body: Value,
}

impl Component {
    /// Whether anything can read this component at all.
    #[must_use]
    pub fn has_outputs(&self) -> bool {
        self.default_output || !self.named_outputs.is_empty()
    }

    /// Whether this is a table VRL consults and nothing else: no `inputs`
    /// writing into it and no source half reading out of it, so not a single
    /// arrow touches it. Every table but a `memory` one is this.
    #[must_use]
    pub fn is_lookup_table(&self) -> bool {
        self.role == Role::Table && self.inputs.is_empty() && !self.has_outputs()
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
    /// What is wrong with a component that only shows once the files are put
    /// together — a piece with no declaration to join. See [`assemble`].
    pub findings: Vec<Finding>,
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
    /// Whether the file was given to Vector with `--config`, which loads each
    /// file on its own and merges nothing. See [`crate::ConfigFile`].
    standalone: bool,
    /// What it was merged from, once it has been. See [`merge`].
    parts: Vec<Part>,
}

impl Entry {
    /// Places the entry in the `file`th file of a pipeline, under `directory`.
    pub(crate) fn in_file(mut self, file: usize, directory: &str, standalone: bool) -> Self {
        self.file = file;
        self.directory = directory.to_owned();
        self.standalone = standalone;
        for input in &mut self.inputs {
            input.file = file;
        }
        self
    }

    /// The name of an enrichment table, for the VRL compiler.
    fn table(&self) -> Option<&str> {
        (self.role == Role::Table).then_some(self.id.as_str())
    }

    /// Whether the entry says what the component is, rather than adding to
    /// one declared elsewhere.
    fn declares(&self) -> bool {
        self.body.get("type").is_some()
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
        let (components, findings) = assemble(self.entries, &[]);
        Document {
            components,
            relaxed_wildcards: self.relaxed_wildcards,
            findings,
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
    Integer(i64),
    Float(f64),
    Null,
    /// A date: nothing the graph reads, and nothing two files set differently
    /// that this could tell apart.
    Other,
}

impl Value {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Map(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Whether Vector's `merge_values` would replace one with the other rather
    /// than refuse: the same kind, and for a number the same kind of number.
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// The kind, for saying two do not merge.
    fn kind(&self) -> &'static str {
        match self {
            Self::Bool(_) => "a boolean",
            Self::Text(_) => "a string",
            Self::List(_) => "a list",
            Self::Map(_) => "a table",
            Self::Integer(_) => "an integer",
            Self::Float(_) => "a float",
            Self::Null => "null",
            Self::Other => "a date",
        }
    }

    /// A scalar as it would be written, for saying which one Vector keeps.
    fn shown(&self) -> String {
        match self {
            Self::Bool(flag) => format!("`{flag}`"),
            Self::Text(text) => format!("`{text}`"),
            Self::Integer(number) => format!("`{number}`"),
            Self::Float(number) => format!("`{number}`"),
            _ => self.kind().to_owned(),
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
                standalone: false,
                parts: Vec::new(),
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
    } else if let Some(number) = node.data.as_integer() {
        Value::Integer(number)
    } else if let Some(number) = node.data.as_floating_point() {
        Value::Float(number)
    } else if node.data.is_null() {
        Value::Null
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
                standalone: false,
                parts: Vec::new(),
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
        toml_edit::Value::Integer(number) => Value::Integer(*number.value()),
        toml_edit::Value::Float(number) => Value::Float(*number.value()),
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
/// component — the case Vector's `--config-dir` exists for. Vector reads the
/// top-level files of the directory as one value before it builds a single
/// component (`load_from_dir`, `merge_into_map`), so a file adding
/// `[[transforms.split.routes]]` to a router another declares, and a file
/// repeating the whole declaration, both come out as one router. Beyond a
/// directory nothing is merged, as in Vector, which `append`s each
/// `--config-dir` to the others and refuses a name they share ("duplicate
/// transform id"); and a file given with `--config` merges with nothing.
///
/// Telling unrelated configs apart is not this function's job: two
/// `vector.yaml` examples side by side are split into separate pipelines by
/// `editors/vscode/src/grouping.ts` before they get here. What arrives is one
/// pipeline, and within one pipeline Vector merges.
///
/// The files merge in name order ([`merge`]), and where two of them set one
/// field to different values the later one wins silently, as in Vector — which
/// is exactly why it is worth a finding ([`clash`]). Whatever is left without
/// a `type` is a finding too, saying why ([`untyped`]).
#[must_use]
pub(crate) fn assemble(entries: Vec<Entry>, names: &[String]) -> (Vec<Component>, Vec<Finding>) {
    let declared: HashSet<(Role, String)> = entries
        .iter()
        .filter(|entry| entry.declares())
        .map(|entry| (entry.role, entry.id.clone()))
        .collect();

    let mut groups: Vec<Vec<Entry>> = Vec::new();
    let mut index: HashMap<(Role, String, String, Option<usize>), usize> = HashMap::new();
    for entry in entries {
        let key = (
            entry.role,
            entry.id.clone(),
            entry.directory.clone(),
            entry.standalone.then_some(entry.file),
        );
        match index.get(&key) {
            Some(&group) => groups[group].push(entry),
            None => {
                index.insert(key, groups.len());
                groups.push(vec![entry]);
            }
        }
    }

    let mut components = Vec::new();
    let mut findings = Vec::new();
    for group in groups {
        let entry = merge(group, names, &mut findings);
        if !entry.declares() {
            let elsewhere = declared.contains(&(entry.role, entry.id.clone()));
            findings.push(untyped(&entry, elsewhere));
        }
        push(&mut components, entry);
    }
    (components, findings)
}

/// A component nothing says the `type` of, with the reason that applies.
///
/// Vector refuses it either way — the component cannot be deserialised without
/// one — but the fix differs. When a `type` exists in another file, the file
/// was written as a piece and the question is how Vector was given the files;
/// when none does, the `type` is simply missing.
fn untyped(entry: &Entry, declared_elsewhere: bool) -> Finding {
    let id = &entry.id;
    let message = if !declared_elsewhere {
        format!("`{id}` has no `type`, and Vector needs one to know what it is")
    } else if entry.standalone {
        format!(
            "`{id}` has no `type`: this adds to a `{id}` declared in another file, but a file \
             given with `--config` is loaded on its own. Only the files of one `--config-dir` \
             are merged",
        )
    } else {
        format!(
            "`{id}` has no `type`: this adds to a `{id}` declared in another directory, and \
             Vector merges the pieces of a component only within one `--config-dir`",
        )
    };
    Finding {
        severity: Severity::Error,
        message,
        range: entry.range,
        file: entry.file,
    }
}

/// One group of [`assemble`], merged into one entry.
///
/// **Order.** By file name, which is the order the fixture this was measured
/// against gives and the only one that can be reproduced. Vector itself does
/// not sort: it merges in the order `read_dir` lists the directory (its
/// `serde_json` keeps insertion order, `preserve_order`), which is
/// alphabetical on some filesystems and hash order on ext4. Lists are
/// concatenated, so the order is the order of a router's routes — which for
/// an `exclusive_route` is the order they are tried in. A config that depends
/// on it depends on the filesystem; `tests/against_vector.rs` re-reads that
/// Vector still does not sort.
///
/// **Where it is.** The component is placed at the first entry whose `type`
/// is the one that won — the declaration, not a file adding a route — so
/// clicking it goes where it is said what it is. Every entry is kept as a
/// [`Part`], for placing each named output at the file that added it.
fn merge(mut group: Vec<Entry>, names: &[String], findings: &mut Vec<Finding>) -> Entry {
    let name = |file: usize| names.get(file).map_or("", String::as_str);
    // Stable, so the entries of one file keep their order.
    group.sort_by(|a, b| name(a.file).cmp(name(b.file)));

    let parts: Vec<Part> = group
        .iter()
        .map(|entry| Part {
            origin: Origin {
                file: entry.file,
                range: entry.range,
            },
            body: entry.body.clone(),
        })
        .collect();

    let mut entries = group.into_iter();
    let Some(mut whole) = entries.next() else {
        unreachable!("a group has at least the entry that started it");
    };
    let mut setters = HashMap::new();
    record(&whole.body, whole.file, "", &mut setters);
    for piece in entries {
        let mut clashes = Vec::new();
        let at = Origin {
            file: piece.file,
            range: piece.range,
        };
        let body = std::mem::replace(&mut whole.body, Value::Null);
        whole.body = merge_values(body, piece.body, piece.file, "", &mut setters, &mut clashes);
        findings.extend(
            clashes
                .into_iter()
                .map(|found| clash(&whole.id, &found, at, names)),
        );
        whole.inputs.extend(piece.inputs);
    }

    let winner = whole.body.get("type");
    if let Some(anchor) = parts
        .iter()
        .find(|part| winner.is_some() && part.body.get("type") == winner)
    {
        whole.file = anchor.origin.file;
        whole.range = anchor.origin.range;
    }
    whole.parts = parts;
    whole
}

/// Where each field of a merged body was last set, by path, so a clash can
/// name the file that loses it.
fn record(value: &Value, file: usize, path: &str, setters: &mut HashMap<String, usize>) {
    setters.insert(path.to_owned(), file);
    if let Value::Map(entries) = value {
        for (key, value) in entries {
            record(value, file, &join(path, key), setters);
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Two files setting one field differently.
struct Clash {
    path: String,
    before: Value,
    before_file: usize,
    after: Value,
}

/// Vector's `merge_values` (`src/config/loading/representation.rs`), noting
/// every field it overwrites with a different value.
///
/// Mappings merge key by key, lists are concatenated, and any other value is
/// replaced by the later one — silently when both are the same kind, with
/// "Incompatible types" when they are not (a string and a table, an integer
/// and a float). The replacement is kept here in both cases, because the graph
/// still has to be drawn; [`clash`] says which it was.
fn merge_values(
    value: Value,
    other: Value,
    file: usize,
    path: &str,
    setters: &mut HashMap<String, usize>,
    clashes: &mut Vec<Clash>,
) -> Value {
    match (value, other) {
        (Value::Map(mut entries), Value::Map(other)) => {
            setters.insert(path.to_owned(), file);
            for (key, value) in other {
                let inner = join(path, &key);
                match entries.iter_mut().find(|(name, _)| *name == key) {
                    Some(slot) => {
                        let existing = std::mem::replace(&mut slot.1, Value::Null);
                        slot.1 = merge_values(existing, value, file, &inner, setters, clashes);
                    }
                    None => {
                        record(&value, file, &inner, setters);
                        entries.push((key, value));
                    }
                }
            }
            Value::Map(entries)
        }
        (Value::List(mut items), Value::List(other)) => {
            setters.insert(path.to_owned(), file);
            items.extend(other);
            Value::List(items)
        }
        (before, after) => {
            if before != after {
                clashes.push(Clash {
                    path: path.to_owned(),
                    before_file: setters.get(path).copied().unwrap_or(file),
                    before,
                    after: after.clone(),
                });
            }
            record(&after, file, path, setters);
            after
        }
    }
}

/// The finding for a [`Clash`], at the entry that wins it.
///
/// A warning, because Vector starts: the later file's value is simply the one
/// it uses, and the earlier file is misleading whoever reads it. An error when
/// the two are different kinds, which Vector refuses to merge, and when the
/// field is `type` — the component is then something the rest of its fields
/// were not written for, and fails validation on the first that does not fit.
fn clash(id: &str, found: &Clash, winner: Origin, names: &[String]) -> Finding {
    let short = |file: usize| {
        names
            .get(file)
            .map_or("another file", |name| name.rsplit(['/', '\\']).next().unwrap_or(name))
            .to_owned()
    };
    let (earlier, later) = (short(found.before_file), short(winner.file));
    let key = &found.path;

    let (severity, message) = if !found.before.same_kind(&found.after) {
        (
            Severity::Error,
            format!(
                "`{id}` sets `{key}` to {} in {earlier} and to {} in {later}, which Vector refuses \
                 to merge",
                found.before.kind(),
                found.after.kind(),
            ),
        )
    } else if key == "type" {
        (
            Severity::Error,
            format!(
                "`{id}` is {} in {earlier} and {} in {later}; Vector keeps {later}'s",
                found.before.shown(),
                found.after.shown(),
            ),
        )
    } else {
        (
            Severity::Warning,
            format!(
                "`{id}` sets `{key}` to {} in {earlier} and to {} in {later}; Vector keeps \
                 {later}'s",
                found.before.shown(),
                found.after.shown(),
            ),
        )
    };
    Finding {
        severity,
        message,
        range: winner.range,
        file: winner.file,
    }
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
    let anchor = Origin {
        file: entry.file,
        range: entry.range,
    };
    let parts = if entry.parts.is_empty() {
        vec![Part {
            origin: anchor,
            body: entry.body.clone(),
        }]
    } else {
        entry.parts
    };

    if entry.role == Role::Table {
        if let Some((source_key, source)) = outputs::table_source(&component_type, &fields) {
            components.push(Component {
                id: source_key,
                role: entry.role,
                component_type: component_type.clone(),
                inputs: Vec::new(),
                range: entry.range,
                output_origins: vec![anchor; source.named.len()],
                named_outputs: source.named,
                default_output: source.default,
                file: entry.file,
                pieces: vec![anchor],
                lookups: Vec::new(),
                opaque_lookups: false,
                programs: Vec::new(),
                path: None,
                csv_headers: true,
            });
        }
    }

    let output_origins = origins(entry.role, &component_type, &outputs.named, anchor, &parts);
    // A table is what is looked up, never what looks: its fields are paths
    // and schemas, not programs.
    let found = if entry.role == Role::Table {
        Found::default()
    } else {
        vrl_in(&entry.body)
    };
    let programs = programs(entry.role, &component_type, &entry.body);
    let path = table_path(entry.role, &component_type, &entry.body);
    let csv_headers = !matches!(
        entry
            .body
            .get("file")
            .and_then(|file| file.get("encoding"))
            .and_then(|encoding| encoding.get("include_headers")),
        Some(Value::Bool(false)),
    );
    components.push(Component {
        id: entry.id,
        role: entry.role,
        component_type,
        inputs: entry.inputs,
        range: entry.range,
        named_outputs: outputs.named,
        output_origins,
        default_output: outputs.default,
        file: entry.file,
        pieces: parts.iter().map(|part| part.origin).collect(),
        lookups: found.tables,
        opaque_lookups: found.opaque,
        programs,
        path,
        csv_headers,
    });
}

/// The lookups in every string of a component. See [`crate::lookups`] for why
/// every string rather than the fields known to hold VRL.
fn vrl_in(value: &Value) -> Found {
    let mut found = Found::default();
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Text(text) => found.merge(lookups::scan(text)),
            Value::List(items) => pending.extend(items.iter().rev()),
            Value::Map(entries) => pending.extend(entries.iter().rev().map(|(_, value)| value)),
            _ => {}
        }
    }
    found
}

/// The files a `remap` reads its program from: `file`, then `files`
/// (`RemapConfig`, `src/transforms/remap.rs`).
fn programs(role: Role, component_type: &str, body: &Value) -> Vec<String> {
    if role != Role::Transform || component_type != "remap" {
        return Vec::new();
    }
    let mut paths = Vec::new();
    if let Some(Value::Text(path)) = body.get("file") {
        paths.push(path.clone());
    }
    if let Some(Value::List(items)) = body.get("files") {
        paths.extend(items.iter().filter_map(|item| match item {
            Value::Text(path) => Some(path.clone()),
            _ => None,
        }));
    }
    paths
}

/// Where an enrichment table's data is: `file.path` for a `file` table
/// (`FileSettings`, `src/enrichment_tables/file.rs`), `path` for a `geoip` or
/// an `mmdb` one (`src/enrichment_tables/geoip.rs`, `mmdb.rs`). A `memory`
/// table has none: it is filled by the pipeline.
fn table_path(role: Role, component_type: &str, body: &Value) -> Option<String> {
    if role != Role::Table {
        return None;
    }
    let found = match component_type {
        "file" => body.get("file")?.get("path")?,
        "geoip" | "mmdb" => body.get("path")?,
        _ => return None,
    };
    match found {
        Value::Text(path) => Some(path.clone()),
        _ => None,
    }
}

/// Which part added each named output.
///
/// Asked of the rules in [`crate::outputs`] rather than worked out from the
/// field names: each part is read on its own, with the merged `type`, and the
/// first to offer an output added it. The declaration is asked first, so an
/// output every part would offer — `_unmatched` — belongs to it.
fn origins(
    role: Role,
    component_type: &str,
    named: &[String],
    anchor: Origin,
    parts: &[Part],
) -> Vec<Origin> {
    let offered: Vec<(Origin, Vec<String>)> = parts
        .iter()
        .filter(|part| part.origin == anchor)
        .chain(parts.iter().filter(|part| part.origin != anchor))
        .map(|part| {
            let fields = Body(&part.body);
            (part.origin, outputs::declared(role, component_type, &fields).named)
        })
        .collect();

    named
        .iter()
        .map(|output| {
            offered
                .iter()
                .find(|(_, names)| names.contains(output))
                .map_or(anchor, |(origin, _)| *origin)
        })
        .collect()
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
