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

    orphans(&components, &edges, &mut findings);
    cycles(&components, &edges, &mut findings);

    Graph {
        components,
        edges,
        findings,
    }
}

fn resolve(
    input: &crate::config::Input,
    consumer: &Component,
    components: &[Component],
    edges: &mut Vec<Edge>,
    findings: &mut Vec<Finding>,
) {
    if input.text.contains('*') {
        let matched: Vec<&Component> = components
            .iter()
            .filter(|candidate| candidate.role != Role::Sink)
            .filter(|candidate| candidate.id != consumer.id)
            .filter(|candidate| matches_glob(&input.text, &candidate.id))
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
            });
            return;
        }

        for producer in matched {
            edges.push(Edge {
                from: producer.id.clone(),
                output: None,
                to: consumer.id.clone(),
                range: input.range,
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
            }),
            None => findings.push(dangling(&input.text, input.range)),
        }
        return;
    }

    findings.push(dangling(&input.text, input.range));
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
        });
        return;
    }

    if producer.id == consumer.id {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{}` reads from itself", consumer.id),
            range: input.range,
        });
        return;
    }

    edges.push(Edge {
        from: producer.id.clone(),
        output: None,
        to: consumer.id.clone(),
        range: input.range,
    });
}

fn dangling(text: &str, range: Range) -> Finding {
    Finding {
        severity: Severity::Error,
        message: format!("no component is called `{text}`"),
        range,
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
        let range = components
            .iter()
            .find(|c| c.id == id)
            .map_or_else(Range::default, |c| c.range);

        findings.push(Finding {
            severity: Severity::Error,
            message: format!("`{id}` is part of a loop, and a Vector topology cannot have one"),
            range,
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

/// Matches a Vector `inputs` pattern against a component ID.
///
/// Vector documents `*` and nothing else, so that is all this does. A pattern
/// with no `*` never reaches here.
fn matches_glob(pattern: &str, id: &str) -> bool {
    let mut parts = pattern.split('*');

    let Some(first) = parts.next() else {
        return false;
    };
    if !id.starts_with(first) {
        return false;
    }

    let mut rest = &id[first.len()..];
    let parts: Vec<&str> = parts.collect();

    for (position, part) in parts.iter().enumerate() {
        let last = position + 1 == parts.len();

        if last {
            // The tail has to land at the end, not merely somewhere after.
            return rest.len() >= part.len() && rest.ends_with(part);
        }

        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }

    true
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
