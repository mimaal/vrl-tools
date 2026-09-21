//! Where events go in a Vector configuration.
//!
//! Reads a `vector.yaml` or `vector.toml` and works out the path an event
//! takes: which source produces it, which transforms it crosses, which sink it
//! ends in. Knows nothing about WebAssembly or VS Code, like the rest of
//! `crates/`.
//!
//! The graph is not inferred. Every transform and sink declares `inputs`, so
//! the edges are written down in the file. The work is resolving what they
//! name, because an input can be a wildcard, and it can name one particular
//! output of a component rather than the component itself.

pub mod config;
pub mod graph;
pub mod layout;
pub mod outputs;
pub mod render;

pub use config::{
    read_toml, read_toml_enrichment_tables, read_yaml, read_yaml_enrichment_tables, Component,
    ConfigError, Input, Role,
};
pub use graph::{build, Edge, Finding, Graph, Severity};
pub use layout::{layout, Layout, Placement, Route, Slot};
pub use render::{diagram, document};

/// Which parser to read a config with.
///
/// Decided by the caller from the file name, because a Vector config carries
/// no marker saying which it is, and guessing from the contents would get an
/// empty file wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Toml,
}

impl Format {
    /// The format a file name implies, if any.
    #[must_use]
    pub fn of(file_name: &str) -> Option<Self> {
        let name = file_name.to_ascii_lowercase();
        if name.ends_with(".yaml") || name.ends_with(".yml") {
            Some(Self::Yaml)
        } else if name.ends_with(".toml") {
            Some(Self::Toml)
        } else {
            None
        }
    }
}

/// A config, read, resolved and drawn.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    /// The Markdown document, ready to open.
    pub document: String,
    pub components: Vec<Component>,
    pub edges: Vec<Edge>,
    pub findings: Vec<Finding>,
    /// Where each component goes when drawn, by column and row. See
    /// [`layout`].
    pub layout: Layout,
}

/// Reads `source` and returns everything that can be said about it.
///
/// # Errors
/// When the document does not parse as its format at all. Anything short of
/// that — a missing `type`, an input naming nothing, no components whatsoever —
/// comes back as an [`Analysis`] with findings in it. A config being written is
/// wrong most of the time, and refusing to draw it then would make this
/// useless exactly when it is wanted.
pub fn analyse(source: &str, format: Format, title: &str) -> Result<Analysis, ConfigError> {
    let components = match format {
        Format::Yaml => read_yaml(source)?,
        Format::Toml => read_toml(source)?,
    };

    let graph = build(components);
    let layout = layout(&graph);

    Ok(Analysis {
        document: document(&graph, title),
        layout,
        components: graph.components,
        edges: graph.edges,
        findings: graph.findings,
    })
}

/// [`analyse`], as JSON, for crossing a language boundary.
///
/// A failure to parse comes back as `{"error": {…}}` rather than as a thrown
/// exception, so the caller has one shape to handle instead of two.
#[must_use]
pub fn analyse_json(source: &str, format: Format, title: &str) -> String {
    match analyse(source, format, title) {
        Ok(analysis) => serde_json::to_string(&analysis)
            .unwrap_or_else(|error| error_json(&error.to_string(), None)),
        Err(error) => error_json(&error.message, error.range),
    }
}

/// [`analyse_json`] for a named file, choosing the parser from the name.
///
/// The name is also the document's title, since it is what the person is
/// looking at.
#[must_use]
pub fn analyse_file_json(source: &str, file_name: &str) -> String {
    match Format::of(file_name) {
        Some(format) => analyse_json(source, format, file_name),
        None => error_json(
            &format!("{file_name} is neither a .yaml, a .yml nor a .toml file"),
            None,
        ),
    }
}

/// The enrichment table names a config file declares, choosing the parser
/// from the file name.
///
/// `None` when the name is not a config's, or the file does not parse: a
/// config being edited is broken most of the time, and the caller is better
/// off keeping what it last read than concluding the tables are gone.
#[must_use]
pub fn enrichment_tables(source: &str, file_name: &str) -> Option<Vec<String>> {
    match Format::of(file_name)? {
        Format::Yaml => read_yaml_enrichment_tables(source).ok(),
        Format::Toml => read_toml_enrichment_tables(source).ok(),
    }
}

fn error_json(message: &str, range: Option<editor_text::Range>) -> String {
    serde_json::json!({
        "error": {
            "message": message,
            "range": range,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{analyse_json, enrichment_tables, Format};

    #[test]
    fn enrichment_tables_are_read_from_both_formats() {
        let yaml = "enrichment_tables:
  hosts:
    type: file
  geo:
    type: geoip
sources: {}
";
        let toml = "[enrichment_tables.hosts]
type = \"file\"

[enrichment_tables.geo]
type = \"geoip\"
";

        for (source, name) in [(yaml, "vector.yaml"), (toml, "vector.toml")] {
            let mut tables = enrichment_tables(source, name).expect("parses");
            tables.sort();
            assert_eq!(tables, ["geo", "hosts"], "{name}");
        }
    }

    #[test]
    fn a_config_without_enrichment_tables_declares_none() {
        assert_eq!(
            enrichment_tables("sources: {}
", "vector.yaml"),
            Some(Vec::new())
        );
        assert_eq!(enrichment_tables("", "vector.toml"), Some(Vec::new()));
    }

    /// A broken file is "unknown", not "no tables": the caller keeps what it
    /// had rather than flagging every lookup while someone types.
    #[test]
    fn a_broken_config_says_nothing_about_its_tables() {
        assert_eq!(enrichment_tables("enrichment_tables: [oops
", "vector.yaml"), None);
        assert_eq!(enrichment_tables("x = 1
", "program.vrl"), None);
    }

    #[test]
    fn a_format_comes_from_the_file_name() {
        assert_eq!(Format::of("vector.yaml"), Some(Format::Yaml));
        assert_eq!(Format::of("vector.YML"), Some(Format::Yaml));
        assert_eq!(Format::of("vector.toml"), Some(Format::Toml));
        assert_eq!(Format::of("vector.json"), None);
    }

    #[test]
    fn the_json_carries_the_document_and_the_findings() {
        let json = analyse_json(
            "sinks:\n  out:\n    type: console\n    inputs: [nope]\n",
            Format::Yaml,
            "vector.yaml",
        );

        assert!(json.contains("\"document\""), "{json}");
        assert!(json.contains("flowchart LR"), "{json}");
        assert!(json.contains("no component is called"), "{json}");
        assert!(json.contains("\"namedOutputs\""), "{json}");
    }

    /// A config that does not parse still answers, so the caller has one shape
    /// to handle rather than two.
    #[test]
    fn a_broken_document_answers_with_an_error() {
        let json = analyse_json("sinks: [oops\n", Format::Yaml, "vector.yaml");

        assert!(json.starts_with("{\"error\""), "{json}");
    }
}
