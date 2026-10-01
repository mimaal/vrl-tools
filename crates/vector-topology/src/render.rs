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

use std::collections::HashMap;

use crate::config::Role;
use crate::graph::{Graph, Severity};
use crate::layout;
use crate::tables::{self, Drawing};

/// The whole document: the diagram, and what is wrong underneath it.
#[must_use]
pub fn document(graph: &Graph, title: &str) -> String {
    document_of_files(graph, title, &[])
}

/// [`document`] for a pipeline read from several files, so that a problem
/// says which file its line is in.
#[must_use]
pub fn document_of_files(graph: &Graph, title: &str, files: &[String]) -> String {
    document_drawn(graph, &tables::drawing(graph, false), title, files)
}

/// [`document_of_files`], drawing what `drawing` says to draw.
#[must_use]
pub fn document_drawn(graph: &Graph, drawing: &Drawing, title: &str, files: &[String]) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&summary(graph));
    out.push_str("\n\n");
    if let Some(left_out) = tables_left_out(graph, drawing) {
        out.push_str(&left_out);
        out.push_str("\n\n");
    }
    out.push_str(&diagram_drawn(graph, drawing));
    out.push_str(&edges(graph));

    if !graph.findings.is_empty() {
        out.push_str(&problems(graph, files));
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

    // Tables are named only when there are any, so the usual config keeps the
    // sentence it always had.
    let tables = match count(Role::Table) {
        0 => String::new(),
        tables => format!(
            ", {tables} {}",
            plural(tables, "enrichment table", "enrichment tables"),
        ),
    };

    format!(
        "{} {}, {} {}, {} {}{tables}, {} {}.",
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

/// Declares the nodes of `components`, each line indented by `indent`.
fn nodes(graph: &Graph, components: &[usize], indent: &str, out: &mut String) {
    for &position in components {
        let component = &graph.components[position];
        let label = escape(&if component.component_type.is_empty() {
            component.id.clone()
        } else {
            format!("{}\n{}", component.id, component.component_type)
        });

        // Shapes carry the role, so the picture reads without a legend:
        // rounded for where events come in, square for what happens to them,
        // a cylinder for where they end up, a framed box for a table, which is
        // consulted from VRL rather than passed through.
        //
        // `vector graph --format mermaid` uses a different set — parallelograms
        // for sources and sinks, a rhombus for transforms, a cylinder for
        // tables. These are not copied: a rhombus around two lines of text is
        // unreadable, and every shape below is one this crate's tests have
        // pushed a quoted, `<br/>`-carrying label through.
        let shape = match component.role {
            Role::Source => format!("([\"{label}\"])"),
            Role::Transform => format!("[\"{label}\"]"),
            Role::Sink => format!("[(\"{label}\")]"),
            Role::Table => format!("[[\"{label}\"]]"),
        };

        out.push_str(&format!("{indent}{}{shape}\n", node_id(position)));
    }
}

/// What the diagram leaves out, said in words: the tables nothing flows
/// through, and how many of the pipeline's tables its VRL reads.
fn tables_left_out(graph: &Graph, drawing: &Drawing) -> Option<String> {
    let hidden = graph
        .components
        .iter()
        .zip(&drawing.drawn)
        .filter(|(component, &drawn)| !drawn && component.is_lookup_table())
        .count();
    if hidden == 0 {
        return None;
    }

    let read = tables::tables(graph)
        .iter()
        .filter(|table| !table.readers.is_empty())
        .count();
    Some(format!(
        "{hidden} {} not drawn: nothing flows through {}, VRL looks {} up by name. \
         {read} of the pipeline's tables {} read by a literal name in its VRL.",
        plural(hidden, "enrichment table is", "enrichment tables are"),
        plural(hidden, "it", "them"),
        plural(hidden, "it", "them"),
        plural(read, "is", "are"),
    ))
}

/// The Mermaid flowchart of the components events pass through.
#[must_use]
pub fn diagram(graph: &Graph) -> String {
    diagram_drawn(graph, &tables::drawing(graph, false))
}

/// The Mermaid flowchart of what `drawing` draws.
#[must_use]
pub fn diagram_drawn(graph: &Graph, drawing: &Drawing) -> String {
    let mut out = String::from("```mermaid\nflowchart LR\n");
    if drawing.count() == 0 {
        // Mermaid rejects an empty graph outright, and an error where a
        // picture should be reads as a bug in the extension rather than as an
        // empty config.
        out.push_str("  empty[\"no components\"]\n```\n");
        return out;
    }

    // Parts that share no arrow are framed apart, each under the name of
    // where its events come from. One part needs no frame.
    let clusters = layout::clusters(graph, drawing);
    let framed = clusters.len() > 1;
    for (number, cluster) in clusters.iter().enumerate() {
        if framed {
            out.push_str(&format!("  subgraph c{number}[\"{}\"]\n", escape(&cluster.title)));
        }
        nodes(graph, &cluster.components, if framed { "    " } else { "  " }, &mut out);
        if framed {
            out.push_str("  end\n");
        }
    }

    // Built once: an edge names its ends, and searching the component list for
    // each of them is quadratic on a pipeline with hundreds of arrows.
    let mut index: HashMap<&str, usize> = HashMap::with_capacity(graph.components.len());
    for (position, component) in graph.components.iter().enumerate() {
        index.entry(component.id.as_str()).or_insert(position);
    }

    for edge in &graph.edges {
        let (Some(&from), Some(&to)) = (
            index.get(edge.from.as_str()),
            index.get(edge.to.as_str()),
        ) else {
            continue;
        };

        // The output is on the arrow, not in the node, because it is a
        // property of this particular path and the same transform usually has
        // several. Written whole — `split.errors`, not `errors` — because a
        // pipeline has more than one router and every one has a `_unmatched`.
        match &edge.output {
            Some(output) => out.push_str(&format!(
                "  {} -->|\"{}\"| {}\n",
                node_id(from),
                escape(&format!("{}.{output}", edge.from)),
                node_id(to),
            )),
            None => out.push_str(&format!("  {} --> {}\n", node_id(from), node_id(to))),
        }
    }

    // Dotted, and without an arrow's meaning: no event travels along a
    // lookup, the program at the far end reads the table.
    for &(table, reader) in &drawing.lookups {
        out.push_str(&format!("  {} -.-> {}\n", node_id(table), node_id(reader)));
    }

    out.push_str("```\n");
    out
}

/// The edges as text, each output written the way an input names it:
/// `` `split.errors --> alerts` ``. The diagram says the same thing, but a
/// list can be searched, diffed and read aloud, and it is the form the config
/// itself uses.
fn edges(graph: &Graph) -> String {
    if graph.edges.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n## Edges\n\n");
    for edge in &graph.edges {
        match &edge.output {
            Some(output) => out.push_str(&format!("- `{}.{output} --> {}`\n", edge.from, edge.to)),
            None => out.push_str(&format!("- `{} --> {}`\n", edge.from, edge.to)),
        }
    }
    out
}

fn problems(graph: &Graph, files: &[String]) -> String {
    let mut out = String::from("\n## Problems\n\n");

    for finding in &graph.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };

        // One-based, because the document is read by a person next to an
        // editor whose gutter counts from one. The file is named only when
        // there is more than one to choose from.
        let place = match files.get(finding.file) {
            Some(file) if files.len() > 1 => format!("`{file}`, line {}", finding.range.start.line + 1),
            _ => format!("line {}", finding.range.start.line + 1),
        };
        out.push_str(&format!("- **{severity}**, {place}: {}\n", finding.message));
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

        assert!(text.contains("-->|\"split.errors\"| "), "{text}");
    }

    /// The same edges as a list, which is what can be searched and read
    /// aloud: every output by the name an input would use for it.
    #[test]
    fn the_document_lists_every_edge_by_its_written_output() {
        let text = document(
            &graph_of(
                "
sources:
  app_logs:
    type: file
transforms:
  split:
    type: route
    inputs: [app_logs]
    route:
      errors: 'true'
sinks:
  out:
    type: console
    inputs: [split.errors]
",
            ),
            "vector.yaml",
        );

        assert!(text.contains("## Edges\n\n- `app_logs --> split`\n- `split.errors --> out`\n"), "{text}");
        assert!(text.find("```mermaid") < text.find("## Edges"), "{text}");
        assert!(text.find("## Edges") < text.find("## Problems"), "{text}");
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

    const WITH_TABLES: &str = "
sources:
  in:
    type: stdin
transforms:
  parse:
    type: remap
    inputs: [in]
    source: '.h = get_enrichment_table_record!(\"hosts\", {})'
sinks:
  out:
    type: console
    inputs: [parse]
enrichment_tables:
  hosts:
    type: file
  spare:
    type: file
";

    /// The tables are counted and accounted for in words, and not drawn.
    #[test]
    fn tables_nothing_flows_through_are_said_not_drawn() {
        let text = document(&graph_of(WITH_TABLES), "vector.yaml");

        assert!(text.contains("2 enrichment tables, 2 connections."), "{text}");
        assert!(text.contains("2 enrichment tables are not drawn"), "{text}");
        assert!(text.contains("1 of the pipeline's tables is read"), "{text}");
        assert!(!text.contains("[["), "a table was drawn: {text}");
    }

    #[test]
    fn asked_for_a_table_is_drawn_with_a_dotted_line_to_its_reader() {
        let graph = graph_of(WITH_TABLES);
        let text = super::diagram_drawn(&graph, &crate::tables::drawing(&graph, true));

        assert!(text.contains("n3[[\"hosts<br/>file\"]]"), "{text}");
        assert!(text.contains("n3 -.-> n1"), "{text}");
        assert!(!text.contains("spare"), "{text}");
    }

    /// Two graphs in one config are framed apart, the main one first, each
    /// under its sources' names.
    #[test]
    fn disconnected_parts_are_subgraphs() {
        let text = diagram(&graph_of(
            "
sources:
  metrics:
    type: internal_metrics
  app:
    type: file
transforms:
  parse:
    type: remap
    inputs: [app]
sinks:
  out:
    type: console
    inputs: [parse]
  prom:
    type: prometheus_exporter
    inputs: [metrics]
",
        ));

        let app = text.find("subgraph c0[\"app\"]").expect(&text);
        let metrics = text.find("subgraph c1[\"metrics\"]").expect(&text);
        assert!(app < metrics, "{text}");
        assert_eq!(text.matches("\n  end\n").count(), 2, "{text}");
        // Every node inside a frame, every arrow after them.
        assert!(text.find("    n1([\"metrics<br/>internal_metrics\"])") > Some(metrics), "{text}");
        assert!(text.find("-->") > text.rfind("\n  end\n"), "{text}");
    }

    /// One connected pipeline is drawn as it always was, with no frame.
    #[test]
    fn a_single_part_has_no_subgraph() {
        assert!(!diagram(&graph_of(STRAIGHT)).contains("subgraph"));
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
