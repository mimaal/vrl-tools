//! Turning components into a graph, and saying what is wrong with it.
//!
//! [`crate::config`] reads what the file says. This works out what it means:
//! which component feeds which, through which output, and what Vector would
//! refuse to start on.
//!
//! The findings are the part `vector graph` cannot give you. It draws a
//! topology it has already validated, because Vector refused to start
//! otherwise. Here the config is being typed, so the interesting states are
//! precisely the ones that do not work yet.
//!
//! Each check below is one of Vector's, named where it lives so the pair can
//! be re-read when the pin moves:
//!
//! - resolving an input — `Graph::add_input` and `expand_globs`
//!   (`src/config/graph.rs`, `src/config/compiler.rs`);
//! - a name with a dot in it — `validation::check_names`;
//! - a repeated name, a missing `inputs`, a repeated input, an empty
//!   `sources` or `sinks` — `validation::check_shape`;
//! - a loop — `Graph::check_for_cycles`;
//! - an output nobody reads — `validation::warnings`, which is the only one of
//!   the lot Vector treats as a warning rather than a refusal to start.

use std::collections::{HashMap, HashSet};

use editor_text::Range;

use crate::config::{Component, Document, Role};

/// How much a finding matters.
///
/// The line is whether Vector would start. An input naming nothing is fatal;
/// an output nobody reads runs happily and throws its events away, which is
/// worse to debug and less urgent to fix. It is the same line Vector draws
/// between the errors `compile` returns and the warnings it hands back
/// alongside a working config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Something wrong with the topology.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
    /// Where to point: the input that does not resolve, or the component the
    /// finding is about.
    pub range: Range,
    /// The file `range` is in. See [`Component::file`].
    pub file: usize,
}

/// One component feeding another.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: String,
    /// The named output the events come out of, or `None` for the default one.
    pub output: Option<String>,
    pub to: String,
    /// The `inputs` entry that declared this edge.
    pub range: Range,
    /// The file `range` is in: the consumer's.
    pub file: usize,
}

/// A configuration, resolved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub components: Vec<Component>,
    pub edges: Vec<Edge>,
    pub findings: Vec<Finding>,
}

/// Resolves a config into a graph.
#[must_use]
pub fn build(document: Document) -> Graph {
    let Document {
        components,
        relaxed_wildcards,
    } = document;

    let mut edges = Vec::new();
    let mut findings = Vec::new();

    // Built once rather than searched for per input. A pipeline of a few
    // hundred components has a few hundred inputs, and a linear scan for each
    // of them is the difference between redrawing between keystrokes and not.
    // A duplicate name resolves to its first declaration, which is the reading
    // the linear scan gave and is reported separately anyway.
    let mut index: HashMap<&str, usize> = HashMap::with_capacity(components.len());
    for (position, component) in components.iter().enumerate() {
        index.entry(component.id.as_str()).or_insert(position);
    }
    // Every output as Vector writes it, in declaration order: the list every
    // input pattern is matched against.
    let written: Vec<(usize, Option<&String>, String)> = components
        .iter()
        .enumerate()
        .flat_map(|(position, component)| {
            component
                .outputs()
                .map(move |(port, name)| (position, port, name))
        })
        .collect();

    for consumer in &components {
        // Resolved per consumer, so a repeat can be spotted and dropped before
        // it reaches the picture. Vector checks the same thing in the same
        // place: `expand_globs` runs first and `check_shape` counts what comes
        // out of it, which is why `inputs = ["app", "app*"]` is a repeat too
        // even though nothing is written twice.
        let mut taken: HashSet<String> = HashSet::new();
        for input in &consumer.inputs {
            let mut resolved = Vec::new();
            resolve(
                input,
                consumer,
                &components,
                &index,
                &written,
                relaxed_wildcards,
                &mut resolved,
                &mut findings,
            );

            for edge in resolved {
                let written = match &edge.output {
                    Some(output) => format!("{}.{output}", edge.from),
                    None => edge.from.clone(),
                };
                if taken.contains(&written) {
                    findings.push(Finding {
                        severity: Severity::Error,
                        message: format!(
                            "`{}` takes `{written}` more than once, which Vector rejects",
                            consumer.id,
                        ),
                        range: input.range,
                        file: consumer.file,
                    });
                    continue;
                }
                taken.insert(written);
                edges.push(edge);
            }
        }
    }

    dotted_names(&components, &mut findings);
    duplicate_names(&components, &mut findings);
    inputs(&components, &mut findings);
    shape(&components, &mut findings);
    cycles(&components, &edges, &index, &mut findings);
    unconsumed(&components, &edges, &mut findings);

    Graph {
        components,
        edges,
        findings,
    }
}

