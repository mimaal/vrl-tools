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
            .any(|message| message.contains("`enrich_*` matches no output of any component")),
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

/// A dot in a component's name is the first thing Vector rejects
/// (`check_names`), because a dot is how an input picks one output of a
/// component. The name is still resolved whole — that is the order
/// `Graph::input_map` uses — so the picture is drawn and the reason it cannot
/// run is said out loud rather than left to Vector.
#[test]
fn a_dotted_component_name_is_reported_and_still_drawn() {
    let graph = graph("broken/dotted-name.yaml");

    let edge = edge(&graph, "app.logs", "out");
    assert!(
        edge.output.is_none(),
        "the dot belongs to the name, not to an output",
    );
    assert!(
        messages(&graph)
            .iter()
            .any(|message| message.contains("a component cannot be called `app.logs`")),
        "{:?}",
        messages(&graph),
    );
}

/// Sources with named outputs, which no field of their own announces: only
/// their `type` does. Getting this wrong drew the one arrow Vector rejects and
/// refused the ones it accepts.
#[test]
fn a_source_with_ports_is_read_by_its_type() {
    let graph = graph("ports.yaml");

    assert_eq!(edge(&graph, "otel", "tag_logs").output.as_deref(), Some("logs"));
    assert_eq!(edge(&graph, "otel", "traces_out").output.as_deref(), Some("traces"));
    assert_eq!(edge(&graph, "dd", "metrics_out").output.as_deref(), Some("metrics"));
    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

/// `disable_llmobs` removes the output, so naming it is an error rather than a
/// path that happens to carry nothing.
#[test]
fn a_disabled_port_is_not_there_to_name() {
    let graph = inline(
        "sources:\n  dd:\n    type: datadog_agent\n    multiple_outputs: true\n    disable_llmobs: true\n\
         sinks:\n  out:\n    type: console\n    inputs: [dd.llmobs]\n",
    );

    assert!(
        messages(&graph)
            .iter()
            .any(|message| message.contains("`dd` has no output `llmobs`")),
        "{:?}",
        messages(&graph),
    );
}

/// An `opentelemetry` source has no default output, so its bare name resolves
/// to nothing in Vector. Saying which outputs it does have is the useful part.
#[test]
fn a_source_without_a_default_output_cannot_be_named_bare() {
    let graph = inline(
        "sources:\n  otel:\n    type: opentelemetry\n\
         sinks:\n  out:\n    type: console\n    inputs: [otel]\n",
    );

    assert!(graph.edges.is_empty(), "{:#?}", graph.edges);
    assert!(
        messages(&graph).iter().any(|message| {
            message.contains("`otel` has no default output") && message.contains("`otel.logs`")
        }),
        "{:?}",
        messages(&graph),
    );
}

/// A `memory` enrichment table is a sink and, under its `source_key`, a
/// source. Both halves are in the graph, and neither is a finding.
#[test]
fn an_enrichment_table_is_wired_into_the_pipeline() {
    let graph = graph("enrichment.yaml");

    edge(&graph, "app_logs", "seen");
    assert!(edge(&graph, "seen_export", "exported").output.is_none());
    assert_eq!(
        edge(&graph, "seen_export", "expired_out").output.as_deref(),
        Some("expired"),
    );
    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

/// VRL reaches a table by name. Wiring one in as an input is a different
/// mistake from naming a sink, and worth its own sentence.
#[test]
fn a_table_cannot_be_read_as_an_input() {
    let graph = inline(
        "sources:\n  app_logs:\n    type: file\n\
         enrichment_tables:\n  hosts:\n    type: file\n\
         sinks:\n  out:\n    type: console\n    inputs: [hosts]\n",
    );

    assert!(
        messages(&graph)
            .iter()
            .any(|message| message.contains("`hosts` is an enrichment table")),
        "{:?}",
        messages(&graph),
    );
}

/// The warning is per output, not per component: something does read `split`,
/// so it is not an orphan, but two of its three routes go nowhere.
#[test]
fn an_output_nobody_reads_is_a_warning_of_its_own() {
    let graph = graph("broken/unread-route.yaml");

    for output in ["split.warnings", "split._unmatched"] {
        let finding = graph
            .findings
            .iter()
            .find(|finding| finding.message.contains(&format!("nothing reads `{output}`")))
            .unwrap_or_else(|| panic!("{output}: {:?}", messages(&graph)));
        assert_eq!(finding.severity, Severity::Warning);
    }

    assert!(
        !messages(&graph)
            .iter()
            .any(|message| message.contains("nothing reads `split.errors`")),
        "{:?}",
        messages(&graph),
    );
}

/// `check_shape`: a transform or a sink with nothing feeding it stops Vector
/// starting. A table is exempt, because Vector never asks it for inputs.
#[test]
fn a_transform_or_sink_without_inputs_is_an_error() {
    let graph = graph("broken/no-inputs.yaml");

    for id in ["out", "drop_debug"] {
        assert!(
            messages(&graph)
                .iter()
                .any(|message| message.contains(&format!("`{id}` has no inputs"))),
            "{id}: {:?}",
            messages(&graph),
        );
    }

    assert!(
        !messages(&graph)
            .iter()
            .any(|message| message.contains("`hosts` has no inputs")),
        "a table nothing writes into is ordinary: {:?}",
        messages(&graph),
    );
}

/// Vector counts the repeats and refuses. The graph says so once, at the
/// repeat, and draws one arrow rather than two identical ones on top of each
/// other.
#[test]
fn an_input_named_twice_is_an_error_and_one_edge() {
    let graph = graph("broken/duplicate-input.toml");

    let repeats = messages(&graph)
        .into_iter()
        .filter(|message| message.contains("takes `parse_logs` more than once"))
        .count();
    assert_eq!(repeats, 1, "{:?}", messages(&graph));

    let drawn = graph
        .edges
        .iter()
        .filter(|edge| edge.from == "parse_logs" && edge.to == "out")
        .count();
    assert_eq!(drawn, 1, "{:#?}", graph.edges);
}

/// A pipeline events cannot leave. Vector: "No sinks defined in the config."
#[test]
fn a_pipeline_with_no_sinks_is_an_error() {
    let graph = graph("broken/no-sinks.yaml");

    assert!(
        messages(&graph).iter().any(|message| message.contains("no sinks")),
        "{:?}",
        messages(&graph),
    );
}

/// An empty file is not a shapeless pipeline, it is one nobody has started
/// writing. Vector's own answer to the same state is a command-line flag, not
/// a fact about the file.
#[test]
fn an_empty_config_is_not_a_shapeless_one() {
    let graph = inline("");

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
}

/// `wildcard_matching: relaxed` is the config saying a pattern may match
/// nothing. Reporting it anyway is a false positive on a config Vector runs.
#[test]
fn relaxed_wildcards_may_match_nothing() {
    let graph = graph("relaxed-wildcards.toml");

    assert!(graph.findings.is_empty(), "{:?}", messages(&graph));
    edge(&graph, "app_logs", "out");
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
