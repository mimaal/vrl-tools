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
    ConfigError, Document, Input, Role,
};
pub use graph::{build, focus, Edge, Finding, Graph, Severity};
pub use layout::{layout, Layout, Placement, Route, Slot};
pub use render::{diagram, document, document_of_files};

/// Which parser to read a config with.
///
/// Decided by the caller from the file name, because a Vector config carries
/// no marker saying which it is, and guessing from the contents would get an
/// empty file wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Toml,
    /// Vector's third config format. Read by the YAML parser, since YAML is a
    /// superset of JSON and the parser keeps the spans either way.
    Json,
}

impl Format {
    /// The format a file name implies, if any. The same three extensions
    /// Vector accepts, and the same ones `--config-dir` keeps.
    #[must_use]
    pub fn of(file_name: &str) -> Option<Self> {
        let name = file_name.to_ascii_lowercase();
        if name.ends_with(".yaml") || name.ends_with(".yml") {
            Some(Self::Yaml)
        } else if name.ends_with(".toml") {
            Some(Self::Toml)
        } else if name.ends_with(".json") {
            Some(Self::Json)
        } else {
            None
        }
    }

    /// Reads a document in this format.
    ///
    /// # Errors
    /// When the document does not parse as this format at all.
    pub fn read(self, source: &str) -> Result<Document, ConfigError> {
        match self {
            Self::Yaml | Self::Json => read_yaml(source),
            Self::Toml => read_toml(source),
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
    /// The files read, in the order given. A component's or a finding's
    /// `file` is an index into this.
    pub files: Vec<String>,
    /// Files that could not be read at all, and why. Their components are
    /// missing from the graph, so a caller showing it live will usually want
    /// to keep the last complete one on screen instead.
    pub unreadable: Vec<Unreadable>,
    /// The component the graph is narrowed to, when it is. See [`focus`].
    pub focus: Option<String>,
}

/// One file of a pipeline, as given to [`analyse_files`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ConfigFile {
    /// Chooses the parser, and names the file in findings. A path relative to
    /// the config directory reads best.
    pub name: String,
    pub source: String,
}

/// A file of a pipeline that did not parse.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unreadable {
    pub file: usize,
    pub message: String,
    pub range: Option<editor_text::Range>,
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
    let graph = build(format.read(source)?);
    let layout = layout(&graph);

    Ok(Analysis {
        document: document(&graph, title),
        layout,
        components: graph.components,
        edges: graph.edges,
        findings: graph.findings,
        files: vec![title.to_owned()],
        unreadable: Vec::new(),
        focus: None,
    })
}

/// Reads a pipeline split across several files, the way Vector reads the
/// files given to it with `--config` (or a glob such as
/// `config/**/*.toml`): each one is a complete config with its own
/// `sources`, `transforms` and `sinks`, and the components of all of them are
/// one topology. An input in one file can name a source in another, and a name
/// used in two files is an error.
///
/// A file that does not parse is listed in [`Analysis::unreadable`] and the
/// rest are still read.
#[must_use]
pub fn analyse_files(files: &[ConfigFile], title: &str) -> Analysis {
    analyse_files_focused(files, title, None)
}

/// [`analyse_files`], narrowed to the paths through one component when
/// `focus` names one that exists. See [`focus`]. The layout is the narrowed
/// graph's own, so a handful of components fill the view rather than sitting
/// where they were in the whole pipeline.
#[must_use]
pub fn analyse_files_focused(files: &[ConfigFile], title: &str, focus: Option<&str>) -> Analysis {
    let mut whole = Document::default();
    let mut unreadable = Vec::new();

    for (position, file) in files.iter().enumerate() {
        let read = match Format::of(&file.name) {
            Some(format) => format.read(&file.source),
            None => Err(ConfigError {
                message: format!("{} is not a .yaml, .yml, .toml or .json file", file.name),
                range: None,
            }),
        };
        match read {
            Ok(read) => {
                whole.components.extend(read.components.into_iter().map(|mut component| {
                    component.file = position;
                    component
                }));
                // Vector merges the globals of every file it is given, so one
                // file relaxing wildcard matching relaxes it for the pipeline.
                // Erring towards relaxed keeps a setting this crate cannot see
                // the whole of from inventing errors.
                whole.relaxed_wildcards |= read.relaxed_wildcards;
            }
            Err(error) => unreadable.push(Unreadable {
                file: position,
                message: error.message,
                range: error.range,
            }),
        }
    }

    let names: Vec<String> = files.iter().map(|file| file.name.clone()).collect();
    let whole = build(whole);
    let (graph, focus) = match focus.and_then(|id| graph::focus(&whole, id).map(|g| (g, id))) {
        Some((narrowed, id)) => (narrowed, Some(id.to_owned())),
        None => (whole, None),
    };
    let layout = layout(&graph);

    Analysis {
        document: document_of_files(&graph, title, &names),
        layout,
        components: graph.components,
        edges: graph.edges,
        findings: graph.findings,
        files: names,
        unreadable,
        focus,
    }
}

