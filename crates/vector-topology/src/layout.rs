//! Where each component sits when the graph is drawn.
//!
//! Only the arrangement — which column, which row — and not pixels. Pixels
//! depend on fonts and zoom, which only the thing doing the drawing knows;
//! the arrangement is the part worth getting right and worth testing, and it
//! is the same whatever draws it.
//!
//! The method is the usual one for a pipeline, a layered layout:
//!
//! - **Columns** follow the flow. A component sits one column to the right of
//!   the furthest component feeding it, so every arrow points right. Sinks
//!   are then pushed to the last column, so where events end up reads as one
//!   edge of the picture rather than being scattered through it.
//! - **Lanes** carry every arrow that skips columns. It gets a slot in each
//!   column it crosses, ordered like a component, so it runs between the
//!   boxes instead of straight through whatever sits in its way.
//! - **Rows** within a column are ordered to keep lines from crossing: each
//!   component and lane moves towards the average row of its neighbours, a
//!   few passes left to right and back again.
//!
//! Before any of that the graph is split into the parts that share no arrow
//! ([`clusters`]) — the pipeline, and Vector's own metrics beside it — and
//! each is arranged on its own, so one is not threaded through the other.
//!
//! A cycle has no left-to-right order, and a topology with one is already an
//! error in [`crate::graph`]. It still has to be drawn, so the edge closing
//! each loop is ignored for placement and drawn going backwards.

use std::collections::HashMap;

use crate::config::Role;
use crate::graph::Graph;
use crate::tables::Drawing;

/// How many times rows are re-ordered, each time in both directions. Beyond a
/// handful the order stops moving for any pipeline a person writes by hand.
const PASSES: usize = 4;

/// The arrangement of a whole graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// One per component drawn, in the order of [`Graph::components`].
    pub components: Vec<Placement>,
    /// The lanes of every arrow that skips columns. An arrow between
    /// neighbouring columns, or one closing a loop, has none.
    pub routes: Vec<Route>,
    /// The parts of the graph no arrow joins, the main one first. Columns and
    /// rows are counted within each.
    pub clusters: Vec<Cluster>,
}

/// A part of the graph that shares no arrow with the rest: one connected
/// subgraph, drawn as a band of its own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    /// What it is called: the sources its events come from, or, when it has
    /// none, its first component.
    pub title: String,
    /// Indexes into [`Graph::components`], in their order.
    pub components: Vec<usize>,
}

/// Where one component goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Index into [`Graph::components`].
    pub component: usize,
    /// Index into [`Layout::clusters`]: the band it is in.
    pub cluster: usize,
    /// Zero-based, left to right, within the cluster.
    pub column: usize,
    /// Zero-based, top to bottom, within the column. Lanes count as rows.
    pub row: usize,
}

/// The way an arrow takes across the columns between its ends.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// Indexes into [`Graph::components`]. Every edge between the two follows
    /// the same lanes, whichever output it leaves by.
    pub from: usize,
    pub to: usize,
    /// The cluster both ends are in.
    pub cluster: usize,
    /// One slot per column crossed, left to right.
    pub via: Vec<Slot>,
}

/// A place in the grid that is not a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub column: usize,
    pub row: usize,
}

/// Arranges every component of `graph`.
#[must_use]
pub fn layout(graph: &Graph) -> Layout {
    arrange(graph, &Drawing::everything(graph))
}

/// Arranges the components of `graph` that `drawing` draws; the rest get no
/// placement at all.
///
/// A table drawn for its lookups goes in the column just before the first
/// component that reads it, so the line between them is short. Lookups do not
/// move anything else: the columns are the flow's, worked out before the
/// tables are placed, so asking for the tables adds boxes without rearranging
/// the pipeline around them.
#[must_use]
pub fn arrange(graph: &Graph, drawing: &Drawing) -> Layout {
    let clusters = clusters(graph, drawing);
    let mut layout = Layout::default();
    for (number, cluster) in clusters.iter().enumerate() {
        arrange_cluster(graph, drawing, number, &cluster.components, &mut layout);
    }
    // In the order of the components, as before there were clusters: it is
    // the order whoever draws creates the nodes in.
    layout.components.sort_by_key(|placement| placement.component);
    layout.clusters = clusters;
    layout
}

