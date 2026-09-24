//! Reading the components out of a Vector configuration.
//!
//! A Vector config is four maps of named components — `sources`, `transforms`,
//! `sinks` and `enrichment_tables` — and every transform, sink and table
//! carries `inputs`, the IDs of the components feeding it. That is the whole
//! graph, stated outright, which is why it can be read rather than inferred.
//!
//! Both formats land in the same [`Component`] list, so everything downstream
//! is written once and neither knows nor cares which it came from. What a
//! component's fields *mean* is [`crate::outputs`]'s, reached through the
//! [`Fields`] trait both parsers implement, so the two cannot drift apart.
//!
//! Positions are kept from the start. A graph that cannot take you to the
//! component you clicked is half a feature, and spans cannot be retrofitted
//! through a serde round trip — which is why neither parser here is the serde
//! one.

use editor_text::{LineIndex, Range};
use saphyr::{LoadableYamlNode, MarkedYaml};

use crate::outputs::{self, Fields};

/// Which of the four maps a component came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
    /// the caller gave. `range` and every input's range are in that file.
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

/// Reads a YAML Vector configuration.
///
/// JSON is read by this too: Vector accepts `.json` configs, and YAML is a
/// superset of JSON, so the same parser handles both and keeps the spans.
///
/// # Errors
/// When the document is not YAML.
pub fn read_yaml(source: &str) -> Result<Document, ConfigError> {
    let index = LineIndex::new(source);

    let documents = MarkedYaml::load_from_str(source).map_err(|error| ConfigError {
        message: error.to_string(),
        range: None,
    })?;

    // An empty file is an empty config, not a failure. It is what every config
    // looks like for its first few seconds.
    let Some(root) = documents.first() else {
        return Ok(Document::default());
    };

    let mut components = Vec::new();
    for role in Role::ALL {
        let Some(section) = root.data.as_mapping_get(role.section()) else {
            continue;
        };
        let Some(entries) = section.data.as_mapping() else {
            continue;
        };

        for (key, body) in entries {
            let Some(id) = key.data.as_str() else { continue };
            let fields = YamlFields(body);
            push(
                &mut components,
                id,
                role,
                &fields,
                yaml_inputs(body, &index),
                yaml_range(key, &index),
            );
        }
    }

    Ok(Document {
        components,
        relaxed_wildcards: root
            .data
            .as_mapping_get(WILDCARD_MATCHING)
            .and_then(|node| node.data.as_str())
            == Some(RELAXED),
    })
}

/// Adds a component and, when it has one, the source half of an enrichment
/// table — a second component under its own name, which is the shape Vector
/// compiles it into.
///
/// Its range is the table's, because that is where the name is written and so
/// where clicking the node should land.
fn push<F: Fields>(
    components: &mut Vec<Component>,
    id: &str,
    role: Role,
    fields: &F,
    inputs: Vec<Input>,
    range: Range,
) {
    let component_type = fields.text("type").unwrap_or_default();
    let outputs = outputs::declared(role, &component_type, fields);

    if role == Role::Table {
        if let Some((source_key, source)) = outputs::table_source(&component_type, fields) {
            components.push(Component {
                id: source_key,
                role,
                component_type: component_type.clone(),
                inputs: Vec::new(),
                range,
                named_outputs: source.named,
                default_output: source.default,
                file: 0,
            });
        }
    }

    components.push(Component {
        id: id.to_owned(),
        role,
        component_type,
        inputs,
        range,
        named_outputs: outputs.named,
        default_output: outputs.default,
        file: 0,
    });
}

/// One component's fields, as saphyr holds them.
#[derive(Clone, Copy)]
struct YamlFields<'a, 'b>(&'a MarkedYaml<'b>);

impl Fields for YamlFields<'_, '_> {
    fn flag(&self, key: &str, default: bool) -> bool {
        self.0
            .data
            .as_mapping_get(key)
            .and_then(|node| node.data.as_bool())
            .unwrap_or(default)
    }

    fn text(&self, key: &str) -> Option<String> {
        self.0
            .data
            .as_mapping_get(key)
            .and_then(|node| node.data.as_str())
            .map(ToOwned::to_owned)
    }

    fn keys(&self, key: &str) -> Option<Vec<String>> {
        let node = self.0.data.as_mapping_get(key)?;
        Some(
            node.data
                .as_mapping()?
                .keys()
                .filter_map(|key| key.data.as_str().map(ToOwned::to_owned))
                .collect(),
        )
    }

    fn names(&self, key: &str) -> Option<Vec<String>> {
        let node = self.0.data.as_mapping_get(key)?;
        Some(
            node.data
                .as_sequence()?
                .iter()
                .filter_map(|entry| {
                    entry
                        .data
                        .as_mapping_get("name")
                        .and_then(|name| name.data.as_str())
                        .map(ToOwned::to_owned)
                })
                .collect(),
        )
    }

    fn has(&self, key: &str) -> bool {
        self.0.data.as_mapping_get(key).is_some()
    }

    fn child(&self, key: &str) -> Option<Self> {
        self.0.data.as_mapping_get(key).map(YamlFields)
    }
}

fn yaml_range(node: &MarkedYaml<'_>, index: &LineIndex<'_>) -> Range {
    index.range(node.span.start.index()..node.span.end.index())
}

