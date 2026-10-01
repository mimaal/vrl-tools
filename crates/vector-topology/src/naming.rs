//! A naming convention for components, when a team has one.
//!
//! Vector does not care what a component is called, beyond a name having no
//! dot in it ([`crate::graph`]). A team often does: every `remap` that
//! normalizes a product ends in `-normalizer`, so that `*-normalizer` in an
//! `inputs` picks them all up — and one that is named differently is not
//! picked up, silently. That is worth saying, but only to whoever asked.
//!
//! So this checks nothing unless it is given a pattern, it is given one per
//! component `type`, and what it finds is an `info`: the config runs.
//!
//! The patterns are regular expressions as the `regex` crate reads them,
//! unanchored unless they anchor themselves. That is not quite the dialect a
//! JavaScript settings file suggests — there is no lookaround and no
//! backreference — and a pattern that does not compile is reported, once,
//! rather than quietly matching nothing.

use editor_text::Range;
use regex::Regex;

use crate::config::Component;
use crate::graph::{Finding, Severity};

/// A finding for every component whose name does not match the pattern for
/// its type, and one for every pattern that is not a regular expression.
pub(crate) fn check(components: &[Component], patterns: &[(String, String)]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut compiled: Vec<(&str, &str, Regex)> = Vec::with_capacity(patterns.len());

    for (kind, pattern) in patterns {
        match Regex::new(pattern) {
            Ok(regex) => compiled.push((kind, pattern, regex)),
            // About a setting, not about a place in the config: at the top of
            // the first file, like the other findings with no line.
            Err(error) => findings.push(Finding {
                severity: Severity::Info,
                message: format!(
                    "the name pattern for `{kind}` components, `{pattern}`, is not a regular expression, so no name is checked against it: {}",
                    reason(&error),
                ),
                range: Range::default(),
                file: 0,
            }),
        }
    }

    // In the order of the components, so the findings read like the config.
    for component in components {
        for (kind, pattern, regex) in &compiled {
            if component.component_type == *kind && !regex.is_match(&component.id) {
                findings.push(Finding {
                    severity: Severity::Info,
                    message: format!(
                        "`{}` does not match the name pattern for `{kind}` components, `{pattern}`",
                        component.id,
                    ),
                    range: component.range,
                    file: component.file,
                });
            }
        }
    }

    findings
}

/// The last line of a `regex` error, which is the one that says what is
/// wrong; the rest draws the pattern with a caret under it.
fn reason(error: &regex::Error) -> String {
    let text = error.to_string();
    text.lines()
        .last()
        .unwrap_or(&text)
        .trim()
        .trim_start_matches("error: ")
        .to_owned()
}