/// The parts of what is drawn that no arrow joins.
///
/// Connected by the edges and by nothing else: a table two parts both read
/// does not make them one, any more than a shared file does. A table drawn
/// for its lookups goes with the first component that reads it.
///
/// The biggest comes first — that is the pipeline, and the rest are what runs
/// beside it — and equal ones keep the order of their first component.
#[must_use]
pub fn clusters(graph: &Graph, drawing: &Drawing) -> Vec<Cluster> {
    let count = graph.components.len();
    let drawn = |position: usize| drawing.drawn.get(position).copied().unwrap_or(false);
    // A table that is there for its lookups, and that no arrow touches.
    let attached = |position: usize| graph.components[position].is_lookup_table();

    let mut index: HashMap<&str, usize> = HashMap::with_capacity(count);
    for (position, component) in graph.components.iter().enumerate() {
        if drawn(position) {
            index.entry(component.id.as_str()).or_insert(position);
        }
    }

    // Union-find, with the smaller index as the root so a part is named by
    // its first component. Path halving keeps it iterative.
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for edge in &graph.edges {
        let (Some(&from), Some(&to)) = (index.get(edge.from.as_str()), index.get(edge.to.as_str()))
        else {
            continue;
        };
        let (a, b) = (root(&mut parent, from), root(&mut parent, to));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }

    let mut number: HashMap<usize, usize> = HashMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for position in 0..count {
        if !drawn(position) || attached(position) {
            continue;
        }
        let part = root(&mut parent, position);
        let slot = *number.entry(part).or_insert_with(|| {
            members.push(Vec::new());
            members.len() - 1
        });
        members[slot].push(position);
    }

    let mut order: Vec<usize> = (0..members.len()).collect();
    // Stable, so equal parts keep the order of their first component. Sized
    // before the tables join, which are not what makes a pipeline the main one.
    order.sort_by_key(|&slot| std::cmp::Reverse(members[slot].len()));
    let mut place = vec![0usize; members.len()];
    for (position, &slot) in order.iter().enumerate() {
        place[slot] = position;
    }

    let mut parts: Vec<Vec<usize>> = order.iter().map(|&slot| members[slot].clone()).collect();
    let mut taken = vec![false; count];
    for &(table, reader) in &drawing.lookups {
        if table >= count || reader >= count || taken[table] || !drawn(table) || !attached(table) {
            continue;
        }
        if let Some(&slot) = number.get(&root(&mut parent, reader)) {
            taken[table] = true;
            parts[place[slot]].push(table);
        }
    }
    // A table drawn with no reader drawn has nowhere to go but a part of its
    // own. `tables::drawing` never produces one; `Drawing::everything` does.
    for position in 0..count {
        if drawn(position) && attached(position) && !taken[position] {
            parts.push(vec![position]);
        }
    }

    parts
        .into_iter()
        .map(|mut components| {
            components.sort_unstable();
            Cluster {
                title: title(graph, &components),
                components,
            }
        })
        .collect()
}

/// How many sources a cluster's title names before it counts the rest.
const TITLE_SOURCES: usize = 3;

fn title(graph: &Graph, components: &[usize]) -> String {
    let sources: Vec<&str> = components
        .iter()
        .map(|&position| &graph.components[position])
        .filter(|component| component.role == Role::Source)
        .map(|component| component.id.as_str())
        .collect();

    match sources.len() {
        0 => components
            .first()
            .map_or_else(String::new, |&position| graph.components[position].id.clone()),
        count if count > TITLE_SOURCES => format!(
            "{} +{}",
            sources[..TITLE_SOURCES].join(", "),
            count - TITLE_SOURCES,
        ),
        _ => sources.join(", "),
    }
}