fn yaml_inputs(body: &MarkedYaml<'_>, index: &LineIndex<'_>) -> Vec<Input> {
    let Some(node) = body.data.as_mapping_get("inputs") else {
        return Vec::new();
    };

    // Vector wants a list. A bare string is read anyway: drawing the edge
    // somebody plainly meant beats refusing over a missing dash.
    if let Some(text) = node.data.as_str() {
        return vec![Input {
            text: text.to_owned(),
            range: yaml_range(node, index),
        }];
    }

    node.data.as_sequence().map_or_else(Vec::new, |entries| {
        entries
            .iter()
            .filter_map(|entry| {
                entry.data.as_str().map(|text| Input {
                    text: text.to_owned(),
                    range: yaml_range(entry, index),
                })
            })
            .collect()
    })
}

/// Reads a TOML Vector configuration.
///
/// # Errors
/// When the document is not TOML.
pub fn read_toml(source: &str) -> Result<Document, ConfigError> {
    let index = LineIndex::new(source);

    // `Document`, not `DocumentMut`. The mutable document is the one built for
    // rewriting a file, and it drops every span on the way: ask it where a key
    // is and it answers `None`, which is a silent wrong answer rather than an
    // error. Nothing here edits anything, so the immutable parse is both the
    // honest choice and the only one that keeps the positions this crate
    // exists to carry.
    let document = toml_edit::Document::parse(source).map_err(|error| ConfigError {
        message: error.message().to_owned(),
        range: error.span().map(|span| index.range(span)),
    })?;

    let mut components = Vec::new();
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

            let fields = TomlFields(table);
            push(
                &mut components,
                id,
                role,
                &fields,
                toml_inputs(table, &index),
                section
                    .key(id)
                    .and_then(toml_edit::Key::span)
                    .map_or_else(Range::default, |span| index.range(span)),
            );
        }
    }

    Ok(Document {
        components,
        relaxed_wildcards: document
            .get(WILDCARD_MATCHING)
            .and_then(toml_edit::Item::as_str)
            == Some(RELAXED),
    })
}

/// One component's fields, as `toml_edit` holds them.
#[derive(Clone, Copy)]
struct TomlFields<'a>(&'a dyn toml_edit::TableLike);

impl Fields for TomlFields<'_> {
    fn flag(&self, key: &str, default: bool) -> bool {
        self.0
            .get(key)
            .and_then(toml_edit::Item::as_bool)
            .unwrap_or(default)
    }

    fn text(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .and_then(toml_edit::Item::as_str)
            .map(ToOwned::to_owned)
    }

    fn keys(&self, key: &str) -> Option<Vec<String>> {
        let table = self.0.get(key)?.as_table_like()?;
        Some(table.iter().map(|(name, _)| name.to_owned()).collect())
    }

    /// `[[transforms.x.routes]]` tables, or an inline array of
    /// `{ name = ..., condition = ... }`.
    fn names(&self, key: &str) -> Option<Vec<String>> {
        let item = self.0.get(key)?;
        let name = |entry: &dyn toml_edit::TableLike| {
            entry
                .get("name")
                .and_then(toml_edit::Item::as_str)
                .map(ToOwned::to_owned)
        };
        if let Some(tables) = item.as_array_of_tables() {
            return Some(tables.iter().filter_map(|table| name(table)).collect());
        }
        Some(
            item.as_array()?
                .iter()
                .filter_map(|value| value.as_inline_table().and_then(|table| name(table)))
                .collect(),
        )
    }

    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }

    fn child(&self, key: &str) -> Option<Self> {
        self.0.get(key).and_then(toml_edit::Item::as_table_like).map(TomlFields)
    }
}

fn toml_inputs(table: &dyn toml_edit::TableLike, index: &LineIndex<'_>) -> Vec<Input> {
    let Some(item) = table.get("inputs") else {
        return Vec::new();
    };

    if let Some(text) = item.as_str() {
        return vec![Input {
            text: text.to_owned(),
            range: item
                .span()
                .map_or_else(Range::default, |span| index.range(span)),
        }];
    }

    item.as_array().map_or_else(Vec::new, |entries| {
        entries
            .iter()
            .filter_map(|entry| {
                entry.as_str().map(|text| Input {
                    text: text.to_owned(),
                    range: entry
                        .span()
                        .map_or_else(Range::default, |span| index.range(span)),
                })
            })
            .collect()
    })
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
    let documents = MarkedYaml::load_from_str(source).map_err(|error| ConfigError {
        message: error.to_string(),
        range: None,
    })?;

    Ok(documents
        .first()
        .and_then(|root| root.data.as_mapping_get(ENRICHMENT_TABLES))
        .and_then(|section| section.data.as_mapping())
        .map(|entries| {
            entries
                .keys()
                .filter_map(|key| key.data.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// [`read_yaml_enrichment_tables`], for a TOML config.
///
/// # Errors
/// When the document is not TOML.
pub fn read_toml_enrichment_tables(source: &str) -> Result<Vec<String>, ConfigError> {
    let index = LineIndex::new(source);
    let document = toml_edit::Document::parse(source).map_err(|error| ConfigError {
        message: error.message().to_owned(),
        range: error.span().map(|span| index.range(span)),
    })?;

    Ok(document
        .get(ENRICHMENT_TABLES)
        .and_then(toml_edit::Item::as_table_like)
        .map(|tables| tables.iter().map(|(name, _)| name.to_owned()).collect())
        .unwrap_or_default())
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
