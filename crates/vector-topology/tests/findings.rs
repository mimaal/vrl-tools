//! What the graph says about the configs in `test-corpus/topology/`.
//!
//! The valid ones have to produce edges and nothing else: a checker that cries
//! about working configurations gets turned off, and then it catches nothing.
//! The ones under `broken/` each have one thing wrong, and a check that stops
//! reporting its own is a regression these tests exist to catch.

use std::path::Path;

use vector_topology::{build, read_toml, read_yaml, Edge, Graph, Severity};

fn graph(name: &str) -> Graph {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-corpus/topology")
        .join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));

    let components = if name.ends_with(".toml") {
        read_toml(&source).expect("the corpus parses")
    } else {
        read_yaml(&source).expect("the corpus parses")
    };

    build(components)
}

fn messages(graph: &Graph) -> Vec<&str> {
    graph
        .findings
        .iter()
        .map(|finding| finding.message.as_str())
        .collect()
}

fn edge<'a>(graph: &'a Graph, from: &str, to: &str) -> &'a Edge {
    graph
        .edges
        .iter()
        .find(|edge| edge.from == from && edge.to == to)
        .unwrap_or_else(|| panic!("no edge {from} -> {to}; got {:#?}", graph.edges))
}

#[test]
fn a_working_config_produces_edges_and_no_findings() {
    let graph = graph("straight.yaml");

    assert_eq!(graph.edges.len(), 3, "{:#?}", graph.edges);
    assert!(edge(&graph, "app_logs", "parse").output.is_none());
    edge(&graph, "parse", "only_errors");
    edge(&graph, "only_errors", "out");

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

#[test]
fn an_edge_remembers_which_output_it_left_by() {
    let graph = graph("named-outputs.yaml");

    assert_eq!(
        edge(&graph, "split", "errors_out").output.as_deref(),
        Some("errors"),
    );
    assert_eq!(
        edge(&graph, "split", "leftovers").output.as_deref(),
        Some("_unmatched"),
    );
    assert_eq!(
        edge(&graph, "strict", "leftovers").output.as_deref(),
        Some("dropped"),
    );

    // The same transform feeding the same sink twice, by two different
    // outputs, is two edges. Collapsing them would lose the only interesting
    // thing about the pair.
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.to == "leftovers")
            .count(),
        2,
    );

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

#[test]
fn a_wildcard_becomes_one_edge_per_match() {
    let graph = graph("wildcards.toml");

    edge(&graph, "parse_json_logs", "out");
    edge(&graph, "parse_text_logs", "out");

    // `parse_*` must not have swept up the source as well.
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.from == "app_logs" && edge.to == "out"),
        "{:#?}",
        graph.edges,
    );

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

#[test]
fn an_input_naming_nothing_is_an_error_at_the_input() {
    let graph = graph("broken/dangling-input.yaml");

    let finding = graph
        .findings
        .iter()
        .find(|finding| finding.message.contains("no component is called `parse`"))
        .unwrap_or_else(|| panic!("{:?}", messages(&graph)));

    assert_eq!(finding.severity, Severity::Error);

    // Pointed at the input that is wrong, not at the sink that contains it.
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-corpus/topology/broken/dangling-input.yaml"),
    )
    .expect("the corpus reads");
    let line = source
        .lines()
        .nth(finding.range.start.line as usize)
        .expect("the range points inside the file");
    assert!(line.contains("- parse"), "line is {line:?}");
}

#[test]
fn a_component_nobody_reads_is_a_warning() {
    let graph = graph("broken/orphan.yaml");

    let finding = graph
        .findings
        .iter()
        .find(|finding| finding.message.contains("nothing reads `enrich`"))
        .unwrap_or_else(|| panic!("{:?}", messages(&graph)));

    // A warning, not an error: Vector starts on this and runs it. The events
    // are built and thrown away, which is worse to find and less urgent.
    assert_eq!(finding.severity, Severity::Warning);
}

#[test]
fn a_wildcard_matching_nothing_is_reported() {
    let graph = graph("broken/empty-wildcard.toml");

    assert!(
        messages(&graph)
            .iter()
            .any(|message| message.contains("`enrich_*` matches no source or transform")),
        "{:?}",
        messages(&graph),
    );

    // And the sink gained no edges from it, rather than quietly gaining all of
    // them.
    assert!(!graph.edges.iter().any(|edge| edge.to == "out"));
}

#[test]
fn a_loop_is_reported_for_every_component_in_it() {
    let graph = graph("broken/cycle.yaml");

    for id in ["a", "b"] {
        assert!(
            messages(&graph)
                .iter()
                .any(|message| message.contains(&format!("`{id}` is part of a loop"))),
            "{:?}",
            messages(&graph),
        );
    }
}

/// Resolution stops at the first reading that works, so a component whose name
/// contains a dot wins over splitting that name into an output.
#[test]
fn a_dotted_component_name_is_not_read_as_an_output() {
    let source = "
sources:
  app.logs:
    type: file
transforms:
  parse:
    type: remap
    inputs:
      - app.logs
sinks:
  out:
    type: console
    inputs:
      - parse
";
    let graph = build(read_yaml(source).expect("parses"));

    let edge = edge(&graph, "app.logs", "parse");
    assert!(
        edge.output.is_none(),
        "the dot belongs to the name, not to an output",
    );
    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

/// A sink is where events stop. Naming one as an input is not a typo the
/// graph should quietly draw.
#[test]
fn nothing_can_read_from_a_sink() {
    let source = "
sources:
  app_logs:
    type: file
sinks:
  first:
    type: console
    inputs:
      - app_logs
  second:
    type: console
    inputs:
      - first
";
    let graph = build(read_yaml(source).expect("parses"));

    assert!(
        messages(&graph)
            .iter()
            .any(|message| message.contains("`first` is a sink")),
        "{:?}",
        messages(&graph),
    );
}