/// Arranges one cluster, adding its placements and routes to `layout`.
fn arrange_cluster(
    graph: &Graph,
    drawing: &Drawing,
    cluster: usize,
    shown: &[usize],
    layout: &mut Layout,
) {
    // The components of the cluster, numbered among themselves; everything
    // below works on those numbers and `shown` turns them back.
    let count = shown.len();
    if count == 0 {
        return;
    }
    let mut local: Vec<Option<usize>> = vec![None; graph.components.len()];
    for (position, &component) in shown.iter().enumerate() {
        local[component] = Some(position);
    }

    let index: HashMap<&str, usize> = shown
        .iter()
        .enumerate()
        .map(|(position, &component)| (graph.components[component].id.as_str(), position))
        .collect();

    // One link per pair, whatever the number of outputs joining them: two
    // routes into the same sink are two arrows but one neighbour.
    let mut links: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .filter_map(|edge| Some((*index.get(edge.from.as_str())?, *index.get(edge.to.as_str())?)))
        .filter(|(from, to)| from != to)
        .collect();
    links.sort_unstable();
    links.dedup();

    let roles: Vec<Role> = shown
        .iter()
        .map(|&component| graph.components[component].role)
        .collect();
    let mut forward = without_back_edges(count, &links);
    let mut columns = columns(&roles, &forward);

    // The lookups, placed against the columns the flow already has.
    let at = |component: usize| local.get(component).copied().flatten();
    let mut lookups: Vec<(usize, usize)> = drawing
        .lookups
        .iter()
        .filter_map(|&(table, reader)| Some((at(table)?, at(reader)?)))
        .filter(|(table, reader)| table != reader)
        .collect();
    lookups.sort_unstable();
    lookups.dedup();
    for &(table, _) in &lookups {
        // Only a table no arrow touches is moved; a `memory` table events
        // flow into is where the flow put it.
        if links.iter().any(|&(from, to)| from == table || to == table) {
            continue;
        }
        let nearest = lookups
            .iter()
            .filter(|&&(other, _)| other == table)
            .map(|&(_, reader)| columns[reader])
            .min()
            .unwrap_or(0);
        columns[table] = nearest.saturating_sub(1);
    }
    // A lookup is laid out like any link once it points right; one that does
    // not (its reader is in the first column, or behind the table) is drawn
    // by whoever draws, without a lane.
    forward.extend(
        lookups
            .iter()
            .copied()
            .filter(|&(table, reader)| columns[table] < columns[reader]),
    );
    forward.sort_unstable();
    forward.dedup();

    // Lanes are nodes as far as ordering goes, numbered after the components.
    // Each long link becomes a chain through them.
    let mut chains: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    let mut steps: Vec<(usize, usize)> = Vec::new();
    for &(from, to) in &forward {
        let mut previous = from;
        let mut lanes = Vec::new();
        for column in columns[from] + 1..columns[to] {
            let lane = columns.len();
            columns.push(column);
            steps.push((previous, lane));
            lanes.push(lane);
            previous = lane;
        }
        steps.push((previous, to));
        if !lanes.is_empty() {
            chains.push((from, to, lanes));
        }
    }

    let rows = rows(&columns, &steps);

    layout.components.extend((0..count).map(|position| Placement {
        component: shown[position],
        cluster,
        column: columns[position],
        row: rows[position],
    }));
    layout.routes.extend(chains.into_iter().map(|(from, to, lanes)| Route {
        from: shown[from],
        to: shown[to],
        cluster,
        via: lanes
            .into_iter()
            .map(|lane| Slot {
                column: columns[lane],
                row: rows[lane],
            })
            .collect(),
    }));
}

/// The links that do not close a loop.
///
/// A depth-first walk from every component in the order they come in; a link
/// back to something still on the walk's path is the one that closes a cycle.
/// That order is by role and name, so the choice is stable and the picture
/// does not flip while somebody types.
fn without_back_edges(count: usize, links: &[(usize, usize)]) -> Vec<(usize, usize)> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        New,
        OnPath,
        Done,
    }

    fn visit(
        node: usize,
        links: &[(usize, usize)],
        state: &mut [State],
        back: &mut Vec<(usize, usize)>,
    ) {
        state[node] = State::OnPath;
        for &(from, to) in links.iter().filter(|(from, _)| *from == node) {
            match state[to] {
                State::New => visit(to, links, state, back),
                State::OnPath => back.push((from, to)),
                State::Done => {}
            }
        }
        state[node] = State::Done;
    }

    let mut state = vec![State::New; count];
    let mut back = Vec::new();
    for node in 0..count {
        if state[node] == State::New {
            visit(node, links, &mut state, &mut back);
        }
    }

    links
        .iter()
        .copied()
        .filter(|link| !back.contains(link))
        .collect()
}

