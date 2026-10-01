//! Enrichment tables, and why most of them are not drawn.
//!
//! A table is a component to Vector (see [`crate::outputs`]), and for a
//! `memory` table that matters: events are written into it and read back out,
//! so it sits in the flow like anything else. Every other kind is consulted
//! from VRL and touched by no arrow at all — and a pipeline can have hundreds.
//! The one this was measured against had 26 components events pass through
//! and 372 `file` tables: drawn as nodes, the tables were fourteen fifteenths
//! of the picture and connected to nothing.
//!
//! So a table nothing flows through is left out of the drawing, and what is
//! said about it instead is the thing a drawing could not say: who reads it.
//! That comes from the VRL ([`crate::lookups`]). Asked for, the tables that
//! are read are drawn after all, each joined to its readers by a line that is
//! not an arrow of events.
//!
//! A table nobody in the pipeline reads is not a finding. Vector says nothing
//! about one — `validation::warnings` walks the outputs of sources and
//! transforms, and a table has none — and a table file is routinely shared by
//! pipelines that each use a few of its tables.

use std::collections::HashMap;

use editor_text::Range;

use crate::graph::Graph;

/// One enrichment table, and who looks it up.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub id: String,
    #[serde(rename = "type")]
    pub table_type: String,
    /// Where it is declared.
    pub file: usize,
    pub range: Range,
    /// Where its data is, for the kinds that read a file. As written: the
    /// file is on the machine Vector runs on.
    pub path: Option<String>,
    /// Whether a `file` table's first line is a header, not a row.
    pub csv_headers: bool,
    /// The components whose VRL looks it up by name, in the pipeline's order.
    /// Empty means unused *here*, by a literal name.
    pub readers: Vec<String>,
}

/// A component's VRL reading a table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lookup {
    pub table: String,
    pub reader: String,
}

/// Every table of the pipeline, in the order declared, with its readers.
///
/// The source half of a `memory` table is not a table here: it is the same
/// table under the name events leave it by, and nothing can look that up.
#[must_use]
pub fn tables(graph: &Graph) -> Vec<Table> {
    let mut found: Vec<Table> = graph
        .components
        .iter()
        .filter(|component| is_table(component))
        .map(|component| Table {
            id: component.id.clone(),
            table_type: component.component_type.clone(),
            file: component.file,
            range: component.range,
            path: component.path.clone(),
            csv_headers: component.csv_headers,
            readers: Vec::new(),
        })
        .collect();

    // By name, once, rather than a search per lookup: a pipeline of 372
    // tables with a few dozen readers is read between keystrokes.
    let mut index: HashMap<&str, usize> = HashMap::with_capacity(found.len());
    for (position, table) in found.iter().enumerate() {
        index.entry(table.id.as_str()).or_insert(position);
    }
    let mut readers: Vec<(usize, String)> = Vec::new();
    for component in &graph.components {
        for name in &component.lookups {
            if let Some(&position) = index.get(name.as_str()) {
                readers.push((position, component.id.clone()));
            }
        }
    }
    drop(index);
    for (position, reader) in readers {
        found[position].readers.push(reader);
    }
    found
}

fn is_table(component: &crate::Component) -> bool {
    component.role == crate::Role::Table && !component.has_outputs()
}

/// What of a graph is drawn.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Drawing {
    /// One per component of the graph: whether it gets a node.
    pub drawn: Vec<bool>,
    /// The lookups drawn, as (table, reader) indexes into the components.
    /// Empty unless tables were asked for.
    pub lookups: Vec<(usize, usize)>,
}

