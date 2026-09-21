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
//! A cycle has no left-to-right order, and a topology with one is already an
//! error in [`crate::graph`]. It still has to be drawn, so the edge closing
//! each loop is ignored for placement and drawn going backwards.

use std::collections::HashMap;

use crate::config::Role;
use crate::graph::Graph;

/// How many times rows are re-ordered, each time in both directions. Beyond a
/// handful the order stops moving for any pipeline a person writes by hand.
const PASSES: usize = 4;

/// The arrangement of a whole graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// One per component, in the order of [`Graph::components`].
    pub components: Vec<Placement>,
    /// The lanes of every arrow that skips columns. An arrow between
    /// neighbouring columns, or one closing a loop, has none.
    pub routes: Vec<Route>,
}

/// Where one component goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Index into [`Graph::components`].
    pub component: usize,
    /// Zero-based, left to right.
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
    let count = graph.components.len();
    if count == 0 {
        return Layout::default();
    }

    let index: HashMap<&str, usize> = graph
        .components
        .iter()
        .enumerate()
        .map(|(position, component)| (component.id.as_str(), position))
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

    let forward = without_back_edges(count, &links);
    let mut columns = columns(graph, &forward);

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

    Layout {
        components: (0..count)
            .map(|component| Placement {
                component,
                column: columns[component],
                row: rows[component],
            })
            .collect(),
        routes: chains
            .into_iter()
            .map(|(from, to, lanes)| Route {
                from,
                to,
                via: lanes
                    .into_iter()
                    .map(|lane| Slot {
                        column: columns[lane],
                        row: rows[lane],
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The links that do not close a loop.
///
/// A depth-first walk from every component in declaration order; a link back
/// to something still on the walk's path is the one that closes a cycle.
/// Declaration order makes the choice stable, so the picture does not flip
/// while somebody types.
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
fn columns(graph: &Graph, forward: &[(usize, usize)]) -> Vec<usize> {
    let count = graph.components.len();
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
    for (position, component) in graph.components.iter().enumerate() {
        // A sink nothing feeds stays where it is: pushing it right would draw
        // an unconnected box at the far end, which says nothing true.
        let fed = forward.iter().any(|&(_, to)| to == position);
        if component.role == Role::Sink && fed {
            column[position] = last.max(1);
        }
    }

    column
}

/// Each component's row within its column.
fn rows(columns: &[usize], forward: &[(usize, usize)]) -> Vec<usize> {
    let width = columns.iter().copied().max().map_or(0, |last| last + 1);

    // Declaration order first: it is what the author wrote, and a sensible
    // starting point for everything the passes below do not change.
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
    use super::{layout, Placement};
    use crate::{build, read_yaml};

    fn placed(source: &str) -> Vec<(String, usize, usize)> {
        let graph = build(read_yaml(source).expect("parses"));
        layout(&graph)
            .components
            .into_iter()
            .map(|Placement { component, column, row }| {
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