/// Each component's column: one past the furthest thing feeding it, with
/// sinks gathered in the last column.
fn columns(roles: &[Role], forward: &[(usize, usize)]) -> Vec<usize> {
    let count = roles.len();
    let mut column = vec![0; count];

    // Longest path on a DAG by relaxation. `count` rounds are enough for any
    // DAG of `count` nodes, and there is no cycle left to keep it going.
    for _ in 0..count {
        let mut moved = false;
        for &(from, to) in forward {
            if column[to] < column[from] + 1 {
                column[to] = column[from] + 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let last = column.iter().copied().max().unwrap_or(0);
    for (position, &role) in roles.iter().enumerate() {
        // A sink nothing feeds stays where it is: pushing it right would draw
        // an unconnected box at the far end, which says nothing true.
        let fed = forward.iter().any(|&(_, to)| to == position);
        if role == Role::Sink && fed {
            column[position] = last.max(1);
        }
    }

    column
}

/// Each component's row within its column.
fn rows(columns: &[usize], forward: &[(usize, usize)]) -> Vec<usize> {
    let width = columns.iter().copied().max().map_or(0, |last| last + 1);

    // The order the components come in first — the order events meet them,
    // see `graph::build` — which is a sensible starting point for everything
    // the passes below do not change.
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); width];
    for (component, &column) in columns.iter().enumerate() {
        order[column].push(component);
    }

    let mut row = vec![0usize; columns.len()];
    let renumber = |order: &[Vec<usize>], row: &mut [usize]| {
        for column in order {
            for (position, &component) in column.iter().enumerate() {
                row[component] = position;
            }
        }
    };
    renumber(&order, &mut row);

    for _ in 0..PASSES {
        for column in 1..width {
            reorder(&mut order[column], &row, |node| {
                forward.iter().filter(move |&&(_, to)| to == node).map(|&(from, _)| from)
            });
            renumber(&order, &mut row);
        }
        for column in (0..width.saturating_sub(1)).rev() {
            reorder(&mut order[column], &row, |node| {
                forward.iter().filter(move |&&(from, _)| from == node).map(|&(_, to)| to)
            });
            renumber(&order, &mut row);
        }
    }

    row
}

/// Sorts one column by the average row of each component's neighbours.
///
/// A component with no neighbours on that side keeps its current row as its
/// key, so it holds its place instead of drifting to the top. The sort is
/// stable, so ties keep the order they had.
fn reorder<I>(column: &mut [usize], row: &[usize], neighbours: impl Fn(usize) -> I)
where
    I: Iterator<Item = usize>,
{
    let mut keyed: Vec<(f64, usize)> = column
        .iter()
        .map(|&node| {
            let rows: Vec<usize> = neighbours(node).map(|other| row[other]).collect();
            #[allow(clippy::cast_precision_loss)]
            let key = if rows.is_empty() {
                row[node] as f64
            } else {
                rows.iter().sum::<usize>() as f64 / rows.len() as f64
            };
            (key, node)
        })
        .collect();

    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (slot, (_, node)) in column.iter_mut().zip(keyed) {
        *slot = node;
    }
}

#[cfg(test)]
mod tests {
    use super::{arrange, layout, Placement};
    use crate::tables::drawing;
    use crate::{build, read_yaml};

    const TABLES: &str = "sources:\n  in:\n    type: stdin\n\
        transforms:\n  parse:\n    type: remap\n    inputs: [in]\n    source: '.h = get_enrichment_table_record!(\"hosts\", {})'\n\
        sinks:\n  out:\n    type: console\n    inputs: [parse]\n\
        enrichment_tables:\n  hosts:\n    type: file\n  unread:\n    type: file\n";

    fn placed(source: &str) -> Vec<(String, usize, usize)> {
        let graph = build(read_yaml(source).expect("parses"));
        layout(&graph)
            .components
            .into_iter()
            .map(|Placement { component, column, row, .. }| {
                (graph.components[component].id.clone(), column, row)
            })
            .collect()
    }

    fn column_of(placed: &[(String, usize, usize)], id: &str) -> usize {
        placed.iter().find(|(name, ..)| name == id).expect(id).1
    }

    #[test]
    fn every_arrow_points_right() {
        let placed = placed(
            "sources:\n  in:\n    type: file\n\
             transforms:\n  a:\n    type: remap\n    inputs: [in]\n  b:\n    type: remap\n    inputs: [a]\n\
             sinks:\n  out:\n    type: console\n    inputs: [b]\n",
        );

        assert_eq!(column_of(&placed, "in"), 0);
        assert_eq!(column_of(&placed, "a"), 1);
        assert_eq!(column_of(&placed, "b"), 2);
        assert_eq!(column_of(&placed, "out"), 3);
    }

    /// A component fed from two depths sits past the deeper one, or one of
    /// its arrows would point left.
    #[test]
    fn a_join_sits_past_its_furthest_input() {
        let placed = placed(
            "sources:\n  in:\n    type: file\n\
             transforms:\n  a:\n    type: remap\n    inputs: [in]\n  join:\n    type: remap\n    inputs: [in, a]\n",
        );

        assert_eq!(column_of(&placed, "join"), 2);
    }

    #[test]
    fn sinks_line_up_on_the_right() {
        let placed = placed(
            "sources:\n  in:\n    type: file\n\
             transforms:\n  a:\n    type: remap\n    inputs: [in]\n\
             sinks:\n  raw:\n    type: console\n    inputs: [in]\n  parsed:\n    type: console\n    inputs: [a]\n",
        );

        assert_eq!(column_of(&placed, "raw"), 2);
        assert_eq!(column_of(&placed, "parsed"), 2);
    }

    /// Two parallel paths drawn crossed are the thing the row ordering is for.
    #[test]
    fn parallel_paths_do_not_cross() {
        let placed = placed(
            "sources:\n  one:\n    type: file\n  two:\n    type: file\n\
             transforms:\n  b:\n    type: remap\n    inputs: [two]\n  a:\n    type: remap\n    inputs: [one]\n",
        );

        let row = |id: &str| placed.iter().find(|(name, ..)| name == id).expect(id).2;
        assert_eq!(row("one") < row("two"), row("a") < row("b"), "{placed:?}");
    }

    /// A loop is already reported as an error. What matters here is that it
    /// is drawn at all, and that the layout terminates.
    #[test]
    fn a_cycle_is_still_placed() {
        let placed = placed(
            "transforms:\n  a:\n    type: remap\n    inputs: [b]\n  b:\n    type: remap\n    inputs: [a]\n",
        );

        assert_eq!(placed.len(), 2);
        assert_ne!(column_of(&placed, "a"), column_of(&placed, "b"));
    }

    /// The defect lanes exist for: `in` feeds both `a` and the sink `out`,
    /// which sits two columns away, and its arrow must not be drawn through
    /// `a`.
    #[test]
    fn an_arrow_skipping_a_column_gets_its_own_lane() {
        let source = "sources:\n  in:\n    type: file\n\
             transforms:\n  a:\n    type: remap\n    inputs: [in]\n\
             sinks:\n  out:\n    type: console\n    inputs: [in, a]\n";
        let graph = build(read_yaml(source).expect("parses"));
        let arranged = layout(&graph);

        assert_eq!(arranged.routes.len(), 1, "{arranged:?}");
        let route = &arranged.routes[0];
        assert_eq!(graph.components[route.from].id, "in");
        assert_eq!(graph.components[route.to].id, "out");
        assert_eq!(route.via.len(), 1);

        let a = arranged.components[1];
        assert_eq!(route.via[0].column, a.column);
        assert_ne!(route.via[0].row, a.row, "the lane and `a` share a place");
    }

    /// The tables are what a real pipeline has most of, and none of them is
    /// on the way anywhere: left out, the picture is the pipeline again.
    #[test]
    fn a_table_that_is_not_drawn_gets_no_place() {
        let graph = build(read_yaml(TABLES).expect("parses"));
        let arranged = arrange(&graph, &drawing(&graph, false));

        let placed: Vec<&str> = arranged
            .components
            .iter()
            .map(|p| graph.components[p.component].id.as_str())
            .collect();
        assert_eq!(placed, ["in", "parse", "out"]);
    }

    /// Asked for, a table sits in the column before its reader and moves
    /// nothing else.
    #[test]
    fn a_table_drawn_for_its_lookup_sits_before_its_reader() {
        let graph = build(read_yaml(TABLES).expect("parses"));
        let without = arrange(&graph, &drawing(&graph, false));
        let with = arrange(&graph, &drawing(&graph, true));
        let column = |layout: &super::Layout, id: &str| {
            layout
                .components
                .iter()
                .find(|p| graph.components[p.component].id == id)
                .map(|p| p.column)
        };

        assert_eq!(column(&with, "hosts"), Some(0));
        assert_eq!(column(&with, "parse"), Some(1));
        assert_eq!(column(&with, "unread"), None, "nothing here reads it");
        for id in ["in", "parse", "out"] {
            assert_eq!(column(&with, id), column(&without, id), "{id} moved");
        }
    }

    const TWO: &str = "sources:\n  metrics:\n    type: internal_metrics\n  http:\n    type: http_server\n  syslog:\n    type: syslog\n\
        transforms:\n  parse:\n    type: remap\n    inputs: [http, syslog]\n    source: '.h = get_enrichment_table_record!(\"hosts\", {})'\n\
        \x20 count:\n    type: remap\n    inputs: [metrics]\n    source: '.h = get_enrichment_table_record!(\"hosts\", {})'\n\
        sinks:\n  out:\n    type: console\n    inputs: [parse]\n  prom:\n    type: prometheus_exporter\n    inputs: [count]\n\
        enrichment_tables:\n  hosts:\n    type: file\n";

    /// A pipeline and Vector's own metrics beside it share no arrow, so they
    /// are two bands: the bigger first, each named for where its events come
    /// from, each with columns of its own.
    #[test]
    fn parts_no_arrow_joins_are_clusters_of_their_own() {
        let graph = build(read_yaml(TWO).expect("parses"));
        let arranged = arrange(&graph, &drawing(&graph, false));
        let ids = |components: &[usize]| -> Vec<&str> {
            components.iter().map(|&c| graph.components[c].id.as_str()).collect()
        };

        assert_eq!(arranged.clusters.len(), 2);
        assert_eq!(arranged.clusters[0].title, "http, syslog");
        assert_eq!(ids(&arranged.clusters[0].components), ["http", "syslog", "parse", "out"]);
        assert_eq!(arranged.clusters[1].title, "metrics");
        assert_eq!(ids(&arranged.clusters[1].components), ["metrics", "count", "prom"]);

        for placement in &arranged.components {
            let id = graph.components[placement.component].id.as_str();
            let cluster = usize::from(["metrics", "count", "prom"].contains(&id));
            assert_eq!(placement.cluster, cluster, "{id}");
        }
        // Each starts at its own first column.
        let column = |id: &str| {
            arranged
                .components
                .iter()
                .find(|p| graph.components[p.component].id == id)
                .map(|p| p.column)
        };
        assert_eq!(column("http"), Some(0));
        assert_eq!(column("metrics"), Some(0));
        assert_eq!(column("out"), Some(2));
        assert_eq!(column("prom"), Some(2));
    }

    /// A table both parts read is drawn once, with the first to read it, and
    /// does not make one part of two.
    #[test]
    fn a_table_two_clusters_read_does_not_join_them() {
        let graph = build(read_yaml(TWO).expect("parses"));
        let arranged = arrange(&graph, &drawing(&graph, true));

        assert_eq!(arranged.clusters.len(), 2);
        let hosts = graph.components.iter().position(|c| c.id == "hosts").expect("hosts");
        let holding: Vec<usize> = (0..2)
            .filter(|&cluster| arranged.clusters[cluster].components.contains(&hosts))
            .collect();
        assert_eq!(holding.len(), 1, "{:?}", arranged.clusters);
    }

    /// A component nothing is wired to is a part of its own, not a stray box
    /// inside somebody else's.
    #[test]
    fn an_unwired_component_is_its_own_cluster() {
        let graph = build(
            read_yaml(
                "sources:\n  in:\n    type: stdin\n  spare:\n    type: stdin\n\
                 sinks:\n  out:\n    type: console\n    inputs: [in]\n",
            )
            .expect("parses"),
        );
        let arranged = layout(&graph);
        let titles: Vec<&str> = arranged.clusters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["in", "spare"]);
    }

    #[test]
    fn no_two_components_share_a_place() {
        let placed = placed(
            "sources:\n  a:\n    type: file\n  b:\n    type: file\n  c:\n    type: file\n\
             sinks:\n  out:\n    type: console\n    inputs: ['*']\n",
        );

        let mut places: Vec<(usize, usize)> = placed.iter().map(|&(_, c, r)| (c, r)).collect();
        places.sort_unstable();
        places.dedup();
        assert_eq!(places.len(), placed.len());
    }
}
