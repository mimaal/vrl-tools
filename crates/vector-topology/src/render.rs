//! The graph as a Markdown document with a Mermaid diagram in it.
//!
//! A document rather than a webview, for the same reason `run` returns one:
//! it can be diffed, copied, searched and scrolled by every feature the editor
//! already has, which no amount of custom HTML would give back. VS Code's
//! built-in Markdown preview renders Mermaid with no extension installed, and
//! so does GitHub — so the same file is both the picture and something worth
//! committing next to the config it describes.
//!
//! Rendering happens here, in the crate that resolved the graph, so that it
//! can be tested without an editor.

use crate::config::Role;
use crate::graph::{Graph, Severity};

/// The whole document: the diagram, and what is wrong underneath it.
#[must_use]
pub fn document(graph: &Graph, title: &str) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&summary(graph));
    out.push_str("\n\n");
    out.push_str(&diagram(graph));

    if !graph.findings.is_empty() {
        out.push_str(&problems(graph));
    }

    out
}

fn summary(graph: &Graph) -> String {
    let count = |role: Role| {
        graph
            .components
            .iter()
            .filter(|component| component.role == role)
            .count()
    };

    format!(
        "{} {}, {} {}, {} {}, {} {}.",
        count(Role::Source),
        plural(count(Role::Source), "source", "sources"),
        count(Role::Transform),
        plural(count(Role::Transform), "transform", "transforms"),
        count(Role::Sink),
        plural(count(Role::Sink), "sink", "sinks"),
        graph.edges.len(),
        plural(graph.edges.len(), "connection", "connections"),
    )
}

/// The Mermaid flowchart.
#[must_use]
pub fn diagram(graph: &Graph) -> String {
    let mut out = String::from("```mermaid\nflowchart LR\n");

    if graph.components.is_empty() {
        // Mermaid rejects an empty graph outright, and an error where a
        // picture should be reads as a bug in the extension rather than as an
        // empty config.
        out.push_str("  empty[\"no components\"]\n```\n");
        return out;
    }

    for (position, component) in graph.components.iter().enumerate() {
        let label = escape(&if component.component_type.is_empty() {
            component.id.clone()
        } else {
            format!("{}\n{}", component.id, component.component_type)
        });

        // Shapes carry the role, so the picture reads without a legend:
        // rounded for where events come in, square for what happens to them,
        // a cylinder for where they end up.
        let shape = match component.role {
            Role::Source => format!("([\"{label}\"])"),
            Role::Transform => format!("[\"{label}\"]"),
            Role::Sink => format!("[(\"{label}\")]"),
        };

        out.push_str(&format!("  {}{shape}\n", node_id(position)));
    }

    for edge in &graph.edges {
        let (Some(from), Some(to)) = (index_of(graph, &edge.from), index_of(graph, &edge.to))
        else {
            continue;
        };

        // The output is on the arrow, not in the node, because it is a
        // property of this particular path and the same transform usually has
        // several.
        match &edge.output {
            Some(output) => out.push_str(&format!(
                "  {} -->|\"{}\"| {}\n",
                node_id(from),
                escape(output),
                node_id(to),
            )),
            None => out.push_str(&format!("  {} --> {}\n", node_id(from), node_id(to))),
        }
    }

    out.push_str("```\n");
    out
}

fn problems(graph: &Graph) -> String {
    let mut out = String::from("\n## Problems\n\n");

    for finding in &graph.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };

        // One-based, because the document is read by a person next to an
        // editor whose gutter counts from one.
        out.push_str(&format!(
            "- **{severity}**, line {}: {}\n",
            finding.range.start.line + 1,
            finding.message,
        ));
    }

    out
}

/// Mermaid node IDs are generated rather than taken from the config.
///
/// A component may be called `app.logs` or `parse-json`, and Mermaid reads
/// punctuation in an ID as syntax. The name goes in the label, where it is
/// quoted and safe.
fn node_id(position: usize) -> String {
    format!("n{position}")
}

fn index_of(graph: &Graph, id: &str) -> Option<usize> {
    graph
        .components
        .iter()
        .position(|component| component.id == id)
}

/// Makes a string safe inside a quoted Mermaid label.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br/>")
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 {
        one
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::{diagram, document, escape};
    use crate::{build, read_yaml};

    fn graph_of(source: &str) -> crate::Graph {
        build(read_yaml(source).expect("parses"))
    }

    const STRAIGHT: &str = "
sources:
  app_logs:
    type: file
transforms:
  parse:
    type: remap
    inputs: [app_logs]
sinks:
  out:
    type: console
    inputs: [parse]
";

    #[test]
    fn the_shapes_carry_the_role() {
        let text = diagram(&graph_of(STRAIGHT));

        assert!(text.contains("n0([\"app_logs<br/>file\"])"), "{text}");
        assert!(text.contains("n1[\"parse<br/>remap\"]"), "{text}");
        assert!(text.contains("n2[(\"out<br/>console\")]"), "{text}");
    }

    #[test]
    fn edges_join_the_generated_ids() {
        let text = diagram(&graph_of(STRAIGHT));

        assert!(text.contains("n0 --> n1"), "{text}");
        assert!(text.contains("n1 --> n2"), "{text}");
    }

    #[test]
    fn a_named_output_rides_on_the_arrow() {
        let text = diagram(&graph_of(
            "
sources:
  app_logs:
    type: file
transforms:
  split:
    type: route
    inputs: [app_logs]
    route:
      errors:
        type: vrl
        source: 'true'
sinks:
  out:
    type: console
    inputs: [split.errors]
",
        ));

        assert!(text.contains("-->|\"errors\"| "), "{text}");
    }

    /// A component name Mermaid would read as syntax has to stay in the label.
    #[test]
    fn punctuation_in_a_name_cannot_reach_the_id() {
        let text = diagram(&graph_of(
            "
sources:
  app.logs-1:
    type: file
sinks:
  out:
    type: console
    inputs: [app.logs-1]
",
        ));

        assert!(text.contains("n0([\"app.logs-1<br/>file\"])"), "{text}");
        assert!(text.contains("n0 --> n1"), "{text}");
    }

    #[test]
    fn an_empty_config_still_draws_something() {
        let text = diagram(&graph_of(""));

        assert!(text.contains("no components"), "{text}");
        assert!(text.starts_with("```mermaid"), "{text}");
    }

    #[test]
    fn a_clean_config_has_no_problems_section() {
        let text = document(&graph_of(STRAIGHT), "vector.yaml");

        assert!(text.starts_with("# vector.yaml"), "{text}");
        assert!(text.contains("1 source, 1 transform, 1 sink, 2 connections."), "{text}");
        assert!(!text.contains("## Problems"), "{text}");
    }

    #[test]
    fn findings_are_listed_with_one_based_lines() {
        let text = document(
            &graph_of(
                "
sources:
  app_logs:
    type: file
sinks:
  out:
    type: console
    inputs: [nope]
",
            ),
            "vector.yaml",
        );

        assert!(text.contains("## Problems"), "{text}");
        assert!(text.contains("**error**, line 8:"), "{text}");
        assert!(text.contains("no component is called `nope`"), "{text}");
    }

    #[test]
    fn labels_cannot_close_their_own_quotes() {
        assert_eq!(escape("a\"b"), "a&quot;b");
        assert_eq!(escape("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(escape("a&b"), "a&amp;b");
    }
}
