//! Reads the configs in `test-corpus/topology/` the way the extension will.
//!
//! The unit tests next to the code check rules in isolation. What only this
//! can check is that the rules survive contact with a file somebody would
//! actually write, in both formats, with the indentation and the block scalars
//! and the dotted keys that come with them.
//!
//! These configs are synthetic, because this repository holds no third-party
//! pipelines. They are written against the published configuration reference;
//! there is no `vector` binary here to confirm them, so if you have one,
//! `vector validate` over the valid ones is the honest cross-check.

use std::path::{Path, PathBuf};

use vector_topology::{read_toml, read_yaml, Component, Role};

fn corpus(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-corpus/topology")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn read(name: &str) -> Vec<Component> {
    if name.ends_with(".toml") {
        read_toml(&corpus(name)).expect("the corpus parses as TOML").components
    } else {
        read_yaml(&corpus(name)).expect("the corpus parses as YAML").components
    }
}

fn find<'a>(components: &'a [Component], id: &str) -> &'a Component {
    components
        .iter()
        .find(|component| component.id == id)
        .unwrap_or_else(|| {
            let ids: Vec<_> = components.iter().map(|c| c.id.as_str()).collect();
            panic!("no component {id}; got {ids:?}")
        })
}

fn inputs(component: &Component) -> Vec<&str> {
    component
        .inputs
        .iter()
        .map(|input| input.text.as_str())
        .collect()
}

#[test]
fn the_simple_shape_reads_end_to_end() {
    let components = read("straight.yaml");
    assert_eq!(components.len(), 4, "{components:#?}");

    let source = find(&components, "app_logs");
    assert_eq!(source.role, Role::Source);
    assert_eq!(source.component_type, "file");
    // A source is fed by the outside world, so it has no inputs. That is the
    // rule an orphan check leans on, so it is worth stating.
    assert!(source.inputs.is_empty());

    assert_eq!(find(&components, "parse").role, Role::Transform);
    assert_eq!(inputs(find(&components, "parse")), ["app_logs"]);
    assert_eq!(inputs(find(&components, "only_errors")), ["parse"]);

    let sink = find(&components, "out");
    assert_eq!(sink.role, Role::Sink);
    assert_eq!(sink.component_type, "console");
    assert_eq!(inputs(sink), ["only_errors"]);
}

#[test]
fn a_route_declares_an_output_per_route_plus_unmatched() {
    let components = read("named-outputs.yaml");

    let split = find(&components, "split");
    assert_eq!(split.component_type, "route");
    assert_eq!(split.named_outputs, ["errors", "warnings", "_unmatched"]);
}

#[test]
fn reroute_dropped_declares_the_dropped_output() {
    let components = read("named-outputs.yaml");

    let strict = find(&components, "strict");
    assert_eq!(strict.component_type, "remap");
    assert_eq!(strict.named_outputs, ["dropped"]);

    // And a plain transform in the same file declares nothing, so the flag is
    // what did it rather than the file.
    assert!(find(&components, "split").named_outputs.contains(&"errors".to_owned()));
}

#[test]
fn an_input_can_name_an_output_rather_than_a_component() {
    let components = read("named-outputs.yaml");

    assert_eq!(inputs(find(&components, "errors_out")), ["split.errors"]);
    assert_eq!(
        inputs(find(&components, "leftovers")),
        ["split._unmatched", "strict.dropped"],
        "a sink can gather several named outputs",
    );
}

#[test]
fn toml_reads_the_same_shapes() {
    let components = read("wildcards.toml");
    assert_eq!(components.len(), 4, "{components:#?}");

    assert_eq!(find(&components, "app_logs").role, Role::Source);
    assert_eq!(inputs(find(&components, "parse_json_logs")), ["app_logs"]);
    assert_eq!(inputs(find(&components, "parse_text_logs")), ["app_logs"]);

    // Kept as written. Expanding it is the resolver's job, and an input that
    // has already been expanded cannot be underlined when it matches nothing.
    assert_eq!(inputs(find(&components, "out")), ["parse_*"]);
}

/// Positions are the reason neither parser here is the serde one, so they are
/// checked rather than assumed.
#[test]
fn a_component_knows_where_it_is_declared() {
    let yaml = corpus("straight.yaml");
    let components = read("straight.yaml");
    let parse = find(&components, "parse");

    let line = yaml
        .lines()
        .nth(parse.range.start.line as usize)
        .expect("the range points inside the file");
    assert!(
        line.contains("parse:"),
        "line {} is {line:?}",
        parse.range.start.line,
    );
}

/// The same, in TOML, where the component name is part of a dotted header
/// rather than a key of its own.
#[test]
fn a_toml_component_knows_where_it_is_declared() {
    let toml = corpus("wildcards.toml");
    let components = read("wildcards.toml");
    let parse = find(&components, "parse_json_logs");

    let line = toml
        .lines()
        .nth(parse.range.start.line as usize)
        .expect("the range points inside the file");
    assert!(
        line.contains("parse_json_logs"),
        "line {} is {line:?}",
        parse.range.start.line,
    );
}

/// Every broken config still has to parse: the findings they exist for are
/// topology, not syntax, and a reader that gave up would report nothing.
#[test]
fn the_broken_configs_still_read() {
    for name in [
        "broken/dangling-input.yaml",
        "broken/orphan.yaml",
        "broken/cycle.yaml",
        "broken/empty-wildcard.toml",
    ] {
        let components = read(name);
        assert!(!components.is_empty(), "{name} read as empty");
    }
}

#[test]
fn an_empty_document_is_an_empty_config() {
    assert!(read_yaml("").expect("empty YAML is fine").components.is_empty());
    assert!(read_toml("").expect("empty TOML is fine").components.is_empty());
}
