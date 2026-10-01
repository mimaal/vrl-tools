//! Outputs that end on purpose.
//!
//! Vector warns about every output nothing reads (`validation::warnings`),
//! and so does the graph. Most of the time that is a route somebody forgot to
//! wire. Sometimes it is the design: a router's `_unmatched` that is meant to
//! be dropped, a tripwire route that should never match, a transform that
//! counts what reaches it and passes nothing on. Those warnings never go
//! away, and three warnings that are always there are how the fourth one —
//! the real one — gets missed.
//!
//! So an output can be marked as terminal, and then it is drawn as an end
//! rather than reported as a problem. Vector knows nothing of this and still
//! prints its warning; the mark is this editor's, about this config.
//!
//! Two ways to say it:
//!
//! - **In the config**, with a comment on the line the thing is written on:
//!   `# vrl-tools: terminal`. On the line that declares a component it marks
//!   every output the component has; on the line a route is written on — its
//!   key under `route`, its `name` under `routes`, or the
//!   `[[transforms.x.routes]]` header above that — it marks that route. An
//!   output nobody writes down (`_unmatched`, `dropped`) has no line of its
//!   own, so it is named after the marker, on the component's line:
//!   `# vrl-tools: terminal _unmatched`. Several names are separated by
//!   commas; the component's own name stands for its default output.
//! - **From outside**, with patterns matched against the output as an input
//!   would name it (`route_by_product._unmatched`) or against the component
//!   (`dropped-handler`, every output of it), with the same wildcards
//!   `inputs` takes. That is the `vrl-tools.terminalOutputs` setting, for a
//!   config that should not carry an editor's comments.
//!
//! A mark on an output something does read changes nothing. A comment that
//! is on no component's or route's line marks nothing, and says so: a mark
//! that silently does nothing is the failure this would otherwise have.

use editor_text::{LineIndex, Range};

use crate::config::{Component, Origin};
use crate::graph::{matches_glob, Finding, Severity};

/// What the comment says, after the `#`.
const MARK: &str = "vrl-tools:";
const TERMINAL: &str = "terminal";

/// One `# vrl-tools: terminal` comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) file: usize,
    /// Zero-based, in the file as written.
    pub(crate) line: u32,
    /// The outputs named after it; empty for "whatever is on this line".
    pub(crate) names: Vec<String>,
    /// The comment itself, for saying it marks nothing.
    pub(crate) range: Range,
}

/// The markers of a config file, YAML or TOML: both start a comment with `#`.
///
/// Found by reading lines, not by asking the parser: neither keeps comments
/// in a form that says which line they were on, and the text to find is
/// exact. A `#` inside a string that happens to be followed by these very
/// words would be read as a marker; it then marks whatever is declared on
/// that line, which for the inside of a multi-line string is nothing.
pub(crate) fn markers(source: &str) -> Vec<Marker> {
    let index = LineIndex::new(source);
    let mut found = Vec::new();
    let mut offset = 0;

    for (number, line) in source.split('\n').enumerate() {
        let start = offset;
        offset += line.len() + 1;

        let Some(hash) = marker_start(line) else { continue };
        let rest = line[hash + 1..].trim_start();
        let Some(rest) = rest.strip_prefix(MARK) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix(TERMINAL) else { continue };
        // `terminal`, not `terminalOutputs` or whatever else may come.
        if rest.chars().next().is_some_and(|next| !next.is_whitespace() && next != ',') {
            continue;
        }

        let end = line.trim_end().len();
        found.push(Marker {
            file: 0,
            line: u32::try_from(number).unwrap_or(u32::MAX),
            names: rest
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            range: index.range(start + hash..start + end),
        });
    }
    found
}

/// Where on a line the `#` of a marker is: the last `#` followed by the mark.
fn marker_start(line: &str) -> Option<usize> {
    line.match_indices('#')
        .map(|(position, _)| position)
        .rfind(|&position| line[position + 1..].trim_start().starts_with(MARK))
}

/// What of a component a marker can land on.
pub(crate) struct Subject<'a> {
    pub(crate) id: &'a str,
    /// Where the component is: the entry with its `type`.
    pub(crate) declared: Origin,
    /// Every entry it was merged from, the declaration included.
    pub(crate) pieces: &'a [Origin],
    pub(crate) default_output: bool,
    pub(crate) named: &'a [String],
    /// Every line a route is written on: its name, where, and the piece that
    /// writes it.
    pub(crate) spots: &'a [(String, Origin, Origin)],
}