/// The part of `graph` events can take through `id`: everything that can
/// reach it, everything it can reach, and the edges between them.
///
/// For a large pipeline this is usually the question being asked — "where do
/// the nginx logs go?" — and the answer is a handful of components out of
/// dozens. The findings are kept whole: a problem elsewhere in the pipeline
/// is still a problem, and the list under the graph still says so.
///
/// `None` when no component is called `id`.
#[must_use]
pub fn focus(graph: &Graph, id: &str) -> Option<Graph> {
    if !graph.components.iter().any(|c| c.id == id) {
        return None;
    }

    // One pass over the edges to build the neighbours, then a walk: scanning
    // every edge for every node visited is the same answer for quadratic work,
    // and focusing is what a large pipeline is for.
    let mut neighbours: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut backwards: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        neighbours.entry(&edge.from).or_default().push(&edge.to);
        backwards.entry(&edge.to).or_default().push(&edge.from);
    }

    let walk = |forward: bool| {
        let from = if forward { &neighbours } else { &backwards };
        let mut seen: HashSet<&str> = HashSet::from([id]);
        let mut queue = vec![id];
        while let Some(current) = queue.pop() {
            for &far in from.get(current).into_iter().flatten() {
                if seen.insert(far) {
                    queue.push(far);
                }
            }
        }
        seen
    };
    let upstream = walk(false);
    let downstream = walk(true);
    let kept = |name: &str| upstream.contains(name) || downstream.contains(name);

    Some(Graph {
        components: graph.components.iter().filter(|c| kept(&c.id)).cloned().collect(),
        // Only edges that lie on a path through `id`: both ends upstream, or
        // both downstream. A sibling feeding the same sink is not on the way.
        edges: graph
            .edges
            .iter()
            .filter(|e| {
                (upstream.contains(e.from.as_str()) && upstream.contains(e.to.as_str()))
                    || (downstream.contains(e.from.as_str()) && downstream.contains(e.to.as_str()))
            })
            .cloned()
            .collect(),
        findings: graph.findings.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve(
    input: &crate::config::Input,
    consumer: &Component,
    components: &[Component],
    index: &HashMap<&str, usize>,
    written: &[(usize, Option<&String>, String)],
    relaxed_wildcards: bool,
    edges: &mut Vec<Edge>,
    findings: &mut Vec<Finding>,
) {
    // A pattern is matched the way Vector's `expand_globs` matches it: against
    // every output of every component, each written as Vector writes an output
    // — `id` for a default output, `id.port` for a named one. So
    // `*_route.errors` picks the `errors` output of every router it names, and
    // `app*` takes `app.dropped` along with `app`. The consumer's own name is
    // the one thing left out.
    if is_glob(&input.text) {
        let matched: Vec<(&Component, Option<&String>)> = written
            .iter()
            .filter(|(_, _, name)| *name != consumer.id)
            .filter(|(_, _, name)| matches_glob(&input.text, name))
            .map(|&(producer, output, _)| (&components[producer], output))
            .collect();

        if matched.is_empty() {
            // `wildcard_matching: relaxed` is the config saying it knows: a
            // pattern is allowed to match nothing, so Vector starts and this
            // is not a finding at all.
            if !relaxed_wildcards {
                findings.push(Finding {
                    severity: Severity::Error,
                    // A wildcard matching nothing reads exactly like one
                    // matching everything, so silence would be the wrong
                    // answer.
                    message: format!(
                        "`{}` matches no output of any component, so `{}` has no inputs from it",
                        input.text, consumer.id,
                    ),
                    range: input.range,
                    file: consumer.file,
                });
            }
            return;
        }

        for (producer, output) in matched {
            edges.push(Edge {
                from: producer.id.clone(),
                output: output.cloned(),
                to: consumer.id.clone(),
                range: input.range,
                file: consumer.file,
            });
        }
        return;
    }

    // Vector looks an input up whole, in a map keyed by the written form of
    // every output (`Graph::input_map`), so the bare name is tried before the
    // dotted reading. A name with a dot in it is rejected outright by
    // `check_names`, which `dotted_names` below reports, so the two readings
    // never actually compete on a config Vector would accept.
    if let Some(&producer) = index.get(input.text.as_str()) {
        push_default_edge(&components[producer], consumer, input, edges, findings);
        return;
    }

    if let Some((id, output)) = input.text.rsplit_once('.') {
        match index.get(id).map(|&position| &components[position]) {
            Some(producer) if producer.named_outputs.iter().any(|o| o == output) => {
                edges.push(Edge {
                    from: producer.id.clone(),
                    output: Some(output.to_owned()),
                    to: consumer.id.clone(),
                    range: input.range,
                    file: consumer.file,
                });
            }
            Some(producer) => findings.push(Finding {
                severity: Severity::Error,
                message: if producer.named_outputs.is_empty() {
                    format!(
                        "`{id}` has no named outputs, so `{}` names nothing",
                        input.text,
                    )
                } else {
                    format!(
                        "`{id}` has no output `{output}`; it offers {}",
                        list(&producer.named_outputs),
                    )
                },
                range: input.range,
                file: consumer.file,
            }),
            None => findings.push(dangling(&input.text, input.range, consumer.file)),
        }
        return;
    }

    findings.push(dangling(&input.text, input.range, consumer.file));
}

fn push_default_edge(
    producer: &Component,
    consumer: &Component,
    input: &crate::config::Input,
    edges: &mut Vec<Edge>,
    findings: &mut Vec<Finding>,
) {
    if producer.id == consumer.id {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{}` reads from itself", consumer.id),
            range: input.range,
            file: consumer.file,
        });
        return;
    }

    if !producer.has_outputs() {
        findings.push(Finding {
            severity: Severity::Error,
            message: match producer.role {
                Role::Sink => format!("`{}` is a sink, so nothing can read from it", producer.id),
                // Worth spelling out: an enrichment table is reached from VRL,
                // by name, not by being wired into the pipeline.
                Role::Table => format!(
                    "`{}` is an enrichment table; VRL looks it up by name instead of reading it as an input",
                    producer.id,
                ),
                Role::Source | Role::Transform => {
                    format!("`{}` has no outputs at all", producer.id)
                }
            },
            range: input.range,
            file: consumer.file,
        });
        return;
    }

    // A router, and an `opentelemetry` source, have no default output, so
    // their bare name is not an output Vector knows ("doesn't match any
    // components"). Which ones they do have is the useful thing to say.
    if !producer.default_output {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!(
                "`{}` has no default output, so an input has to name one of its outputs: {}",
                producer.id,
                producer
                    .named_outputs
                    .iter()
                    .map(|output| format!("`{}.{output}`", producer.id))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            range: input.range,
            file: consumer.file,
        });
        return;
    }

    edges.push(Edge {
        from: producer.id.clone(),
        output: None,
        to: consumer.id.clone(),
        range: input.range,
        file: consumer.file,
    });
}

fn dangling(text: &str, range: Range, file: usize) -> Finding {
    Finding {
        severity: Severity::Error,
        message: format!("no component is called `{text}`"),
        range,
        file,
    }
}

/// A component whose name contains a dot.
///
/// Vector rejects it before anything else happens (`check_names`), because a
/// dot is how an input names one output of a component: a component called
/// `app.logs` and an `app` with a `logs` output would be written the same way
/// and there would be no telling them apart.
fn dotted_names(components: &[Component], findings: &mut Vec<Finding>) {
    for component in components {
        if component.id.contains('.') {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!(
                    "a component cannot be called `{}`: a dot in a name is how an input picks one output of a component",
                    component.id,
                ),
                range: component.range,
                file: component.file,
            });
        }
    }
}

/// Two components with one name.
///
/// Vector refuses to start on it, whichever way round it happens: two
/// sources with the same ID in two files, or a source and a sink sharing one
/// ("More than one component with name ..."). Within one file the YAML or TOML
/// parser usually catches it first; across files only this can. Every
/// declaration is marked, because which one is the mistake is not for a
/// reader of the config to decide.
fn duplicate_names(components: &[Component], findings: &mut Vec<Finding>) {
    let mut counts: HashMap<&str, usize> = HashMap::with_capacity(components.len());
    for component in components {
        *counts.entry(component.id.as_str()).or_default() += 1;
    }

    for component in components {
        let count = counts.get(component.id.as_str()).copied().unwrap_or(1);
        if count > 1 {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!(
                    "{count} components are called `{}`, and Vector requires every name to be unique",
                    component.id,
                ),
                range: component.range,
                file: component.file,
            });
        }
    }
}

/// A transform or a sink with nothing feeding it.
///
/// `check_shape` refuses to start on it ("Sink \"out\" has no inputs"), an
/// empty list and a missing key alike. An enrichment table is left out, as
/// Vector leaves it out: it is not among `config.sinks` when the check runs,
/// and a table nothing writes into is ordinary — it is loaded from a file, or
/// filled by VRL.
fn inputs(components: &[Component], findings: &mut Vec<Finding>) {
    for component in components {
        if component.role.needs_inputs() && component.inputs.is_empty() {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!(
                    "`{}` has no inputs, so nothing reaches it and Vector will not start",
                    component.id,
                ),
                range: component.range,
                file: component.file,
            });
        }
    }
}

/// A pipeline with nowhere for events to come from, or nowhere for them to go.
///
/// `check_shape` again, and it is about the pipeline rather than any one
/// component, so there is no line to point at: the range is the start of the
/// first file. An entirely empty config says nothing — that is every config
/// for its first few seconds, and Vector's own answer to it is the
/// `--allow-empty-config` flag rather than a fact about the file.
///
/// An enrichment table is not a sink here, exactly as in Vector: `check_shape`
/// runs on the builder, before a `memory` table is folded in among the sinks.
fn shape(components: &[Component], findings: &mut Vec<Finding>) {
    if components.is_empty() {
        return;
    }

    let has = |role: Role| components.iter().any(|component| component.role == role);
    for (role, missing) in [
        (Role::Source, "no sources, so no events ever enter the pipeline"),
        (Role::Sink, "no sinks, so no events ever leave it"),
    ] {
        if !has(role) {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!("this pipeline declares {missing}"),
                range: Range::default(),
                file: 0,
            });
        }
    }
}

/// Outputs whose events go nowhere.
///
/// Per output, not per component, because that is what Vector warns about
/// ("Transform \"split._unmatched\" has no consumers") and because it is the
/// interesting case: a `route` with three routes wired to one sink looks
/// finished and quietly throws two thirds of its events away. A component
/// events only end at — a sink, an enrichment table — has no outputs, so it is
/// never reported.
fn unconsumed(components: &[Component], edges: &[Edge], findings: &mut Vec<Finding>) {
    // The outputs something reads, written the way an input names them, so
    // this is one pass over the edges rather than one pass per output.
    let mut read: HashSet<String> = HashSet::with_capacity(edges.len());
    for edge in edges {
        read.insert(match &edge.output {
            Some(output) => format!("{}.{output}", edge.from),
            None => edge.from.clone(),
        });
    }

    for component in components {
        for (_, written) in component.outputs() {
            if read.contains(&written) {
                continue;
            }

            findings.push(Finding {
                severity: Severity::Warning,
                message: format!("nothing reads `{written}`, so the events it produces go nowhere"),
                range: component.range,
                file: component.file,
            });
        }
    }
}

/// Components that feed each other in a loop.
///
/// Vector's topology is a DAG; a cycle has no order to run in and no order to
/// draw in either, so this is reported once per component involved rather than
/// letting the renderer meet it unprepared.
///
/// A component is in a loop exactly when it can reach itself, which is exactly
/// when its strongly connected component holds more than one node — so this is
/// Tarjan's algorithm, run once over the whole graph. Asking "can this reach
/// itself?" of each component in turn is the same answer for cubic work: a
/// chain of 400 took 212ms that way, which is longer than the pause between
/// two keystrokes.
///
/// Iterative, not recursive, because the depth is the length of the longest
/// path: a pipeline with a thousand transforms in a row would otherwise
/// overflow the stack, and in wasm that is a trap the whole module has to be
/// rebuilt from.
fn cycles(
    components: &[Component],
    edges: &[Edge],
    index: &HashMap<&str, usize>,
    findings: &mut Vec<Finding>,
) {
    let count = components.len();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); count];
    for edge in edges {
        if let (Some(&from), Some(&to)) = (
            index.get(edge.from.as_str()),
            index.get(edge.to.as_str()),
        ) {
            adjacency[from].push(to);
        }
    }

    for position in strongly_connected(&adjacency) {
        let component = &components[position];
        findings.push(Finding {
            severity: Severity::Error,
            message: format!(
                "`{}` is part of a loop, and a Vector topology cannot have one",
                component.id,
            ),
            range: component.range,
            file: component.file,
        });
    }
}

/// The nodes that lie on a cycle, in declaration order.
///
/// Tarjan's algorithm, written with an explicit stack of (node, next edge to
/// follow) instead of recursion. A component of more than one node is a cycle;
/// a single node is one only if it points at itself, which the graph never
/// builds — a self-reading component is reported before it becomes an edge —
/// but which costs one comparison to be right about.
fn strongly_connected(adjacency: &[Vec<usize>]) -> Vec<usize> {
    const UNVISITED: usize = usize::MAX;

    let count = adjacency.len();
    let mut order = vec![UNVISITED; count];
    let mut low = vec![0usize; count];
    let mut on_stack = vec![false; count];
    let mut in_cycle = vec![false; count];
    let mut stack: Vec<usize> = Vec::new();
    let mut path: Vec<(usize, usize)> = Vec::new();
    let mut next = 0usize;

    for root in 0..count {
        if order[root] != UNVISITED {
            continue;
        }

        order[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        path.push((root, 0));

        while let Some(&(node, edge)) = path.last() {
            if edge < adjacency[node].len() {
                path.last_mut().expect("just read").1 += 1;
                let to = adjacency[node][edge];
                if order[to] == UNVISITED {
                    order[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    path.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(order[to]);
                }
                continue;
            }

            path.pop();
            if let Some(&(parent, _)) = path.last() {
                low[parent] = low[parent].min(low[node]);
            }

            if low[node] == order[node] {
                let mut members = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    members.push(member);
                    if member == node {
                        break;
                    }
                }
                if members.len() > 1 || adjacency[node].contains(&node) {
                    for member in members {
                        in_cycle[member] = true;
                    }
                }
            }
        }
    }

    (0..count).filter(|&node| in_cycle[node]).collect()
}

/// Whether an input is a pattern rather than a name. Vector passes every
/// input through `glob::Pattern`; one without these characters can only match
/// itself, which the exact lookup handles with better error messages.
fn is_glob(input: &str) -> bool {
    input.contains(['*', '?', '['])
}

/// Matches a pattern the way `glob::Pattern::matches` does with its default
/// options, which is what Vector's `expand_globs` uses: `*` is any run of
/// characters (dots included), `?` any one character, `[abc]`, `[a-z]` and
/// `[!abc]` a class. A `[` with no closing `]` is taken literally, where the
/// glob crate would reject the pattern and Vector would fall back to the
/// literal string.
fn matches_glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob_from(&pattern, &text)
}

fn glob_from(pattern: &[char], text: &[char]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else {
        return text.is_empty();
    };

    match first {
        '*' => (0..=text.len()).any(|skip| glob_from(rest, &text[skip..])),
        '?' => !text.is_empty() && glob_from(rest, &text[1..]),
        '[' => match class(rest) {
            Some((matches, after)) => {
                !text.is_empty() && matches(text[0]) && glob_from(after, &text[1..])
            }
            None => text.first() == Some(&'[') && glob_from(rest, &text[1..]),
        },
        literal => text.first() == Some(&literal) && glob_from(rest, &text[1..]),
    }
}

/// Parses a `[...]` class whose `[` is already consumed, returning its test
/// and what follows the `]`, or `None` when it is never closed.
fn class(pattern: &[char]) -> Option<(impl Fn(char) -> bool, &[char])> {
    let (negated, body) = match pattern.first() {
        Some('!') => (true, &pattern[1..]),
        _ => (false, pattern),
    };
    // A `]` right after the opening is a member, not the end.
    let close = body.iter().skip(1).position(|&c| c == ']')? + 1;
    let members: Vec<char> = body[..close].to_vec();
    let after = &body[close + 1..];

    let test = move |c: char| {
        let mut hit = false;
        let mut i = 0;
        while i < members.len() {
            if i + 2 < members.len() && members[i + 1] == '-' {
                hit |= members[i] <= c && c <= members[i + 2];
                i += 3;
            } else {
                hit |= members[i] == c;
                i += 1;
            }
        }
        hit != negated
    };
    Some((test, after))
}

fn list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::matches_glob;

    #[test]
    fn a_trailing_star_matches_a_prefix() {
        assert!(matches_glob("parse_*", "parse_json"));
        assert!(matches_glob("parse_*", "parse_"));
        assert!(!matches_glob("parse_*", "app_logs"));
    }

    #[test]
    fn a_leading_star_matches_a_suffix() {
        assert!(matches_glob("*_logs", "app_logs"));
        assert!(!matches_glob("*_logs", "logs_app"));
    }

    #[test]
    fn a_star_in_the_middle_matches_across() {
        assert!(matches_glob("parse*logs", "parse_app_logs"));
        assert!(!matches_glob("parse*logs", "parse_app_events"));
    }

    /// The tail has to be the end of the ID. Without that, `parse_*_json`
    /// would match `parse_a_json_extra`, and an input would claim a component
    /// it does not name.
    #[test]
    fn the_tail_is_anchored() {
        assert!(matches_glob("parse_*_json", "parse_a_json"));
        assert!(!matches_glob("parse_*_json", "parse_a_json_extra"));
    }

    #[test]
    fn a_bare_star_matches_everything() {
        assert!(matches_glob("*", "anything"));
        assert!(matches_glob("*", ""));
    }
}
