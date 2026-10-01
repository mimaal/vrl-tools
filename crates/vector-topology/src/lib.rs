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
pub mod lookups;
mod naming;
pub mod outputs;
pub mod render;
pub mod routes;
pub mod tables;
mod terminal;
mod vars;

pub use config::{
    read_toml, read_toml_enrichment_tables, read_yaml, read_yaml_enrichment_tables, Component,
    ConfigError, Document, Input, Role,
};
pub use graph::{build, focus, Edge, Finding, Graph, Mark, Severity, Unread};
pub use layout::{arrange, clusters, layout, Cluster, Layout, Placement, Route, Slot};
pub use render::{diagram, document, document_of_files};
pub use tables::{Lookup, Table};

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

    /// [`Self::read`], stopping short of putting the components together, so
    /// the pieces of one component in several files can be. See
    /// [`config::assemble`].
    fn entries(self, source: &str) -> Result<config::Entries, ConfigError> {
        match self {
            Self::Yaml | Self::Json => config::yaml_entries(source),
            Self::Toml => config::toml_entries(source),
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
    /// Every enrichment table of the pipeline and who reads it — the whole
    /// pipeline's, whatever the graph is narrowed to. See [`tables`].
    pub tables: Vec<Table>,
    /// The lookups drawn: a table in the layout, joined to a component that
    /// reads it. Empty unless [`Options::show_tables`].
    pub lookups: Vec<Lookup>,
    /// The VRL programs the pipeline's `remap`s read from files, and whether
    /// each was handed over to be read. One that was not leaves its
    /// component's lookups unknown rather than empty.
    pub programs: Vec<ProgramRef>,
    /// The components with a lookup whose table only the compiler can name.
    /// While there are any, "nothing reads this table" is not certain.
    pub opaque_lookups: Vec<String>,
    /// The outputs nothing reads that are marked as ending on purpose.
    pub terminal: Vec<String>,
    /// The outputs nothing reads that are not, with where a mark would go.
    pub unread: Vec<Unread>,
}

/// How to read a pipeline, beyond which files it is.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Narrow the graph to the paths through this component. See [`focus`].
    pub focus: Option<String>,
    /// Draw the enrichment tables the components on screen read, each joined
    /// to its readers. See [`tables`].
    pub show_tables: bool,
    /// The VRL programs the config names by path (`remap`'s `file` and
    /// `files`), read by the caller: this crate opens no files.
    pub programs: Vec<Program>,
    /// Outputs that end on purpose, as patterns over `component.output` or
    /// `component`, with the wildcards `inputs` takes. What a
    /// `# vrl-tools: terminal` comment says from inside the config.
    pub terminal_outputs: Vec<String>,
    /// A regular expression the names of the components of a type should
    /// match, by type: `{"remap": "-normalizer$"}`. A name that does not is
    /// an `info` finding. Empty, which is the default, checks nothing.
    pub component_name_pattern: std::collections::BTreeMap<String, String>,
}

/// A VRL program kept in a file of its own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Program {
    /// The path exactly as the config writes it, which is how it is matched.
    pub path: String,
    pub source: String,
}

/// A program a component reads from a file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRef {
    pub component: String,
    /// As written in the config.
    pub path: String,
    /// The config file that names it, for resolving a relative path.
    pub file: usize,
    /// Whether [`Options::programs`] had it.
    pub read: bool,
}

/// One file of a pipeline, as given to [`analyse_files`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ConfigFile {
    /// Chooses the parser, and names the file in findings. A path relative to
    /// the config directory reads best.
    pub name: String,
    pub source: String,
    /// Whether Vector is given this file with `--config`, which loads it on
    /// its own: nothing in it merges with another file. `false` for the files
    /// of a `--config-dir`, whose top-level files Vector merges, and for files
    /// whose loading is not known — merging is then the only reading in which
    /// a file of pieces works at all. See [`config::assemble`].
    #[serde(default)]
    pub standalone: bool,
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
    let drawing = tables::drawing(&graph, false);

    Ok(Analysis {
        document: document(&graph, title),
        layout: arrange(&graph, &drawing),
        tables: tables::tables(&graph),
        lookups: Vec::new(),
        programs: programs_of(&graph.components, &[]),
        opaque_lookups: opaque(&graph.components),
        terminal: graph.terminal,
        unread: graph.unread,
        components: graph.components,
        edges: graph.edges,
        findings: graph.findings,
        files: vec![title.to_owned()],
        unreadable: Vec::new(),
        focus: None,
    })
}

/// Every program the components read from a file, and whether it was given.
fn programs_of(components: &[Component], given: &[Program]) -> Vec<ProgramRef> {
    components
        .iter()
        .flat_map(|component| {
            component.programs.iter().map(move |path| ProgramRef {
                component: component.id.clone(),
                path: path.clone(),
                file: component.file,
                read: given.iter().any(|program| program.path == *path),
            })
        })
        .collect()
}

fn opaque(components: &[Component]) -> Vec<String> {
    components
        .iter()
        .filter(|component| component.opaque_lookups)
        .map(|component| component.id.clone())
        .collect()
}

/// Reads a pipeline split across several files, the way Vector reads the
/// files given to it with `--config` (or a glob such as `config/**/*.toml`)
/// and `--config-dir`. The components of all of them are one topology: an
/// input in one file can name a source in another, and a name declared in two
/// files is an error — unless one of them only adds to a component the other
/// declares, from the same directory, and neither is [`ConfigFile::standalone`],
/// which is how `--config-dir` merges its files. See [`config::assemble`].
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
    analyse_pipeline(
        files,
        title,
        &Options {
            focus: focus.map(ToOwned::to_owned),
            ..Options::default()
        },
    )
}