/// The outputs of `subject` the markers mark, as an input would name them.
///
/// `used` is set for every marker that landed on the component, whatever it
/// then marked, so [`stray`] can report the ones that landed nowhere.
pub(crate) fn marked(
    markers: &[Marker],
    subject: &Subject<'_>,
    used: &mut [bool],
    findings: &mut Vec<Finding>,
) -> Vec<String> {
    let mut outputs: Vec<String> = Vec::new();
    let mut mark = |output: String| {
        if !outputs.contains(&output) {
            outputs.push(output);
        }
    };
    let named = |name: &str| format!("{}.{name}", subject.id);
    let on = |origin: &Origin, marker: &Marker| {
        origin.file == marker.file && origin.range.start.line == marker.line
    };

    for (position, marker) in markers.iter().enumerate() {
        let declared = on(&subject.declared, marker);
        let piece = subject.pieces.iter().find(|piece| on(piece, marker));
        let routes: Vec<&String> = subject
            .spots
            .iter()
            .filter(|(_, spot, _)| on(spot, marker))
            .map(|(name, _, _)| name)
            .collect();
        if piece.is_none() && routes.is_empty() {
            continue;
        }
        if let Some(seen) = used.get_mut(position) {
            *seen = true;
        }

        if !marker.names.is_empty() {
            for name in &marker.names {
                if subject.named.contains(name) {
                    mark(named(name));
                } else if name == subject.id && subject.default_output {
                    mark(subject.id.to_owned());
                } else {
                    findings.push(Finding {
                        severity: Severity::Info,
                        message: format!(
                            "`{name}` is not an output of `{}`, so this marks nothing; {}",
                            subject.id,
                            offers(subject),
                        ),
                        range: marker.range,
                        file: marker.file,
                    });
                }
            }
        } else if !routes.is_empty() {
            // The route written on this line, and nothing else of the
            // component: that is what a comment beside a route means.
            for name in routes {
                if subject.named.contains(name) {
                    mark(named(name));
                }
            }
        } else if declared {
            if subject.default_output {
                mark(subject.id.to_owned());
            }
            for name in subject.named {
                mark(named(name));
            }
        } else if let Some(piece) = piece {
            // The header of a file's piece of the component: the routes that
            // piece adds.
            for (name, _, part) in subject.spots {
                if part == piece && subject.named.contains(name) {
                    mark(named(name));
                }
            }
        }
    }
    outputs
}

fn offers(subject: &Subject<'_>) -> String {
    let mut names: Vec<String> = subject.named.iter().map(|name| format!("`{name}`")).collect();
    if subject.default_output {
        names.insert(0, format!("`{}` for its default output", subject.id));
    }
    if names.is_empty() {
        "it has no outputs".to_owned()
    } else {
        format!("it offers {}", names.join(", "))
    }
}

/// A finding for every marker that is on no component's and no route's line.
pub(crate) fn stray(markers: &[Marker], used: &[bool]) -> Vec<Finding> {
    markers
        .iter()
        .zip(used)
        .filter(|(_, &used)| !used)
        .map(|(marker, _)| Finding {
            severity: Severity::Info,
            message: "this `vrl-tools: terminal` is not on the line of a component or of a route, \
                      so it marks nothing"
                .to_owned(),
            range: marker.range,
            file: marker.file,
        })
        .collect()
}

/// Whether `output` of `component` ends on purpose: marked in the config, or
/// matched by one of the `patterns` from outside it.
#[must_use]
pub(crate) fn accepts(component: &Component, output: &str, patterns: &[String]) -> bool {
    component.terminal.iter().any(|marked| marked == output)
        || patterns.iter().any(|pattern| {
            let pattern = pattern.trim();
            !pattern.is_empty()
                && (matches_glob(pattern, output) || matches_glob(pattern, &component.id))
        })
}

#[cfg(test)]
mod tests {
    use super::markers;

    fn read(source: &str) -> Vec<(u32, Vec<String>)> {
        markers(source).into_iter().map(|marker| (marker.line, marker.names)).collect()
    }

    #[test]
    fn a_marker_is_a_comment_with_these_words() {
        assert_eq!(
            read("[transforms.a] # vrl-tools: terminal\ntype = \"remap\"\n#vrl-tools:terminal\n"),
            [(0, vec![]), (2, vec![])],
        );
    }

    #[test]
    fn names_after_it_are_the_outputs_it_marks() {
        assert_eq!(
            read("  split: # vrl-tools: terminal _unmatched, errors dropped\n"),
            [(0, vec!["_unmatched".to_owned(), "errors".to_owned(), "dropped".to_owned()])],
        );
    }

    #[test]
    fn other_comments_and_other_words_are_not_markers() {
        assert!(read("# terminal\n# vrl-tools: terminalOutputs\n# vrl-tools: ignore\nx = \"#\"\n").is_empty());
    }

    /// A `#` earlier on the line — in a value — does not hide the marker.
    #[test]
    fn the_marker_is_found_after_another_hash() {
        let found = markers("condition = '.tag == \"#1\"' # vrl-tools: terminal\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range.start.character, 27);
    }

    #[test]
    fn windows_line_endings_do_not_become_a_name() {
        assert_eq!(read("[transforms.a] # vrl-tools: terminal\r\ntype = \"remap\"\r\n"), [(0, vec![])]);
    }
}
