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

/// The two routers and output wildcards, as Vector resolves them.
#[test]
fn routers_and_output_wildcards_resolve_without_findings() {
    let graph = graph("routers.toml");

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));

    let alerts: Vec<(&str, Option<&str>)> = graph
        .edges
        .iter()
        .filter(|e| e.to == "alerts")
        .map(|e| (e.from.as_str(), e.output.as_deref()))
        .collect();
    assert_eq!(
        alerts,
        [("nginx_route", Some("errors")), ("api_route", Some("errors"))],
        "`*_route.errors` takes the errors output of both routers",
    );

    assert_eq!(edge(&graph, "by_tier", "gold").output.as_deref(), Some("gold"));
    assert_eq!(
        graph.edges.iter().filter(|e| e.to == "other").count(),
        2,
        "an exclusive_route's routes and its _unmatched",
    );
}

fn inline(source: &str) -> Graph {
    build(read_yaml(source).expect("parses"))
}

/// A router has no default output. Vector rejects its bare name; so does
/// this, and says which outputs there are.
#[test]
fn a_router_read_by_its_bare_name_is_an_error() {
    let graph = inline(
        "sources:\n  in:\n    type: file\n\
         transforms:\n  split:\n    type: route\n    inputs: [in]\n    route:\n      errors: '.x'\n\
         sinks:\n  out:\n    type: console\n    inputs: [split]\n",
    );

    let message = messages(&graph)
        .into_iter()
        .find(|m| m.contains("no default output"))
        .unwrap_or_else(|| panic!("{:?}", messages(&graph)));
    assert!(message.contains("`split.errors`") && message.contains("`split._unmatched`"), "{message}");
}

/// `*` reaches named outputs too: `strict*` takes `strict.dropped` along with
/// `strict`, as `glob::Pattern` would.
#[test]
fn a_wildcard_reaches_named_outputs() {
    let graph = inline(
        "sources:\n  in:\n    type: file\n\
         transforms:\n  strict:\n    type: remap\n    inputs: [in]\n    reroute_dropped: true\n\
         sinks:\n  out:\n    type: console\n    inputs: ['strict*']\n",
    );

    let outputs: Vec<Option<&str>> = graph
        .edges
        .iter()
        .filter(|e| e.to == "out")
        .map(|e| e.output.as_deref())
        .collect();
    assert_eq!(outputs, [None, Some("dropped")], "{:?}", graph.edges);
}

/// `?` and character classes are glob syntax Vector accepts.
#[test]
fn question_marks_and_classes_match_like_glob() {
    let graph = inline(
        "sources:\n  app1:\n    type: file\n  app2:\n    type: file\n  app10:\n    type: file\n\
         sinks:\n  one_digit:\n    type: console\n    inputs: ['app?']\n\
         \x20 only_one:\n    type: console\n    inputs: ['app[1]']\n\
         \x20 not_one:\n    type: console\n    inputs: ['app[!1]']\n",
    );

    let from = |sink: &str| -> Vec<&str> {
        graph.edges.iter().filter(|e| e.to == sink).map(|e| e.from.as_str()).collect()
    };
    assert_eq!(from("one_digit"), ["app1", "app2"]);
    assert_eq!(from("only_one"), ["app1"]);
    assert_eq!(from("not_one"), ["app2"]);
}

/// `exclusive_route` in YAML: a list of `{name, condition}`.
#[test]
fn an_exclusive_route_is_read_from_yaml() {
    let graph = inline(
        "sources:\n  in:\n    type: file\n\
         transforms:\n  tiers:\n    type: exclusive_route\n    inputs: [in]\n    routes:\n\
         \x20     - name: gold\n        condition: '.tier == \"gold\"'\n\
         sinks:\n  out:\n    type: console\n    inputs: [tiers.gold, tiers._unmatched]\n",
    );

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
    assert_eq!(graph.edges.iter().filter(|e| e.to == "out").count(), 2);
}