/// [`analyse_files`], read the way `options` says.
#[must_use]
pub fn analyse_pipeline(files: &[ConfigFile], title: &str, options: &Options) -> Analysis {
    let focus = options.focus.as_deref();
    let mut entries = Vec::new();
    let mut markers = Vec::new();
    let mut relaxed_wildcards = false;
    let mut unreadable = Vec::new();

    for (position, file) in files.iter().enumerate() {
        let read = match Format::of(&file.name) {
            Some(format) => format.entries(&file.source),
            None => Err(ConfigError {
                message: format!("{} is not a .yaml, .yml, .toml or .json file", file.name),
                range: None,
            }),
        };
        match read {
            Ok(read) => {
                let directory = directory(&file.name);
                entries.extend(
                    read.entries
                        .into_iter()
                        .map(|entry| entry.in_file(position, directory, file.standalone)),
                );
                markers.extend(read.markers.into_iter().map(|mut marker| {
                    marker.file = position;
                    marker
                }));
                // Vector merges the globals of every file it is given, so one
                // file relaxing wildcard matching relaxes it for the pipeline.
                // Erring towards relaxed keeps a setting this crate cannot see
                // the whole of from inventing errors.
                relaxed_wildcards |= read.relaxed_wildcards;
            }
            Err(error) => unreadable.push(Unreadable {
                file: position,
                message: error.message,
                range: error.range,
            }),
        }
    }

    let names: Vec<String> = files.iter().map(|file| file.name.clone()).collect();
    let (mut components, findings) = config::assemble(entries, &names, &markers);

    // What a program kept in a file looks up belongs to the component that
    // runs it, exactly as if it were written inline.
    for component in &mut components {
        for path in &component.programs {
            if let Some(program) = options.programs.iter().find(|program| program.path == *path) {
                let mut found = lookups::Found {
                    tables: std::mem::take(&mut component.lookups),
                    opaque: component.opaque_lookups,
                };
                found.merge(lookups::scan(&program.source));
                component.lookups = found.tables;
                component.opaque_lookups = found.opaque;
            }
        }
    }

    let whole = build(Document {
        components,
        relaxed_wildcards,
        findings,
        terminal: options.terminal_outputs.clone(),
        name_patterns: options
            .component_name_pattern
            .iter()
            .map(|(kind, pattern)| (kind.clone(), pattern.clone()))
            .collect(),
    });
    // About the whole pipeline, whatever is drawn of it: narrowing the graph
    // to one component does not make the other readers of a table go away.
    let all_tables = tables::tables(&whole);
    let programs = programs_of(&whole.components, &options.programs);
    let opaque_lookups = opaque(&whole.components);

    let (graph, focus) = match focus.and_then(|id| graph::focus(&whole, id).map(|g| (g, id))) {
        Some((mut narrowed, id)) => {
            // A table has no edges, so narrowing drops every one. The ones
            // the components left read come back, to be drawn if asked for.
            let read: Vec<&String> = narrowed.components.iter().flat_map(|c| &c.lookups).collect();
            let back: Vec<Component> = whole
                .components
                .iter()
                .filter(|component| component.is_lookup_table() && read.contains(&&component.id))
                .cloned()
                .collect();
            narrowed.components.extend(back);
            (narrowed, Some(id.to_owned()))
        }
        None => (whole, None),
    };
    let drawing = tables::drawing(&graph, options.show_tables);
    let layout = arrange(&graph, &drawing);

    Analysis {
        document: render::document_drawn(&graph, &drawing, title, &names),
        layout,
        tables: all_tables,
        lookups: drawing
            .lookups
            .iter()
            .map(|&(table, reader)| Lookup {
                table: graph.components[table].id.clone(),
                reader: graph.components[reader].id.clone(),
            })
            .collect(),
        programs,
        opaque_lookups,
        terminal: graph.terminal,
        unread: graph.unread,
        components: graph.components,
        edges: graph.edges,
        findings: graph.findings,
        files: names,
        unreadable,
        focus,
    }
}

