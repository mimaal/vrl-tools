//! Compiles VRL with the real compiler and reports what it says, at positions
//! an editor can use.
//!
//! This crate knows nothing about WebAssembly or VS Code. It takes a program as
//! text and gives back a serialisable answer, so the same core can sit behind a
//! `wasm-bindgen` wrapper today and behind a language server tomorrow.
//!
//! The rule that makes the whole thing worth building: no heuristics. Every
//! diagnostic here comes from `vrl::compiler::compile`, which is the same code
//! path `vector validate` takes. If this crate accepts a program, Vector
//! accepts it — for the pinned version of the language, which is
//! [`VRL_VERSION`].

mod stdlib;
mod text;

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use vrl::compiler::Function;
use vrl::diagnostic::{Diagnostic as VrlDiagnostic, DiagnosticList, Severity as VrlSeverity};

pub use stdlib::{
    stdlib, stdlib_json, Closure, Example, Function as StdlibFunction, Parameter, Stdlib,
};
pub use text::{LineIndex, Position, Range};

/// The version of the `vrl` crate this checker compiles against.
///
/// Pinned exactly in the workspace manifest, and shown to the user, because a
/// program that compiles here still fails in production if the Vector running
/// there speaks an older VRL. `version_matches_the_pin` keeps this honest.
pub const VRL_VERSION: &str = "0.29.0";

/// Error codes with a page on <https://errors.vrl.dev>. The crate itself only
/// links this range, and a link to a 404 is worse than no link.
const DOCUMENTED_CODES: std::ops::RangeInclusive<usize> = 100..=110;

/// What the compiler had to say about a program.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// `true` when the program compiled. Warnings do not make it `false`.
    pub compiled: bool,
    /// The `vrl` crate version that produced this answer.
    pub vrl_version: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// One diagnostic, already converted to editor coordinates.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    /// The compiler's error code, e.g. `103`.
    pub code: usize,
    pub message: String,
    /// Where to underline: the primary label's span, or the first label's, or
    /// the start of the document when the diagnostic carries no span at all.
    pub range: Range,
    /// Every label the compiler attached, primary one included. These become
    /// `relatedInformation` in the editor rather than being glued onto the
    /// message.
    pub labels: Vec<Label>,
    /// Rendered notes: hints, examples and documentation links.
    pub notes: Vec<String>,
    /// The page for this error code, when one exists.
    pub documentation_url: Option<String>,
}

/// A span the compiler pointed at while explaining a diagnostic.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub message: String,
    pub primary: bool,
    pub range: Range,
}

/// Mirrors `vrl::diagnostic::Severity`.
///
/// `Bug` means the compiler itself misbehaved; it is kept separate from `Error`
/// here so a client can tell the user that, rather than blaming their program.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Bug,
    Error,
    Warning,
    Note,
}

impl From<VrlSeverity> for Severity {
    fn from(severity: VrlSeverity) -> Self {
        match severity {
            VrlSeverity::Bug => Self::Bug,
            VrlSeverity::Error => Self::Error,
            VrlSeverity::Warning => Self::Warning,
            VrlSeverity::Note => Self::Note,
        }
    }
}

/// The standard library, built once.
///
/// `vrl::stdlib::all()` boxes almost two hundred function objects on every
/// call, and this runs on every keystroke.
pub(crate) fn functions() -> &'static [Box<dyn Function>] {
    static FUNCTIONS: OnceLock<Vec<Box<dyn Function>>> = OnceLock::new();
    FUNCTIONS.get_or_init(vrl::stdlib::all)
}

/// Compiles `source` and reports the result.
///
/// `sample_event` is accepted but not used yet. It is in the signature from the
/// start because knowing the shape of `.` is what will eventually turn "this
/// might fail" into "this field does not exist" (phase 5 of the plan), and
/// retrofitting it would mean changing every layer above this one.
#[must_use]
pub fn check(source: &str, sample_event: Option<&str>) -> Check {
    let _ = sample_event;

    let index = LineIndex::new(source);

    let (compiled, diagnostics) = match vrl::compiler::compile(source, functions()) {
        Ok(result) => (true, convert(&result.warnings, &index)),
        Err(diagnostics) => (false, convert(&diagnostics, &index)),
    };

    Check {
        compiled,
        vrl_version: VRL_VERSION.to_owned(),
        diagnostics,
    }
}

/// Compiles `source` and returns the answer as JSON, which is the shape every
/// wrapper around this crate wants.
///
/// # Errors
///
/// Only if the answer fails to serialise, which would be a bug in this crate.
pub fn check_json(source: &str, sample_event: Option<&str>) -> serde_json::Result<String> {
    serde_json::to_string(&check(source, sample_event))
}

fn convert(diagnostics: &DiagnosticList, index: &LineIndex<'_>) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| convert_one(diagnostic, index))
        .collect()
}

