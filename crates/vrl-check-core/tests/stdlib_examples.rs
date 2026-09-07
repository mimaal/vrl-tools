//! Compiles every example the standard library ships.
//!
//! The `vrl` crate documents each function with runnable examples, and those
//! examples are the closest thing to a real corpus that can be had without
//! committing somebody's production parser to this repository. Every example
//! whose documented result is a value must compile; if one stops compiling
//! after a version bump, the language moved and this checker needs to move
//! with it.
//!
//! Examples whose documented result is an error are compiled too, but they are
//! only required not to panic: several of them fail at runtime rather than at
//! compile time, which is precisely the distinction this crate exists to draw.

use vrl_check_core::{check, Severity};

/// Examples that do not compile on their own, with the reason. Each entry is
/// the function name plus the example title, so a rename shows up as a failure
/// here rather than silently widening the exemption.
///
/// These three build a path to a fixture file with `CARGO_MANIFEST_DIR` at
/// compile time and paste it into a VRL string literal. On Windows that path
/// arrives full of backslashes, which VRL reads as escape sequences, so the
/// example is a syntax error before the checker ever gets an opinion. The
/// examples are fine; the way the crate embeds a host path into them is not
/// portable.
const NOT_SELF_CONTAINED: &[(&str, &str)] = &[
    ("encode_proto", "message"),
    ("parse_proto", "message"),
    ("validate_json_schema", "valid payload"),
];

fn is_error(source: &str) -> Option<String> {
    let result = check(source, None);
    result
        .diagnostics
        .into_iter()
        .find(|d| matches!(d.severity, Severity::Error | Severity::Bug))
        .map(|d| d.message)
}

#[test]
fn every_documented_example_compiles() {
    let mut failures = Vec::new();
    let mut checked = 0;

    for function in vrl::stdlib::all() {
        for example in function.examples() {
            if example.result.is_err() {
                // Documented failures: compiling must not panic, but the
                // program is allowed to be rejected.
                let _ = check(example.source, None);
                continue;
            }

            if NOT_SELF_CONTAINED.contains(&(function.identifier(), example.title)) {
                continue;
            }

            checked += 1;
            if let Some(message) = is_error(example.source) {
                failures.push(format!(
                    "{} / {}: {message}\n      {}",
                    function.identifier(),
                    example.title,
                    example.source,
                ));
            }
        }
    }

    assert!(
        checked > 200,
        "only {checked} examples were checked; the stdlib dump looks truncated",
    );

    assert!(
        failures.is_empty(),
        "{} of {checked} stdlib examples do not compile:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
}
