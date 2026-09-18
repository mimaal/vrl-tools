//! Reading the components out of a Vector configuration.
//!
//! A Vector config is three maps of named components — `sources`, `transforms`
//! and `sinks` — and every transform and sink carries `inputs`, the IDs of the
//! components feeding it. That is the whole graph, stated outright, which is
//! why it can be read rather than inferred.
//!
//! Both formats land in the same [`Component`] list, so everything downstream
//! is written once and neither knows nor cares which it came from.
//!
//! Positions are kept from the start. A graph that cannot take you to the
//! component you clicked is half a feature, and spans cannot be retrofitted
//! through a serde round trip — which is why neither parser here is the serde
//! one.

use editor_text::{LineIndex, Range};
use saphyr::{LoadableYamlNode, MarkedYaml};

use crate::outputs;

/// Which of the three maps a component came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Source,
    Transform,
    Sink,
}

impl Role {
    /// The key this role lives under in a config.
    const fn section(self) -> &'static str {
        match self {
            Self::Source => "sources",
            Self::Transform => "transforms",
            Self::Sink => "sinks",
        }
    }

    const ALL: [Self; 3] = [Self::Source, Self::Transform, Self::Sink];
}

/// One entry of a component's `inputs`, as written.
///
/// Kept verbatim — wildcard, dotted output and all — because resolving it is a
/// separate job, and because what somebody typed is what has to be underlined
/// when it resolves to nothing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Input {
    pub text: String,
    pub range: Range,
}

/// A source, transform or sink.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Component {
    pub id: String,
    pub role: Role,
    /// The `type` field: `file`, `remap`, `route`, `console`.
    ///
    /// Empty when the config omits it. Vector would reject that, but it is no
    /// reason to refuse to draw what is there.
    #[serde(rename = "type")]
    pub component_type: String,
    pub inputs: Vec<Input>,
    /// Where the component is declared, for going to it from the graph.
    pub range: Range,
    /// The outputs this component offers besides its default one. See
    /// [`crate::outputs`].
    pub named_outputs: Vec<String>,
}

/// Why a config could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub message: String,
    /// Absent when the parser could not say where the trouble was.
    pub range: Option<Range>,
}

/// Reads a YAML Vector configuration.
///
/// # Errors
/// When the document is not YAML.
pub fn read_yaml(source: &str) -> Result<Vec<Component>, ConfigError> {
    let index = LineIndex::new(source);

    let documents = MarkedYaml::load_from_str(source).map_err(|error| ConfigError {
        message: error.to_string(),
        range: None,
    })?;

    // An empty file is an empty config, not a failure. It is what every config
    // looks like for its first few seconds.
    let Some(root) = documents.first() else {
        return Ok(Vec::new());
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

            components.push(Component {
                id: id.to_owned(),
                role,
                component_type: body
                    .data
                    .as_mapping_get("type")
                    .and_then(|node| node.data.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                inputs: yaml_inputs(body, &index),
                range: yaml_range(key, &index),
                named_outputs: yaml_named_outputs(body),
            });
        }
    }

    Ok(components)
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

fn yaml_named_outputs(body: &MarkedYaml<'_>) -> Vec<String> {
    let routes = body.data.as_mapping_get("route").and_then(|node| {
        node.data.as_mapping().map(|entries| {
            entries
                .keys()
                .filter_map(|key| key.data.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
    });

    outputs::declared(
        routes,
        yaml_flag(body, "reroute_dropped", false),
        yaml_flag(body, "reroute_unmatched", true),
    )
}

fn yaml_flag(body: &MarkedYaml<'_>, key: &str, default: bool) -> bool {
    body.data
        .as_mapping_get(key)
        .and_then(|node| node.data.as_bool())
        .unwrap_or(default)
}

/// Reads a TOML Vector configuration.
///
/// # Errors
/// When the document is not TOML.
pub fn read_toml(source: &str) -> Result<Vec<Component>, ConfigError> {
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

            components.push(Component {
                id: id.to_owned(),
                role,
                component_type: table
                    .get("type")
                    .and_then(toml_edit::Item::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                inputs: toml_inputs(table, &index),
                range: section
                    .key(id)
                    .and_then(toml_edit::Key::span)
                    .map_or_else(Range::default, |span| index.range(span)),
                named_outputs: toml_named_outputs(table),
            });
        }
    }

    Ok(components)
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

fn toml_named_outputs(table: &dyn toml_edit::TableLike) -> Vec<String> {
    let routes = table
        .get("route")
        .and_then(toml_edit::Item::as_table_like)
        .map(|routes| {
            routes
                .iter()
                .map(|(name, _)| name.to_owned())
                .collect::<Vec<_>>()
        });

    outputs::declared(
        routes,
        toml_flag(table, "reroute_dropped", false),
        toml_flag(table, "reroute_unmatched", true),
    )
}

fn toml_flag(table: &dyn toml_edit::TableLike, key: &str, default: bool) -> bool {
    table
        .get(key)
        .and_then(toml_edit::Item::as_bool)
        .unwrap_or(default)
}