fn convert_one(diagnostic: &VrlDiagnostic, index: &LineIndex<'_>) -> Diagnostic {
    let labels: Vec<Label> = diagnostic
        .labels
        .iter()
        .map(|label| Label {
            message: label.message.clone(),
            primary: label.primary,
            range: index.range(label.span.range()),
        })
        .collect();

    // Underline the primary label; fall back to the first label, since a
    // diagnostic pinned nowhere would be reported at the top of the file and
    // read as noise.
    let range = labels
        .iter()
        .find(|label| label.primary)
        .or_else(|| labels.first())
        .map_or_else(Range::default, |label| label.range);

    Diagnostic {
        severity: diagnostic.severity.into(),
        code: diagnostic.code,
        message: diagnostic.message.clone(),
        range,
        labels,
        notes: diagnostic
            .notes
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        documentation_url: DOCUMENTED_CODES
            .contains(&diagnostic.code)
            .then(|| format!("https://errors.vrl.dev/{}", diagnostic.code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(source: &str) -> Vec<Diagnostic> {
        check(source, None)
            .diagnostics
            .into_iter()
            .filter(|d| matches!(d.severity, Severity::Error | Severity::Bug))
            .collect()
    }

    #[test]
    fn a_valid_program_compiles() {
        let result = check(".status = to_int(.status) ?? 0\n", None);

        assert!(result.compiled, "diagnostics: {:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| !matches!(d.severity, Severity::Error | Severity::Bug)),
            "{:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn an_unhandled_fallible_assignment_is_an_error() {
        let result = check(".parsed = parse_json(.message)\n", None);

        assert!(!result.compiled);
        let error = result
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error)
            .expect("the compiler rejects an unhandled fallible assignment");

        // The exact wording belongs to the compiler, not to us; what this crate
        // owns is that the message survives intact and points somewhere real.
        assert!(!error.message.is_empty());
        assert_eq!(error.range.start.line, 0);
        assert!(error.range.end.character > error.range.start.character);
        assert!(error.labels.iter().any(|label| label.primary));
    }

    #[test]
    fn handling_the_error_makes_it_compile() {
        let source = "parsed, err = parse_json(.message)\nif err == null {\n  . = merge(., object!(parsed))\n}\n";

        assert!(errors(source).is_empty(), "{:?}", errors(source));
    }

    #[test]
    fn an_unknown_function_is_reported_where_it_is_written() {
        let source = ".x = 1\n.y = definitely_not_a_function(.x)\n";
        let diagnostics = errors(source);
        let error = diagnostics.first().expect("unknown function is an error");

        assert_eq!(error.range.start.line, 1);
        assert_eq!(error.range.start.character, 5);
    }

    #[test]
    fn a_syntax_error_is_reported_rather_than_panicking() {
        let diagnostics = errors(".foo = \n");

        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn spans_are_utf16_columns_not_byte_offsets() {
        // The accented words push the call eight bytes further along than it is
        // characters, which is exactly the drift that misplaces squiggles.
        let source = ".mensaje = \"café con leña\"\n.y = definitely_not_a_function(.mensaje)\n";
        let error = errors(source).into_iter().next().expect("an error");

        assert_eq!(error.range.start, Position::new(1, 5));
    }

    #[test]
    fn a_diagnostic_carries_its_notes() {
        let error = errors(".parsed = parse_json(.message)\n")
            .into_iter()
            .next()
            .expect("an error");

        assert!(
            !error.notes.is_empty(),
            "the compiler explains how to handle the error; keep those notes",
        );
    }

    #[test]
    fn documented_codes_get_a_link_and_others_do_not() {
        for diagnostic in errors(".parsed = parse_json(.message)\n") {
            match diagnostic.documentation_url {
                Some(url) => {
                    assert!(DOCUMENTED_CODES.contains(&diagnostic.code));
                    assert_eq!(url, format!("https://errors.vrl.dev/{}", diagnostic.code));
                }
                None => assert!(!DOCUMENTED_CODES.contains(&diagnostic.code)),
            }
        }
    }

    #[test]
    fn the_answer_serialises_as_camel_case_json() {
        let json = check_json(".x = 1\n", None).expect("serialises");

        assert!(json.contains("\"vrlVersion\":\"0.29.0\""), "{json}");
        assert!(json.contains("\"compiled\":true"), "{json}");
    }

    /// The pinned version appears in three places: the Cargo manifest that
    /// decides what is compiled, the constant this crate reports, and the
    /// package.json field the grammar generator reads. They must agree.
    #[test]
    fn version_matches_the_pin() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the crate lives two levels below the workspace root");

        let cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
        assert!(
            cargo.contains(&format!("vrl = {{ version = \"={VRL_VERSION}\"")),
            "Cargo.toml does not pin vrl {VRL_VERSION}",
        );

        let package = std::fs::read_to_string(root.join("package.json")).expect("package.json");
        assert!(
            package.contains(&format!("\"crateVersion\": \"{VRL_VERSION}\"")),
            "package.json does not record vrl {VRL_VERSION}",
        );
    }
}