/// [`analyse_files_focused`], taking and giving JSON: an array of
/// `{name, source}`.
#[must_use]
pub fn analyse_files_json(files_json: &str, title: &str, focus: Option<&str>) -> String {
    match serde_json::from_str::<Vec<ConfigFile>>(files_json) {
        Ok(files) => serde_json::to_string(&analyse_files_focused(&files, title, focus))
            .unwrap_or_else(|error| error_json(&error.to_string(), None)),
        Err(error) => error_json(&format!("not a list of config files: {error}"), None),
    }
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
            &format!("{file_name} is not a .yaml, .yml, .toml or .json file"),
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
        Format::Yaml | Format::Json => read_yaml_enrichment_tables(source).ok(),
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
    use super::{analyse_files, analyse_json, enrichment_tables, ConfigFile, Format};

    fn file(name: &str, source: &str) -> ConfigFile {
        ConfigFile {
            name: name.to_owned(),
            source: source.to_owned(),
        }
    }

    /// The layout Vector reads with `-c 'config/**/*.toml'`: sources in one
    /// file, the transforms reading them in another. Read one at a time, every
    /// input in the second file names "nothing".
    #[test]
    fn a_pipeline_split_across_files_is_one_graph() {
        let analysis = analyse_files(
            &[
                file("sources.toml", "[sources.app]\ntype = \"file\"\n"),
                file(
                    "nginx/parse.toml",
                    "[transforms.parse]\ntype = \"remap\"\ninputs = [\"app\"]\n\n[sinks.out]\ntype = \"console\"\ninputs = [\"parse\"]\n",
                ),
            ],
            "config",
        );

        assert!(analysis.findings.is_empty(), "{:?}", analysis.findings);
        assert_eq!(analysis.edges.len(), 2);
        let parse = analysis.components.iter().find(|c| c.id == "parse").expect("parse");
        assert_eq!(analysis.files[parse.file], "nginx/parse.toml");
        assert_eq!(analysis.edges[0].file, parse.file);
    }

    #[test]
    fn a_name_used_in_two_files_is_an_error_in_both() {
        let analysis = analyse_files(
            &[
                file("a.toml", "[sources.app]\ntype = \"file\"\n"),
                file("b.yaml", "sinks:\n  app:\n    type: console\n    inputs: [app]\n"),
            ],
            "config",
        );

        let duplicate: Vec<usize> = analysis
            .findings
            .iter()
            .filter(|f| f.message.contains("called `app`"))
            .map(|f| f.file)
            .collect();
        assert_eq!(duplicate, [0, 1], "{:?}", analysis.findings);
    }

    /// A file mid-edit does not take the rest of the pipeline with it.
    #[test]
    fn a_broken_file_is_reported_and_the_rest_still_read() {
        let analysis = analyse_files(
            &[
                file("ok.toml", "[sources.app]\ntype = \"file\"\n"),
                file("broken.toml", "[sinks.out\n"),
            ],
            "config",
        );

        assert_eq!(analysis.components.len(), 1);
        assert_eq!(analysis.unreadable.len(), 1);
        assert_eq!(analysis.unreadable[0].file, 1);
    }

    /// Narrowed to one component, the graph is its paths and nothing else,
    /// laid out on its own.
    #[test]
    fn focusing_keeps_only_the_paths_through_a_component() {
        let files = [file(
            "vector.toml",
            "[sources.a]\ntype = \"file\"\n[sources.b]\ntype = \"file\"\n\
             [transforms.pa]\ntype = \"remap\"\ninputs = [\"a\"]\n\
             [transforms.pb]\ntype = \"remap\"\ninputs = [\"b\"]\n\
             [sinks.out]\ntype = \"console\"\ninputs = [\"pa\", \"pb\"]\n",
        )];

        let focused = super::analyse_files_focused(&files, "config", Some("pa"));
        let ids: Vec<&str> = focused.components.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "pa", "out"]);
        assert_eq!(focused.edges.len(), 2, "pb -> out is not on a path through pa");
        assert_eq!(focused.layout.components.len(), 3);
        assert_eq!(focused.focus.as_deref(), Some("pa"));

        let unknown = super::analyse_files_focused(&files, "config", Some("gone"));
        assert_eq!(unknown.components.len(), 5, "a name that went away shows everything");
        assert_eq!(unknown.focus, None);
    }

    #[test]
    fn the_exported_problems_name_their_file() {
        let analysis = analyse_files(
            &[
                file("sources.toml", "[sources.app]\ntype = \"file\"\n"),
                file("sinks.toml", "[sinks.out]\ntype = \"console\"\ninputs = [\"nope\"]\n"),
            ],
            "config",
        );

        assert!(
            analysis.document.contains("`sinks.toml`, line 3"),
            "{}",
            analysis.document,
        );
    }

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
        assert_eq!(Format::of("vector.json"), Some(Format::Json));
        assert_eq!(Format::of("vector.ini"), None);
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