/// The directory a file name is in, `""` at the top. Either separator, since
/// the name comes from whoever calls.
fn directory(name: &str) -> &str {
    name.rfind(['/', '\\']).map_or("", |end| &name[..end])
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

/// [`analyse_pipeline`], taking and giving JSON: the files as an array of
/// `{name, source, standalone}`, the options as [`Options`] in camelCase. An
/// empty `options_json` is the defaults.
#[must_use]
pub fn analyse_pipeline_json(files_json: &str, title: &str, options_json: &str) -> String {
    let files = match serde_json::from_str::<Vec<ConfigFile>>(files_json) {
        Ok(files) => files,
        Err(error) => return error_json(&format!("not a list of config files: {error}"), None),
    };
    let options = if options_json.trim().is_empty() {
        Options::default()
    } else {
        match serde_json::from_str::<Options>(options_json) {
            Ok(options) => options,
            Err(error) => return error_json(&format!("not the graph's options: {error}"), None),
        }
    };
    serde_json::to_string(&analyse_pipeline(&files, title, &options))
        .unwrap_or_else(|error| error_json(&error.to_string(), None))
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
    use super::{analyse_files, analyse_json, enrichment_tables, ConfigFile, Format, Severity};

    fn file(name: &str, source: &str) -> ConfigFile {
        ConfigFile {
            name: name.to_owned(),
            source: source.to_owned(),
            standalone: false,
        }
    }

    /// A file given with `--config`.
    fn alone(name: &str, source: &str) -> ConfigFile {
        ConfigFile {
            standalone: true,
            ..file(name, source)
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

    /// A router declared in one file, with each of the other files in the
    /// directory adding its own route: `--config-dir` merges them into one
    /// component, and so must the graph. Read as separate components, each
    /// piece was a second `route_by_product` with no type, no inputs and no
    /// outputs, and every route after the first vanished.
    #[test]
    fn a_component_written_across_files_of_one_directory_is_one_component() {
        let analysis = analyse_files(
            &[
                file(
                    "config/base.toml",
                    "[sources.input]\ntype = \"http_server\"\n\n\
                     [transforms.route_by_product]\ntype = \"exclusive_route\"\ninputs = [\"input\"]\n\n\
                     [sinks.dead]\ntype = \"blackhole\"\ninputs = [\"route_by_product._unmatched\"]\n",
                ),
                file(
                    "config/product_a.toml",
                    "[[transforms.route_by_product.routes]]\nname = \"product_a\"\ncondition = \"true\"\n\n\
                     [transforms.normalize_a]\ntype = \"remap\"\ninputs = [\"route_by_product.product_a\"]\nsource = \".\"\n",
                ),
                file(
                    "config/product_b.toml",
                    "[[transforms.route_by_product.routes]]\nname = \"product_b\"\ncondition = \"true\"\n\n\
                     [transforms.normalize_b]\ntype = \"remap\"\ninputs = [\"route_by_product.product_b\"]\nsource = \".\"\n",
                ),
                file(
                    "config/sinks.toml",
                    "[sinks.out]\ntype = \"console\"\ninputs = [\"normalize_*\"]\nencoding.codec = \"json\"\n",
                ),
            ],
            "config",
        );

        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
        let router: Vec<_> = analysis
            .components
            .iter()
            .filter(|c| c.id == "route_by_product")
            .collect();
        assert_eq!(router.len(), 1);
        assert_eq!(router[0].component_type, "exclusive_route");
        assert_eq!(router[0].named_outputs, ["product_a", "product_b", "_unmatched"]);
        assert_eq!(analysis.files[router[0].file], "config/base.toml", "placed where it is declared");
    }

    /// Vector merges within one `--config-dir`, not across two: there the
    /// piece is a component of its own, with no type, and a second name.
    #[test]
    fn pieces_in_another_directory_are_not_merged() {
        let analysis = analyse_files(
            &[
                file(
                    "a/base.toml",
                    "[transforms.split]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n",
                ),
                file(
                    "b/more.toml",
                    "[[transforms.split.routes]]\nname = \"x\"\ncondition = \"true\"\n",
                ),
            ],
            "config",
        );

        assert_eq!(analysis.components.iter().filter(|c| c.id == "split").count(), 2);
        assert!(
            analysis.findings.iter().any(|f| f.message.contains("2 components are called `split`")),
            "{:#?}",
            analysis.findings,
        );
        let untyped = analysis
            .findings
            .iter()
            .find(|f| f.message.contains("has no `type`"))
            .expect("the piece is reported");
        assert!(untyped.message.contains("another directory"), "{}", untyped.message);
        assert_eq!(analysis.files[untyped.file], "b/more.toml");
    }

    /// Given with `--config`, every file is loaded on its own, so a piece is a
    /// file Vector fails to load — even beside its declaration.
    #[test]
    fn pieces_given_with_config_are_not_merged() {
        let analysis = analyse_files(
            &[
                alone(
                    "config/base.toml",
                    "[sources.in]
type = \"stdin\"
[transforms.split]
type = \"exclusive_route\"
inputs = [\"in\"]
                     [[transforms.split.routes]]
name = \"a\"
condition = \"true\"
                     [sinks.out]
type = \"console\"
inputs = [\"split.*\"]
",
                ),
                alone(
                    "config/more.toml",
                    "[[transforms.split.routes]]
name = \"b\"
condition = \"true\"
",
                ),
            ],
            "config",
        );

        let router = analysis.components.iter().find(|c| c.file == 0 && c.id == "split").unwrap();
        assert_eq!(router.named_outputs, ["a", "_unmatched"], "the piece's route is not added");
        let untyped = analysis
            .findings
            .iter()
            .find(|f| f.message.contains("has no `type`"))
            .expect("the piece is reported");
        assert!(untyped.message.contains("--config"), "{}", untyped.message);
        assert_eq!(analysis.files[untyped.file], "config/more.toml");
    }

    #[test]
    fn a_component_no_file_gives_a_type_says_so() {
        let analysis = analyse_files(
            &[file("vector.yaml", "sources:
  app:
    path: /var/log
sinks:
  out:
    type: console
    inputs: [app]
")],
            "config",
        );

        let messages: Vec<&str> = analysis.findings.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(messages, ["`app` has no `type`, and Vector needs one to know what it is"]);
    }

    /// The base file of the table measured against `vector validate
    /// --config-dir` 0.55.0: a router with one route, read by a sink that
    /// takes a second route from the other file.
    const BASE: &str = "[sources.in]\ntype = \"demo_logs\"\nformat = \"json\"\n\
        [transforms.r]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n\
        [[transforms.r.routes]]\nname = \"one\"\ncondition = '.x == 1'\n\
        [sinks.out]\ntype = \"blackhole\"\ninputs = [\"r.one\", \"r.two\", \"r._unmatched\"]\n\
        buffer.max_events = ${N}\n";

    const ROUTE_TWO: &str = "[[transforms.r.routes]]\nname = \"two\"\ncondition = '.x == 2'\n";

    fn messages(analysis: &super::Analysis) -> Vec<&str> {
        analysis.findings.iter().map(|f| f.message.as_str()).collect()
    }

    /// Row one: `b.toml` only adds a route. Vector validates it.
    #[test]
    fn a_file_adding_a_route_validates() {
        let analysis = analyse_files(
            &[file("config/a.toml", BASE), file("config/b.toml", ROUTE_TWO)],
            "config",
        );
        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
    }

    /// Row two: `b.toml` repeats the declaration with the same `type`. Vector
    /// validates it: the files are one value before they are components, so
    /// there is no second `r` to be a duplicate of.
    #[test]
    fn a_declaration_repeated_with_the_same_type_is_one_component() {
        let analysis = analyse_files(
            &[
                file("config/a.toml", BASE),
                file(
                    "config/b.toml",
                    &format!("[transforms.r]\ntype = \"exclusive_route\"\n{ROUTE_TWO}"),
                ),
            ],
            "config",
        );

        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
        let r: Vec<_> = analysis.components.iter().filter(|c| c.id == "r").collect();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].named_outputs, ["one", "two", "_unmatched"]);
        assert_eq!(analysis.files[r[0].file], "config/a.toml", "placed at the first declaration");
    }

    /// Row three: `b.toml` says `route`. Vector keeps it and then fails on
    /// `routes`, which a `route` does not have. Exactly one finding names the
    /// conflict, at the file that wins it.
    #[test]
    fn a_declaration_repeated_with_another_type_names_the_conflict() {
        let analysis = analyse_files(
            &[
                file("config/a.toml", BASE),
                file("config/b.toml", &format!("[transforms.r]\ntype = \"route\"\n{ROUTE_TWO}")),
            ],
            "config",
        );

        let conflicts: Vec<_> = analysis
            .findings
            .iter()
            .filter(|f| f.message.contains(" in a.toml and "))
            .collect();
        assert_eq!(conflicts.len(), 1, "{:#?}", analysis.findings);
        assert_eq!(
            conflicts[0].message,
            "`r` is `exclusive_route` in a.toml and `route` in b.toml; Vector keeps b.toml's",
        );
        assert_eq!(conflicts[0].severity, Severity::Error);
        assert_eq!(analysis.files[conflicts[0].file], "config/b.toml");
        assert!(!messages(&analysis).iter().any(|m| m.contains("components are called")));
    }

    /// Any other field set twice: Vector runs, with the later value.
    #[test]
    fn a_field_set_differently_in_two_files_is_a_warning_naming_the_winner() {
        let analysis = analyse_files(
            &[
                file(
                    "config/a.toml",
                    "[sources.in]\ntype = \"stdin\"\n[sinks.out]\ntype = \"console\"\ninputs = [\"in\"]\nencoding.codec = \"json\"\n",
                ),
                file("config/b.toml", "[sinks.out]\ntype = \"console\"\nencoding.codec = \"text\"\n"),
            ],
            "config",
        );

        assert_eq!(
            messages(&analysis),
            ["`out` sets `encoding.codec` to `json` in a.toml and to `text` in b.toml; Vector keeps b.toml's"],
        );
        assert_eq!(analysis.findings[0].severity, Severity::Warning);
    }

    /// A string in one file and a table in the other is not a replacement
    /// but a refusal ("Incompatible types").
    #[test]
    fn values_of_different_kinds_are_an_error() {
        let analysis = analyse_files(
            &[
                file(
                    "config/a.toml",
                    "[sources.in]\ntype = \"stdin\"\ndecoding = \"json\"\n[sinks.out]\ntype = \"console\"\ninputs = [\"in\"]\n",
                ),
                file("config/b.toml", "[sources.in]\ndecoding.codec = \"json\"\n"),
            ],
            "config",
        );

        let found = analysis
            .findings
            .iter()
            .find(|f| f.message.contains("refuses to merge"))
            .expect("the incompatible values are reported");
        assert_eq!(found.severity, Severity::Error);
    }

    /// Across two directories Vector does not merge: two `--config-dir`s are
    /// appended, and a name they share is "duplicate id". Given with
    /// `--config`, likewise.
    #[test]
    fn declarations_that_vector_does_not_merge_are_still_duplicates() {
        let app = "[sources.app]\ntype = \"file\"\n";
        for (a, b) in [
            (file("prod/a.toml", app), file("staging/a.toml", app)),
            (alone("config/a.toml", app), alone("config/b.toml", app)),
        ] {
            let analysis = analyse_files(&[a, b], "config");
            assert_eq!(analysis.components.len(), 2);
            assert!(messages(&analysis).iter().any(|m| m.contains("called `app`")));
        }
    }

    /// The order the routes are tried in is the order of the files' names.
    #[test]
    fn routes_are_merged_in_file_name_order() {
        let router = "[sources.in]\ntype = \"stdin\"\n[transforms.r]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n\
                      [sinks.out]\ntype = \"console\"\ninputs = [\"r.*\"]\n";
        // Given out of order on purpose: the merge sorts, the caller need not.
        let analysis = analyse_files(
            &[
                file("config/topology.toml", router),
                file("config/product.toml", "[[transforms.r.routes]]\nname = \"product\"\ncondition = \"true\"\n"),
                file("config/00-overlay.toml", "[[transforms.r.routes]]\nname = \"overlay\"\ncondition = \"true\"\n"),
            ],
            "config",
        );

        let r = analysis.components.iter().find(|c| c.id == "r").unwrap();
        assert_eq!(r.named_outputs, ["overlay", "product", "_unmatched"]);
    }

    /// Each route remembers the file that added it; the router is still where
    /// its `type` is.
    #[test]
    fn a_route_remembers_the_file_that_added_it() {
        let analysis = analyse_files(
            &[file("config/a.toml", BASE), file("config/b.toml", ROUTE_TWO)],
            "config",
        );

        let r = analysis.components.iter().find(|c| c.id == "r").unwrap();
        let from: Vec<&str> = r
            .output_origins
            .iter()
            .map(|origin| analysis.files[origin.file].as_str())
            .collect();
        assert_eq!(from, ["config/a.toml", "config/b.toml", "config/a.toml"]);
        assert_eq!(r.output_origins[1].range.start.line, 1, "at the route's own name");
        assert_eq!(r.output_origins[2].range.start.line, 3, "`_unmatched` is written nowhere: at the router");
        assert_eq!(analysis.files[r.file], "config/a.toml");
        assert_eq!(r.pieces.len(), 2);
    }

    /// `inputs = [${X}, "nope"]`: the variable is something the editor cannot
    /// know, and is said so; `nope` is still an error.
    #[test]
    fn an_input_from_a_variable_is_information_not_an_error() {
        let analysis = analyse_files(
            &[file(
                "vector.toml",
                "[sources.in]\ntype = \"stdin\"\n[sinks.out]\ntype = \"console\"\ninputs = [${X}, \"nope\"]\n",
            )],
            "config",
        );

        let variable = analysis
            .findings
            .iter()
            .find(|f| f.message.contains("environment variable `X`"))
            .expect("the variable input is reported");
        assert_eq!(variable.severity, Severity::Info);
        assert_eq!(
            (variable.range.start.character, variable.range.end.character),
            (10, 14)
        );

        let nope = analysis.findings.iter().find(|f| f.message.contains("`nope`")).unwrap();
        assert_eq!(nope.severity, Severity::Error);
        assert!(analysis.edges.iter().all(|e| e.to != "out"), "no edge is drawn");
    }

    /// An input a piece adds is underlined in the piece's file, not in the
    /// file that declares the component.
    #[test]
    fn an_input_added_by_a_piece_points_at_the_piece() {
        let analysis = analyse_files(
            &[
                file("config/a.toml", "[sinks.out]\ntype = \"console\"\ninputs = []\n"),
                file("config/b.toml", "[sinks.out]\ninputs = [\"nope\"]\n"),
            ],
            "config",
        );

        let dangling = analysis
            .findings
            .iter()
            .find(|f| f.message.contains("`nope`"))
            .expect("the input names nothing");
        assert_eq!(analysis.files[dangling.file], "config/b.toml");
        assert_eq!(dangling.range.start.line, 1);
    }

    /// Vector interpolates before it parses, so an unquoted variable is a
    /// TOML value to it. Unread, the file lost every component it declares.
    #[test]
    fn an_unquoted_variable_does_not_make_a_file_unreadable() {
        let source = "[sinks.out]\ntype = \"http\"\ninputs = [\"in\"]\nbuffer.type = \"disk\"\nbuffer.max_size = ${BUFFER_SIZE_BYTES}\n\n\
                      [sources.in]\ntype = \"http_server\"\naddress = \"0.0.0.0:${PORT:-8080}\"\n";
        let analysis = analyse_files(&[file("config/base.toml", source)], "config");

        assert!(analysis.unreadable.is_empty(), "{:?}", analysis.unreadable);
        assert_eq!(analysis.components.len(), 2);
        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);

        // Positions are the file's as written, not the interpolated text's.
        let source_line = source.lines().position(|l| l == "[sources.in]").unwrap();
        let input = analysis.components.iter().find(|c| c.id == "in").unwrap();
        assert_eq!(input.range.start.line as usize, source_line);
    }

    #[test]
    fn a_variable_keeps_a_name_readable_on_both_ends_of_an_edge() {
        let analysis = analyse_files(
            &[file(
                "vector.yaml",
                "sources:\n  ${ENV}_app:\n    type: file\nsinks:\n  out:\n    type: console\n    inputs: [\"${ENV}_app\"]\n",
            )],
            "config",
        );

        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
        assert_eq!(analysis.edges[0].from, "${ENV}_app");
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

    /// Declared backwards and across files on purpose: the order handed on is
    /// the order events meet the components, whatever file each is in.
    #[test]
    fn components_come_in_the_order_events_meet_them() {
        let analysis = analyse_files(
            &[
                file(
                    "config/a-sinks.toml",
                    "[sinks.zeta]
type = \"console\"
inputs = [\"last\"]
                     [sinks.alpha]
type = \"console\"
inputs = [\"last\", \"beta\"]
                     [enrichment_tables.hosts]
type = \"file\"
",
                ),
                file(
                    "config/b-transforms.toml",
                    "[transforms.last]
type = \"remap\"
inputs = [\"middle\"]
                     [transforms.middle]
type = \"remap\"
inputs = [\"zz_first\"]
                     [transforms.zz_first]
type = \"remap\"
inputs = [\"in_b\", \"in_a\"]
                     [transforms.beta]
type = \"remap\"
inputs = [\"in_a\"]
",
                ),
                file(
                    "config/c-sources.toml",
                    "[sources.in_b]
type = \"stdin\"
[sources.in_a]
type = \"stdin\"
",
                ),
            ],
            "config",
        );

        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
        let ids: Vec<&str> = analysis.components.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            // Sources by name; `beta` and `zz_first` are both ready at once
            // and go by name, then the chain in the only order it has; sinks
            // by name; the table last.
            ["in_a", "in_b", "beta", "zz_first", "middle", "last", "alpha", "zeta", "hosts"],
        );

        let edges: Vec<String> =
            analysis.edges.iter().map(|edge| format!("{}>{}", edge.from, edge.to)).collect();
        assert_eq!(
            edges,
            [
                "in_a>beta",
                "in_a>zz_first",
                "in_b>zz_first",
                "beta>alpha",
                "zz_first>middle",
                "middle>last",
                "last>alpha",
                "last>zeta",
            ],
        );

        // The diagram declares its nodes in that order too.
        let nodes: Vec<&str> = analysis
            .document
            .lines()
            .filter(|line| line.starts_with("  n") && !line.contains("-->"))
            .map(|line| line.split('"').nth(1).and_then(|label| label.split('<').next()).unwrap_or(""))
            .collect();
        assert_eq!(nodes, &ids[..8], "the table is not drawn");
    }

    /// A loop has no order. It is reported as one, and its components still
    /// come out, once each, after everything that has a place.
    #[test]
    fn a_loop_does_not_lose_or_repeat_a_component() {
        let analysis = analyse_files(
            &[file(
                "vector.toml",
                "[sources.in]
type = \"stdin\"
                 [transforms.b]
type = \"remap\"
inputs = [\"a\"]
                 [transforms.a]
type = \"remap\"
inputs = [\"b\", \"in\"]
                 [transforms.ok]
type = \"remap\"
inputs = [\"in\"]
                 [sinks.out]
type = \"console\"
inputs = [\"a\", \"ok\"]
",
            )],
            "config",
        );

        let ids: Vec<&str> = analysis.components.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["in", "ok", "a", "b", "out"]);
    }

    /// A router with two routes nothing reads, an `_unmatched` nothing reads,
    /// and a remap whose output nothing reads: four warnings, as Vector gives.
    fn dead_ends(router: &str, routes: &str, counter: &str) -> String {
        format!(
            "[sources.in]\ntype = \"stdin\"\n\n\
             {router}\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n\n\
             {routes}\n\
             {counter}\ntype = \"remap\"\ninputs = [\"in\"]\nsource = \".n = 1\"\n\n\
             [sinks.out]\ntype = \"console\"\ninputs = [\"in\"]\n"
        )
    }

    const ROUTES: &str = "[[transforms.r.routes]]\nname = \"a\"\ncondition = \".x == 1\"\n\n\
                          [[transforms.r.routes]]\nname = \"b\"\ncondition = \".x == 2\"\n";

    fn unread(analysis: &super::Analysis) -> Vec<&str> {
        analysis.unread.iter().map(|unread| unread.output.as_str()).collect()
    }

    fn one(source: &str) -> super::Analysis {
        analyse_files(&[file("vector.toml", source)], "config")
    }

    #[test]
    fn unmarked_every_unread_output_is_a_warning_with_a_place_for_its_mark() {
        let source = dead_ends("[transforms.r]", ROUTES, "[transforms.count]");
        let analysis = one(&source);

        assert_eq!(unread(&analysis), ["r.a", "r.b", "r._unmatched", "count"]);
        assert!(analysis.terminal.is_empty());
        assert_eq!(analysis.findings.len(), 4);
        for entry in &analysis.unread {
            let finding = &analysis.findings[entry.finding];
            assert_eq!(finding.severity, Severity::Warning);
            assert!(finding.message.contains(&format!("`{}`", entry.output)), "{}", finding.message);
        }

        let line = |text: &str| source.lines().position(|line| line == text).unwrap() as u32;
        let mark = |output: &str| {
            let mark = &analysis.unread.iter().find(|u| u.output == output).unwrap().mark;
            (mark.line, mark.name.clone())
        };
        // A route has a line of its own, and a short one: the mark goes there.
        assert_eq!(mark("r.a"), (line("name = \"a\""), None));
        assert_eq!(mark("r.b"), (line("name = \"b\""), None));
        // `_unmatched` is written nowhere, so it is named on the router's line.
        assert_eq!(mark("r._unmatched"), (line("[transforms.r]"), Some("_unmatched".to_owned())));
        // The only output of a component is the component.
        assert_eq!(mark("count"), (line("[transforms.count]"), None));
    }

    #[test]
    fn a_comment_on_a_components_line_ends_all_its_outputs() {
        let analysis = one(&dead_ends(
            "[transforms.r] # vrl-tools: terminal",
            ROUTES,
            "[transforms.count]  # vrl-tools: terminal",
        ));

        assert!(analysis.findings.is_empty(), "{:#?}", analysis.findings);
        assert_eq!(analysis.terminal, ["r.a", "r.b", "r._unmatched", "count"]);
        assert!(analysis.unread.is_empty());
    }

    /// Beside a route, the comment is about that route — on its `name`, or on
    /// the header above it — and the rest of the router is still reported.
    #[test]
    fn a_comment_on_a_routes_line_ends_that_route_alone() {
        for routes in [
            ROUTES.replace("name = \"a\"", "name = \"a\" # vrl-tools: terminal"),
            ROUTES.replacen("[[transforms.r.routes]]", "[[transforms.r.routes]] # vrl-tools: terminal", 1),
        ] {
            let analysis = one(&dead_ends("[transforms.r]", &routes, "[transforms.count]"));
            assert_eq!(analysis.terminal, ["r.a"], "{routes}");
            assert_eq!(unread(&analysis), ["r.b", "r._unmatched", "count"], "{routes}");
        }
    }

    /// An output nobody writes down is named after the marker.
    #[test]
    fn a_named_marker_ends_only_what_it_names() {
        let analysis = one(&dead_ends(
            "[transforms.r] # vrl-tools: terminal _unmatched, b",
            ROUTES,
            "[transforms.count]",
        ));

        assert_eq!(analysis.terminal, ["r.b", "r._unmatched"]);
        assert_eq!(unread(&analysis), ["r.a", "count"]);
    }

    /// The component's own name stands for its default output, which is how
    /// a remap's is told from its `dropped`.
    #[test]
    fn a_components_own_name_marks_its_default_output() {
        let source = "[sources.in]\ntype = \"stdin\"\n\
                      [transforms.parse] # vrl-tools: terminal parse\ntype = \"remap\"\ninputs = [\"in\"]\n\
                      reroute_dropped = true\nsource = \".\"\n\
                      [sinks.out]\ntype = \"console\"\ninputs = [\"in\"]\n";
        let analysis = one(source);

        assert_eq!(analysis.terminal, ["parse"]);
        assert_eq!(unread(&analysis), ["parse.dropped"]);

        let unmarked = one(&source.replace(" # vrl-tools: terminal parse", ""));
        let mark = &unmarked.unread.iter().find(|u| u.output == "parse").unwrap().mark;
        assert_eq!(mark.name.as_deref(), Some("parse"), "bare would end `dropped` too");
    }

    #[test]
    fn the_marker_is_read_from_yaml_too() {
        let analysis = analyse_files(
            &[file(
                "vector.yaml",
                "sources:\n  in:\n    type: stdin\n\
                 transforms:\n  split: # vrl-tools: terminal _unmatched\n    type: route\n    inputs: [in]\n\
                 \x20   route:\n      errors: '.level == \"error\"' # vrl-tools: terminal\n      warns: '.level == \"warn\"'\n\
                 sinks:\n  out:\n    type: console\n    inputs: [in]\n",
            )],
            "config",
        );

        assert_eq!(analysis.terminal, ["split.errors", "split._unmatched"]);
        assert_eq!(unread(&analysis), ["split.warns"]);
        // A `route` condition can run over several lines, so its mark is not
        // offered on its own line: it goes on the router's, named.
        let mark = &analysis.unread[0].mark;
        assert_eq!((mark.line, mark.name.as_deref()), (4, Some("warns")));
    }

    /// A router written across files: the comment on a file's piece ends the
    /// routes that file adds.
    #[test]
    fn a_comment_on_a_piece_ends_the_routes_that_piece_adds() {
        let analysis = analyse_files(
            &[
                file(
                    "config/a.toml",
                    "[sources.in]\ntype = \"stdin\"\n[transforms.r]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n\
                     [sinks.out]\ntype = \"console\"\ninputs = [\"r._unmatched\"]\n",
                ),
                file(
                    "config/b.yaml",
                    "transforms:\n  r: # vrl-tools: terminal\n    routes:\n      - name: one\n        condition: 'true'\n      - name: two\n        condition: 'true'\n",
                ),
                file("config/c.toml", "[[transforms.r.routes]]\nname = \"three\"\ncondition = \"true\"\n"),
            ],
            "config",
        );

        assert_eq!(analysis.terminal, ["r.one", "r.two"]);
        assert_eq!(unread(&analysis), ["r.three"]);
        let mark = &analysis.unread[0].mark;
        assert_eq!((analysis.files[mark.file].as_str(), mark.line), ("config/c.toml", 1));
    }

    #[test]
    fn patterns_from_outside_end_outputs_and_whole_components() {
        let files = [file("vector.toml", &dead_ends("[transforms.r]", ROUTES, "[transforms.count]"))];
        let with = |patterns: &str| {
            super::analyse_pipeline(
                &files,
                "config",
                &options(&format!(r#"{{"terminalOutputs":{patterns}}}"#)),
            )
        };

        let exact = with(r#"["r._unmatched", "count"]"#);
        assert_eq!(exact.terminal, ["r._unmatched", "count"]);
        assert_eq!(unread(&exact), ["r.a", "r.b"]);

        // The same wildcards `inputs` takes, against the output as written.
        assert_eq!(with(r#"["r.[ab]"]"#).terminal, ["r.a", "r.b"]);
        assert_eq!(with(r#"["*._unmatched"]"#).terminal, ["r._unmatched"]);
        // A component's name ends every output it has.
        assert_eq!(with(r#"["r"]"#).terminal, ["r.a", "r.b", "r._unmatched"]);
        // Naming something that is not there, or nothing, changes nothing.
        assert_eq!(with(r#"["nope", ""]"#).findings.len(), 4);
    }

    /// The mark is about an output nothing reads. On one that is read it is
    /// not an end, and nothing is drawn as one.
    #[test]
    fn a_mark_on_an_output_something_reads_changes_nothing() {
        let analysis = one(
            "[sources.in]\ntype = \"stdin\"\n[transforms.parse] # vrl-tools: terminal\ntype = \"remap\"\ninputs = [\"in\"]\n\
             [sinks.out]\ntype = \"console\"\ninputs = [\"parse\"]\n",
        );
        assert!(analysis.terminal.is_empty());
        assert!(analysis.findings.is_empty());
    }

    /// A comment that lands on nothing, or names nothing, says so. One that
    /// silently did nothing would look exactly like one that worked.
    #[test]
    fn a_marker_that_marks_nothing_says_so() {
        let source = dead_ends(
            "[transforms.r] # vrl-tools: terminal unmatched",
            ROUTES,
            "[transforms.count]\n# vrl-tools: terminal",
        );
        let analysis = one(&source);

        let notes: Vec<&str> = analysis
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(notes.len(), 2, "{:#?}", analysis.findings);
        assert!(notes[0].contains("`unmatched` is not an output of `r`"), "{}", notes[0]);
        assert!(notes[0].contains("`_unmatched`"), "it says what there is: {}", notes[0]);
        assert!(notes[1].contains("not on the line of a component or of a route"), "{}", notes[1]);
        assert!(analysis.terminal.is_empty());

        let stray = analysis.findings.iter().find(|f| f.message.contains("not on the line")).unwrap();
        let line = source.lines().position(|line| line == "# vrl-tools: terminal").unwrap();
        assert_eq!(stray.range.start.line as usize, line);
    }

    fn router(routes: &str) -> String {
        format!(
            "[sources.in]\ntype = \"stdin\"\n[transforms.r]\ntype = \"exclusive_route\"\ninputs = [\"in\"]\n\
             {routes}[sinks.out]\ntype = \"console\"\ninputs = [\"r.*\"]\n"
        )
    }

    /// The broader route first: the narrower one after it gets nothing, and
    /// the warning is on the route that is dead.
    #[test]
    fn a_route_hidden_by_an_earlier_one_is_a_warning_at_the_hidden_route() {
        let source = router(
            "[[transforms.r.routes]]\nname = \"acme\"\ncondition = '.vendor == \"acme\"'\n\
             [[transforms.r.routes]]\nname = \"acme-fw\"\ncondition = '.vendor == \"acme\" && .kind == \"fw\"'\n",
        );
        let analysis = analyse_files(&[file("vector.toml", &source)], "config");

        assert_eq!(
            messages(&analysis),
            ["`r.acme-fw` can never match: `r.acme` is tried first, and its condition holds whenever this one does"],
        );
        assert_eq!(analysis.findings[0].severity, Severity::Warning);
        let line = source.lines().position(|line| line == "name = \"acme-fw\"").unwrap();
        assert_eq!(analysis.findings[0].range.start.line as usize, line);
    }

    /// Written the right way round, and for conditions the check does not
    /// read, nothing is said.
    #[test]
    fn routes_in_a_working_order_and_conditions_not_read_say_nothing() {
        for routes in [
            "[[transforms.r.routes]]\nname = \"acme-fw\"\ncondition = '.vendor == \"acme\" && .kind == \"fw\"'\n\
             [[transforms.r.routes]]\nname = \"acme\"\ncondition = '.vendor == \"acme\"'\n",
            "[[transforms.r.routes]]\nname = \"a\"\ncondition = 'exists(.vendor)'\n\
             [[transforms.r.routes]]\nname = \"b\"\ncondition = 'exists(.vendor) && .kind == \"fw\"'\n",
            "[[transforms.r.routes]]\nname = \"a\"\ncondition = { type = \"is_log\" }\n\
             [[transforms.r.routes]]\nname = \"b\"\ncondition = { type = \"is_log\" }\n",
        ] {
            let analysis = analyse_files(&[file("vector.toml", &router(routes))], "config");
            assert!(analysis.findings.is_empty(), "{routes}: {:#?}", analysis.findings);
        }
    }

    /// The `type: vrl` spelling of a condition is the same condition.
    #[test]
    fn a_fully_specified_vrl_condition_is_read_too() {
        let analysis = analyse_files(
            &[file(
                "vector.yaml",
                "sources:\n  in:\n    type: stdin\ntransforms:\n  r:\n    type: exclusive_route\n    inputs: [in]\n    routes:\n\
                 \x20     - name: all\n        condition:\n          type: vrl\n          source: '.x == 1'\n\
                 \x20     - name: some\n        condition: '.x == 1 && .y == 2'\n\
                 sinks:\n  out:\n    type: console\n    inputs: ['r.*']\n",
            )],
            "config",
        );
        assert_eq!(analysis.findings.len(), 1, "{:#?}", analysis.findings);
        assert!(analysis.findings[0].message.starts_with("`r.some` can never match: `r.all`"));
    }

    /// Across files the order is the files' — and that is what goes wrong:
    /// an overlay named to sort first, with the broad condition.
    #[test]
    fn a_route_hidden_by_another_files_says_which_file_comes_first() {
        let analysis = analyse_files(
            &[
                file("config/topology.toml", &router("")),
                file(
                    "config/product.toml",
                    "[[transforms.r.routes]]\nname = \"fw\"\ncondition = '.kind == \"fw\" && .vendor == \"acme\"'\n",
                ),
                file(
                    "config/00-overlay.toml",
                    "[[transforms.r.routes]]\nname = \"overlay\"\ncondition = '.vendor == \"acme\"'\n",
                ),
            ],
            "config",
        );

        assert_eq!(
            messages(&analysis),
            ["`r.fw` can never match: `r.overlay` is tried first, because 00-overlay.toml merges before \
              product.toml, and its condition holds whenever this one does"],
        );
        assert_eq!(analysis.files[analysis.findings[0].file], "config/product.toml");
    }

    const NAMED: &str = "[sources.in]\ntype = \"stdin\"\n\
        [transforms.acme_fw-normalizer]\ntype = \"remap\"\ninputs = [\"in\"]\nsource = \".\"\n\
        [transforms.fixup]\ntype = \"remap\"\ninputs = [\"in\"]\nsource = \".\"\n\
        [transforms.keep]\ntype = \"filter\"\ninputs = [\"in\"]\ncondition = \"true\"\n\
        [sinks.out]\ntype = \"console\"\ninputs = [\"*-normalizer\", \"fixup\", \"keep\"]\n";

    fn named(patterns: &str) -> super::Analysis {
        super::analyse_pipeline(
            &[file("vector.toml", NAMED)],
            "config",
            &options(&format!(r#"{{"componentNamePattern":{patterns}}}"#)),
        )
    }

    /// Off unless asked for: a naming convention is the team's, not Vector's.
    #[test]
    fn names_are_not_checked_unless_a_pattern_is_given() {
        assert!(analyse_files(&[file("vector.toml", NAMED)], "config").findings.is_empty());
        assert!(named("{}").findings.is_empty());
    }

    /// The components of the type the pattern is for, and only those; a note,
    /// at the component, since Vector runs it whatever it is called.
    #[test]
    fn a_name_that_breaks_the_pattern_for_its_type_is_a_note() {
        let analysis = named(r#"{"remap": "^[a-z0-9_]+-normalizer$"}"#);

        assert_eq!(
            messages(&analysis),
            ["`fixup` does not match the name pattern for `remap` components, `^[a-z0-9_]+-normalizer$`"],
        );
        let finding = &analysis.findings[0];
        assert_eq!(finding.severity, Severity::Info);
        let fixup = analysis.components.iter().find(|c| c.id == "fixup").unwrap();
        assert_eq!((finding.file, finding.range), (fixup.file, fixup.range));
    }

    #[test]
    fn each_type_has_its_own_pattern() {
        let analysis = named(r#"{"remap": "normalizer$", "filter": "^only-", "console": "^out$"}"#);
        assert_eq!(
            messages(&analysis),
            [
                "`fixup` does not match the name pattern for `remap` components, `normalizer$`",
                "`keep` does not match the name pattern for `filter` components, `^only-`",
            ],
        );
    }

    /// A pattern that is not a regular expression is said once, rather than
    /// matching nothing and looking like a convention everybody follows.
    #[test]
    fn a_pattern_that_does_not_compile_says_so_once() {
        let analysis = named(r#"{"remap": "(-normalizer"}"#);

        assert_eq!(analysis.findings.len(), 1, "{:#?}", analysis.findings);
        assert_eq!(analysis.findings[0].severity, Severity::Info);
        assert!(
            analysis.findings[0]
                .message
                .starts_with("the name pattern for `remap` components, `(-normalizer`, is not a regular expression"),
            "{}",
            analysis.findings[0].message,
        );
    }

    const LOOKUPS: &str = "[sources.in]\ntype = \"stdin\"\n\
        [transforms.inline]\ntype = \"remap\"\ninputs = [\"in\"]\n\
        source = '.a = get_enrichment_table_record!(\"hosts\", {})'\n\
        [transforms.filed]\ntype = \"remap\"\ninputs = [\"in\"]\nfile = \"programs/filed.vrl\"\n\
        [sinks.out]\ntype = \"console\"\ninputs = [\"inline\", \"filed\"]\n\
        [enrichment_tables.hosts]\ntype = \"file\"\nfile.path = \"hosts.csv\"\n\
        [enrichment_tables.geo]\ntype = \"file\"\nfile.path = \"geo.csv\"\n\
        [enrichment_tables.spare]\ntype = \"file\"\nfile.path = \"spare.csv\"\n";

    fn options(json: &str) -> super::Options {
        serde_json::from_str(json).expect("the options parse")
    }

    /// The tables stay in the list — the grouping, the summary and the
    /// sidebar count them — and leave the drawing.
    #[test]
    fn tables_are_listed_and_not_placed() {
        let analysis = analyse_files(&[file("config/a.toml", LOOKUPS)], "config");

        assert_eq!(analysis.components.len(), 7);
        let placed: Vec<&str> = analysis
            .layout
            .components
            .iter()
            .map(|p| analysis.components[p.component].id.as_str())
            .collect();
        assert_eq!(placed, ["in", "filed", "inline", "out"]);
        assert!(analysis.findings.is_empty(), "an unread table is not a finding");
    }

    /// `file =` is where a long program lives. Until somebody hands the file
    /// over, what it reads is unknown, and the analysis says which file.
    #[test]
    fn a_program_in_a_file_is_asked_for_and_then_read() {
        let files = [file("config/a.toml", LOOKUPS)];
        let before = analyse_files(&files, "config");
        let readers = |analysis: &super::Analysis, id: &str| {
            analysis.tables.iter().find(|t| t.id == id).expect(id).readers.clone()
        };

        assert_eq!(before.programs.len(), 1);
        assert_eq!(before.programs[0].component, "filed");
        assert_eq!(before.programs[0].path, "programs/filed.vrl");
        assert!(!before.programs[0].read);
        assert_eq!(readers(&before, "hosts"), ["inline"]);
        assert!(readers(&before, "geo").is_empty());

        let after = super::analyse_pipeline(
            &files,
            "config",
            &options(
                r#"{"programs":[{"path":"programs/filed.vrl","source":".g = find_enrichment_table_records!(table: \"geo\", condition: {})\n.h = get_enrichment_table_record!(\"hosts\", {})"}]}"#,
            ),
        );
        assert!(after.programs[0].read);
        assert_eq!(readers(&after, "hosts"), ["filed", "inline"]);
        assert_eq!(readers(&after, "geo"), ["filed"]);
        assert!(readers(&after, "spare").is_empty());
    }

    /// Asked for, only the tables somebody on screen reads are drawn.
    #[test]
    fn showing_tables_draws_the_ones_read_by_what_is_on_screen() {
        let files = [file("config/a.toml", LOOKUPS)];
        let placed = |analysis: &super::Analysis| -> Vec<String> {
            analysis
                .layout
                .components
                .iter()
                .map(|p| analysis.components[p.component].id.clone())
                .collect()
        };

        let shown = super::analyse_pipeline(&files, "config", &options(r#"{"showTables":true}"#));
        assert_eq!(placed(&shown), ["in", "filed", "inline", "out", "hosts"]);
        assert_eq!(shown.lookups.len(), 1);
        assert_eq!((shown.lookups[0].table.as_str(), shown.lookups[0].reader.as_str()), ("hosts", "inline"));

        // Narrowed to the remap that reads nothing, no table is on screen —
        // and the list of tables is still the whole pipeline's.
        let narrowed = super::analyse_pipeline(
            &files,
            "config",
            &options(r#"{"showTables":true,"focus":"filed"}"#),
        );
        assert_eq!(placed(&narrowed), ["in", "filed", "out"]);
        assert_eq!(narrowed.tables.len(), 3);

        let on_reader = super::analyse_pipeline(
            &files,
            "config",
            &options(r#"{"showTables":true,"focus":"inline"}"#),
        );
        assert_eq!(placed(&on_reader), ["in", "inline", "out", "hosts"]);
    }

    /// A table named through a variable validates in Vector, so its reader
    /// is named rather than the table called unused.
    #[test]
    fn a_lookup_the_editor_cannot_follow_is_said() {
        let analysis = analyse_files(
            &[file(
                "vector.toml",
                "[sources.in]\ntype = \"stdin\"\n[transforms.r]\ntype = \"remap\"\ninputs = [\"in\"]\n\
                 source = \"\"\"\nname = \\\"t\\\"\n.x = get_enrichment_table_record!(name, {})\n\"\"\"\n\
                 [sinks.out]\ntype = \"console\"\ninputs = [\"r\"]\n[enrichment_tables.t]\ntype = \"file\"\n",
            )],
            "config",
        );

        assert_eq!(analysis.opaque_lookups, ["r"]);
        assert!(analysis.tables[0].readers.is_empty());
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
