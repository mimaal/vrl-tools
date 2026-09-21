//! Turning components into a graph, and saying what is wrong with it.
//!
//! [`crate::config`] reads what the file says. This works out what it means:
//! which component feeds which, through which output, and which inputs name
//! something that is not there.
//!
//! The findings are the part `vector graph` cannot give you. It draws a
//! topology it has already validated, because Vector refused to start
//! otherwise. Here the config is being typed, so the interesting states are
//! precisely the ones that do not work yet.

use editor_text::Range;

use crate::config::{Component, Role};

/// How much a finding matters.
///
/// The line is whether Vector would start. An input naming nothing is fatal;
/// a transform nobody reads runs happily and throws its work away, which is
/// worse to debug and less urgent to fix.
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
    /// Where to point: the input that does not resolve, or the component that
    /// nobody reads.
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

/// Resolves components into a graph.
#[must_use]
pub fn build(components: Vec<Component>) -> Graph {
    let mut edges = Vec::new();
    let mut findings = Vec::new();

    for consumer in &components {
        for input in &consumer.inputs {
            resolve(input, consumer, &components, &mut edges, &mut findings);
        }
    }

    duplicates(&components, &mut findings);
    orphans(&components, &edges, &mut findings);
    cycles(&components, &edges, &mut findings);

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

    let walk = |forward: bool| {
        let mut seen = vec![id.to_owned()];
        let mut queue = vec![id.to_owned()];
        while let Some(current) = queue.pop() {
            for edge in &graph.edges {
                let (near, far) = if forward { (&edge.from, &edge.to) } else { (&edge.to, &edge.from) };
                if *near == current && !seen.contains(far) {
                    seen.push(far.clone());
                    queue.push(far.clone());
                }
            }
        }
        seen
    };
    let upstream = walk(false);
    let downstream = walk(true);
    let kept = |name: &String| upstream.contains(name) || downstream.contains(name);

    Some(Graph {
        components: graph.components.iter().filter(|c| kept(&c.id)).cloned().collect(),
        // Only edges that lie on a path through `id`: both ends upstream, or
        // both downstream. A sibling feeding the same sink is not on the way.
        edges: graph
            .edges
            .iter()
            .filter(|e| {
                (upstream.contains(&e.from) && upstream.contains(&e.to))
                    || (downstream.contains(&e.from) && downstream.contains(&e.to))
            })
            .cloned()
            .collect(),
        findings: graph.findings.clone(),
    })
}

fn resolve(
    input: &crate::config::Input,
    consumer: &Component,
    components: &[Component],
    edges: &mut Vec<Edge>,
    findings: &mut Vec<Finding>,
) {
    // A pattern is matched the way Vector's `expand_globs` matches it: against
    // every output of every source and transform, each written as Vector
    // writes an output — `id` for a default output, `id.port` for a named one.
    // So `*_route.errors` picks the `errors` output of every router it names,
    // and `app*` takes `app.dropped` along with `app`. The consumer's own
    // name is the one thing left out.
    if is_glob(&input.text) {
        let matched: Vec<(&Component, Option<&String>)> = outputs_of(components)
            .filter(|(_, _, written)| *written != consumer.id)
            .filter(|(_, _, written)| matches_glob(&input.text, written))
            .map(|(producer, output, _)| (producer, output))
            .collect();

        if matched.is_empty() {
            findings.push(Finding {
                severity: Severity::Error,
                // A wildcard matching nothing reads exactly like one matching
                // everything, so silence would be the wrong answer.
                message: format!(
                    "`{}` matches no source or transform, so `{}` has no inputs from it",
                    input.text, consumer.id,
                ),
                range: input.range,
                file: consumer.file,
            });
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

    // An ID is tried whole before it is split, because a component may legally
    // be named with a dot in it and that reading has to win over one that
    // invents an output.
    if let Some(producer) = components.iter().find(|c| c.id == input.text) {
        push_default_edge(producer, consumer, input, edges, findings);
        return;
    }

    if let Some((id, output)) = input.text.rsplit_once('.') {
        match components.iter().find(|c| c.id == id) {
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
    if producer.role == Role::Sink {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{}` is a sink, so nothing can read from it", producer.id),
            range: input.range,
            file: consumer.file,
        });
        return;
    }

    if producer.id == consumer.id {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{}` reads from itself", consumer.id),
            range: input.range,
            file: consumer.file,
        });
        return;
    }

    // A router has no default output, so its bare name is not an output
    // Vector knows ("doesn't match any components"). Which ones it does have
    // is the useful thing to say.
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

/// Two components with one name.
///
/// Vector refuses to start on it, whichever way round it happens: two
/// sources with the same ID in two files, or a source and a sink sharing one
/// ("More than one component with name ..."). Within one file the YAML or TOML
/// parser usually catches it first; across files only this can. Every
/// declaration is marked, because which one is the mistake is not for a
/// reader of the config to decide.
fn duplicates(components: &[Component], findings: &mut Vec<Finding>) {
    for component in components {
        let count = components.iter().filter(|other| other.id == component.id).count();
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

/// Components whose events go nowhere.
///
/// A sink is where events stop, so it is never orphaned. Everything else
/// produces events for somebody, and if nobody reads it the work is done and
/// discarded — which Vector will start up and do, quietly, forever.
fn orphans(components: &[Component], edges: &[Edge], findings: &mut Vec<Finding>) {
    for component in components {
        if component.role == Role::Sink {
            continue;
        }

        if edges.iter().any(|edge| edge.from == component.id) {
            continue;
        }

        findings.push(Finding {
            severity: Severity::Warning,
            message: format!(
                "nothing reads `{}`, so the events it produces go nowhere",
                component.id,
            ),
            range: component.range,
            file: component.file,
        });
    }
}

/// Components that feed each other in a loop.
///
/// Vector's topology is a DAG; a cycle has no order to run in and no order to
/// draw in either, so this is reported once per component involved rather than
/// letting the renderer meet it unprepared.
fn cycles(components: &[Component], edges: &[Edge], findings: &mut Vec<Finding>) {
    let mut in_cycle: Vec<&str> = Vec::new();

    for component in components {
        if reaches(&component.id, &component.id, edges, &mut Vec::new()) {
            in_cycle.push(&component.id);
        }
    }

    for id in in_cycle {
        let (range, file) = components
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| (Range::default(), 0), |c| (c.range, c.file));

        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{id}` is part of a loop, and a Vector topology cannot have one"),
            range,
            file,
        });
    }
}

/// Whether `target` is reachable from `from`, following edges.
fn reaches<'a>(from: &str, target: &str, edges: &'a [Edge], seen: &mut Vec<&'a str>) -> bool {
    for edge in edges.iter().filter(|edge| edge.from == from) {
        if edge.to == target {
            return true;
        }
        if seen.contains(&edge.to.as_str()) {
            continue;
        }
        seen.push(&edge.to);
        if reaches(&edge.to, target, edges, seen) {
            return true;
        }
    }
    false
}

/// Every output a component can be read from, as Vector writes it: `id` for
/// the default output of a component that has one, `id.port` for each named
/// output. Sinks have none.
fn outputs_of(components: &[Component]) -> impl Iterator<Item = (&Component, Option<&String>, String)> {
    components
        .iter()
        .filter(|component| component.role != Role::Sink)
        .flat_map(|component| {
            let default = component
                .default_output
                .then(|| (component, None, component.id.clone()));
            let named = component
                .named_outputs
                .iter()
                .map(move |output| (component, Some(output), format!("{}.{output}", component.id)));
            default.into_iter().chain(named)
        })
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
