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

/// Examples that cannot compile outside the `vrl` checkout, with the reason.
///
/// Each entry is the function name plus the example title, so a rename shows
/// up as a failure here rather than silently widening the exemption — which is
/// exactly what the 0.29 -> 0.35 bump did, when `encode_proto`'s example was
/// retitled from "message" to "Encode to proto".
///
/// Two different things are going on, and they are listed separately because
/// collapsing them into "these ones fail" is how a real regression hides.
///
/// The first group builds a path to a fixture file with `CARGO_MANIFEST_DIR`
/// at compile time and pastes it into a VRL string literal. On Windows that
/// path arrives full of backslashes, which VRL reads as escape sequences, so
/// the example is a syntax error before the checker ever gets an opinion; on
/// other platforms it gets as far as looking for a file that is not shipped in
/// the published crate. The examples are fine, the way they embed a host path
/// is not portable.
const EMBEDS_A_HOST_PATH: &[(&str, &str)] = &[
    ("encode_proto", "Encode to proto"),
    ("parse_proto", "Parse proto"),
    ("validate_json_schema", "Payload contains a valid email"),
    (
        "validate_json_schema",
        "Payload contains a custom format declaration, with ignore_unknown_formats set to true",
    ),
];

/// The second group names a fixture by a path relative to the `vrl`
/// repository, and the function reads it while compiling, so the call is
/// rejected with E403 anywhere that repository is not the working directory.
/// Nothing about the example is wrong; it simply documents a feature whose
/// input is a file on disk.
const READS_A_REPOSITORY_FIXTURE: &[(&str, &str)] = &[
    ("parse_groks", "Parse using aliases from file"),
    ("parse_etld", "Parse eTLD with custom PSL"),
];

fn is_exempt(function: &str, title: &str) -> bool {
    let entry = (function, title);
    EMBEDS_A_HOST_PATH.contains(&entry) || READS_A_REPOSITORY_FIXTURE.contains(&entry)
}

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

            if is_exempt(function.identifier(), example.title) {
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