impl Drawing {
    /// Everything, and no lookups: the graph as its edges alone describe it.
    #[must_use]
    pub fn everything(graph: &Graph) -> Self {
        Self {
            drawn: vec![true; graph.components.len()],
            lookups: Vec::new(),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.drawn.iter().filter(|&&drawn| drawn).count()
    }
}

/// Decides what is drawn: every component events pass through, and — with
/// `show_tables` — the tables the components of *this* graph read. A table
/// nothing here reads stays out either way; drawing 368 boxes joined to
/// nothing is the picture this exists to avoid.
#[must_use]
pub fn drawing(graph: &Graph, show_tables: bool) -> Drawing {
    let mut drawn: Vec<bool> = graph
        .components
        .iter()
        .map(|component| !component.is_lookup_table())
        .collect();
    let mut lookups = Vec::new();

    if show_tables {
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (position, component) in graph.components.iter().enumerate() {
            if is_table(component) {
                index.entry(component.id.as_str()).or_insert(position);
            }
        }
        for (reader, component) in graph.components.iter().enumerate() {
            for name in &component.lookups {
                if let Some(&table) = index.get(name.as_str()) {
                    drawn[table] = true;
                    lookups.push((table, reader));
                }
            }
        }
    }

    Drawing { drawn, lookups }
}

#[cfg(test)]
mod tests {
    use super::{drawing, tables};
    use crate::{build, read_toml};

    const CONFIG: &str = r#"
[enrichment_tables.hosts]
type = "file"
file.path = "/etc/vector/hosts.csv"
file.encoding.type = "csv"

[enrichment_tables.spare]
type = "file"
file.path = "/etc/vector/spare.csv"
file.encoding = { type = "csv", include_headers = false }

[enrichment_tables.cache]
type = "memory"
inputs = ["parse"]

[sources.in]
type = "stdin"

[transforms.parse]
type = "remap"
inputs = ["in"]
source = '''
.host = get_enrichment_table_record!("hosts", { "ip": .ip })
.seen = get_enrichment_table_record("cache", { "ip": .ip }) ?? null
'''

[transforms.keep]
type = "filter"
inputs = ["parse"]
condition = 'exists(get_enrichment_table_record!("hosts", { "ip": .ip }))'

[sinks.out]
type = "console"
inputs = ["keep"]
"#;

    fn graph() -> crate::Graph {
        build(read_toml(CONFIG).expect("parses"))
    }

    #[test]
    fn a_table_knows_its_readers_and_its_path() {
        let tables = tables(&graph());
        let of = |id: &str| tables.iter().find(|table| table.id == id).expect(id);

        assert_eq!(of("hosts").readers, ["parse", "keep"], "a remap and a condition");
        assert_eq!(of("hosts").path.as_deref(), Some("/etc/vector/hosts.csv"));
        assert!(of("hosts").csv_headers, "the default");
        assert!(of("spare").readers.is_empty());
        assert!(!of("spare").csv_headers);
        assert_eq!(of("cache").readers, ["parse"]);
        assert_eq!(of("cache").path, None, "a memory table has no file");
    }

    #[test]
    fn a_table_nothing_flows_through_is_not_drawn() {
        let graph = graph();
        let drawing = drawing(&graph, false);
        let drawn: Vec<&str> = graph
            .components
            .iter()
            .zip(&drawing.drawn)
            .filter(|(_, &drawn)| drawn)
            .map(|(component, _)| component.id.as_str())
            .collect();

        // `cache` takes inputs: events flow into it, so it stays.
        assert_eq!(drawn, ["in", "parse", "keep", "out", "cache"]);
        assert!(drawing.lookups.is_empty());
    }

    #[test]
    fn asked_for_the_tables_read_are_drawn_and_the_unread_one_is_not() {
        let graph = graph();
        let drawing = drawing(&graph, true);
        let id = |position: usize| graph.components[position].id.as_str();

        let drawn: Vec<&str> = (0..graph.components.len())
            .filter(|&position| drawing.drawn[position])
            .map(id)
            .collect();
        assert!(drawn.contains(&"hosts"));
        assert!(!drawn.contains(&"spare"), "{drawn:?}");

        let lookups: Vec<(&str, &str)> =
            drawing.lookups.iter().map(|&(table, reader)| (id(table), id(reader))).collect();
        assert_eq!(lookups, [("hosts", "parse"), ("cache", "parse"), ("hosts", "keep")]);
    }

    /// No finding of any severity: Vector says nothing about an unread table.
    #[test]
    fn an_unused_table_is_not_a_finding() {
        assert!(graph().findings.is_empty(), "{:#?}", graph().findings);
    }
}
