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
mod run;
pub(crate) mod sample;
mod text;

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use vrl::compiler::{CompileConfig, Function};
use vrl::diagnostic::{Diagnostic as VrlDiagnostic, DiagnosticList, Severity as VrlSeverity};

pub use run::{run, run_json, Run};
pub use stdlib::{
    stdlib, stdlib_json, Closure, Example, Function as StdlibFunction, Parameter, Stdlib,
};
pub use text::{LineIndex, Position, Range};

/// The version of the `vrl` crate this checker compiles against.
///
/// Pinned exactly in the workspace manifest, and shown to the user, because a
/// program that compiles here still fails in production if the Vector running
/// there speaks an older VRL. `version_matches_the_pin` keeps this honest.
pub const VRL_VERSION: &str = "0.35.0";

/// The Vector release that depends on exactly [`VRL_VERSION`].
///
/// It is what the status bar tooltip names, because nobody deploys a `vrl`
/// crate version — they deploy a Vector. Keeping it here rather than in the
/// TypeScript means there is one place to change and no second copy to drift:
/// the extension asks the module.
///
/// Vector's release number is not the crate's: Vector 0.58.0 is the release
/// whose `Cargo.lock` pins `vrl` 0.35.0. Some Vector releases (0.57.0, and
/// 0.50/0.51 before them) consume `vrl` from git `branch = main` and have no
/// pinnable crate version at all, so they are not candidates for this pin.
pub const VECTOR_RELEASE: &str = "0.58.0";

/// Error codes with a page on <https://errors.vrl.dev>. The crate itself only
/// links this range, and a link to a 404 is worse than no link.
const DOCUMENTED_CODES: std::ops::RangeInclusive<usize> = 100..=110;

/// The three diagnostics that say "your error handling is unnecessary":
/// `unnecessary error assignment` (104), `can't abort infallible function`
/// (620) and `unnecessary error coalescing operation` (651).
///
/// They matter here because a sample event makes more expressions infallible,
/// and so makes more error handling "unnecessary" — but only for that one
/// event. A parser that copes with a missing `.hostname` is not wrong because
/// today's sample happens to have one; it is the reason the parser survives
/// contact with production. So when one of these appears *because of* a
/// sample, it is reported as a warning: still worth knowing, not worth
/// stopping for. Every other diagnostic keeps the severity the compiler gave
/// it, and with no sample nothing is adjusted at all.
const OVER_DEFENSIVE: &[usize] = &[104, 620, 651];

/// What the compiler had to say about a program.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// `true` when the program compiled. Warnings do not make it `false`.
    pub compiled: bool,
    /// The `vrl` crate version that produced this answer.
    pub vrl_version: String,
    pub diagnostics: Vec<Diagnostic>,
    /// `true` when a sample event was supplied and the program was typed
    /// against its shape. Worth surfacing: it changes what counts as an error,
    /// so a person should be able to tell which of the two answers they are
    /// looking at.
    pub typed_with_sample: bool,
    /// Why the sample event was ignored, when one was supplied and could not
    /// be used. The program is still checked, against an unknown event.
    pub sample_error: Option<String>,
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
    /// `true` when this was an error the compiler raised only because a sample
    /// event narrowed the types, and it is one of the "your error handling is
    /// unnecessary" family, so it has been reported as a warning instead. See
    /// `OVER_DEFENSIVE`.
    #[serde(default)]
    pub relaxed_by_sample: bool,
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
/// With a `sample_event`, the program is typed against that event's shape:
/// `.message` is a string because the sample says so, and `.msg` does not
/// exist because the sample says that too. Without one, `.` is of unknown
/// shape, which is the safe assumption and the noisy one — every field access
/// is "might be anything".
///
/// A sample that cannot be read does not stop the check. It is reported in
/// `sample_error` and the program is compiled against an unknown event, since
/// having the wrong diagnostics is worse than having the pessimistic ones, but
/// having none at all is worse still.
#[must_use]
pub fn check(source: &str, sample_event: Option<&str>) -> Check {
    let index = LineIndex::new(source);

    let (external, sample_error) = match sample_event.map(sample::event_from_json) {
        None => (sample::unknown_env(), None),
        Some(Ok(event)) => (sample::external_env(&event), None),
        Some(Err(error)) => (sample::unknown_env(), Some(error)),
    };

    let typed_with_sample = sample_event.is_some() && sample_error.is_none();

    let (mut compiled, mut diagnostics) = match vrl::compiler::compile_with_external(
        source,
        functions(),
        &external,
        CompileConfig::default(),
    ) {
        Ok(result) => (true, convert(&result.warnings, &index)),
        Err(diagnostics) => (false, convert(&diagnostics, &index)),
    };

    if typed_with_sample {
        relax_over_defensive(&mut diagnostics, source, &index);

        // If every reason to reject the program was "your error handling is
        // redundant for this event", the program compiles for the events it
        // was written for, and saying otherwise would contradict the empty
        // list of errors now on screen.
        compiled = compiled || !diagnostics.iter().any(is_error);
    }

    Check {
        compiled,
        vrl_version: VRL_VERSION.to_owned(),
        diagnostics,
        typed_with_sample,
        sample_error,
    }
}

/// Whether a diagnostic stops the program from being accepted.
pub(crate) fn is_error(diagnostic: &Diagnostic) -> bool {
    matches!(diagnostic.severity, Severity::Error | Severity::Bug)
}

/// Softens the "your error handling is unnecessary" diagnostics that only the
/// sample event produced.
///
/// The test is the compiler's own: compile the same program again with the
/// event unknown, and see which of these complaints survive. One that survives
/// is unconditional — `1 ?? 2` is redundant whatever the event looks like —
/// and stays an error. One that does not is an artefact of typing against a
/// single example, and becomes a warning with a note saying so.
fn relax_over_defensive(diagnostics: &mut [Diagnostic], source: &str, index: &LineIndex<'_>) {
    if !diagnostics
        .iter()
        .any(|d| OVER_DEFENSIVE.contains(&d.code) && d.severity == Severity::Error)
    {
        return;
    }

    let baseline = match vrl::compiler::compile_with_external(
        source,
        functions(),
        &sample::unknown_env(),
        CompileConfig::default(),
    ) {
        Ok(result) => convert(&result.warnings, index),
        Err(diagnostics) => convert(&diagnostics, index),
    };

    for diagnostic in diagnostics {
        if !OVER_DEFENSIVE.contains(&diagnostic.code) || diagnostic.severity != Severity::Error {
            continue;
        }

        let unconditional = baseline
            .iter()
            .any(|other| other.code == diagnostic.code && other.range == diagnostic.range);
        if unconditional {
            continue;
        }

        diagnostic.severity = Severity::Warning;
        diagnostic.relaxed_by_sample = true;
        diagnostic.notes.push(
            "this is only unnecessary for the sample event; without it the compiler wants \
             the error handled, so it is a warning here rather than an error"
                .to_owned(),
        );
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

pub(crate) fn convert(diagnostics: &DiagnosticList, index: &LineIndex<'_>) -> Vec<Diagnostic> {
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
        relaxed_by_sample: false,
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

    /// The headline of phase 5: the same program, checked twice, with the only
    /// difference being that the compiler was told what an event looks like.
    #[test]
    fn a_sample_event_turns_might_fail_into_a_fact() {
        let source = ".level = downcase(.message)\n.next = .count + 1\n";

        assert!(
            !errors(source).is_empty(),
            "with an unknown event, .message might not be a string and .count might not be a number",
        );

        let with_sample = check(source, Some(r#"{"message":"HELLO","count":1}"#));
        assert!(
            with_sample.compiled,
            "the sample settles both types: {:?}",
            with_sample.diagnostics,
        );
        assert!(with_sample.typed_with_sample);
    }

    /// What a sample cannot do, which matters just as much: a function that
    /// can fail on perfectly valid input still fails. `parse_json` takes a
    /// string and the sample proves `.message` is one, but the string still
    /// might not be JSON, and only the runtime can know.
    #[test]
    fn a_sample_event_does_not_excuse_a_genuinely_fallible_call() {
        let result = check(
            ".parsed = parse_json(.message)\n",
            Some(r#"{"message":"{\"a\":1}"}"#),
        );

        assert!(!result.compiled);
        assert!(result.diagnostics.iter().any(|d| d.code == 103));
    }

    /// The other half: a sample makes *more* things errors, not fewer. A field
    /// the sample does not have is a typo, and now the compiler can say so.
    #[test]
    fn a_sample_event_makes_a_misspelled_field_visible() {
        let sample = Some(r#"{"message":"hello"}"#);
        let result = check(".n = .mesage + 1\n", sample);

        assert!(!result.compiled, "adding 1 to a field that does not exist");
        assert!(result.typed_with_sample);
    }

    /// The trap a sample sets, and the reason `OVER_DEFENSIVE` exists: a
    /// parser written to survive a missing field is told its error handling is
    /// unnecessary, because today's sample happens to have the field. That is
    /// worth knowing and is not worth a red squiggle.
    #[test]
    fn a_defensive_program_is_not_broken_by_a_sample() {
        let source = ".host = string(.hostname) ?? \"unknown\"\n";
        let result = check(source, Some(r#"{"hostname":"web-01"}"#));

        assert!(result.compiled, "{:?}", result.diagnostics);
        let relaxed: Vec<&Diagnostic> = result
            .diagnostics
            .iter()
            .filter(|d| d.relaxed_by_sample)
            .collect();

        assert_eq!(relaxed.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(relaxed[0].code, 651);
        assert_eq!(relaxed[0].severity, Severity::Warning);
        assert!(
            relaxed[0].notes.iter().any(|note| note.contains("sample event")),
            "the note has to say why this is not an error",
        );
    }

    /// The other side of the same rule: redundant error handling that is
    /// redundant whatever the event looks like is still an error, because the
    /// sample had nothing to do with it.
    #[test]
    fn error_handling_that_is_always_redundant_stays_an_error() {
        let result = check(".x = 1 ?? 2\n", Some(r#"{"message":"hello"}"#));

        assert!(!result.compiled);
        let error = result
            .diagnostics
            .iter()
            .find(|d| d.code == 651)
            .expect("the coalesce is redundant");

        assert_eq!(error.severity, Severity::Error);
        assert!(!error.relaxed_by_sample);
    }

    #[test]
    fn nothing_is_relaxed_without_a_sample() {
        let result = check(".x = 1 ?? 2\n", None);

        assert!(result.diagnostics.iter().all(|d| !d.relaxed_by_sample));
    }

    #[test]
    fn a_broken_sample_is_reported_without_giving_up_on_the_program() {
        let result = check(".x = 1\n", Some("{not json"));

        assert!(result.compiled, "the program is still checked");
        assert!(!result.typed_with_sample);
        assert!(result.sample_error.is_some());
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

        assert!(json.contains(&format!("\"vrlVersion\":\"{VRL_VERSION}\"")), "{json}");
        assert!(json.contains("\"compiled\":true"), "{json}");
    }

    /// The pinned version appears in three places: the Cargo manifest that
    /// decides what is compiled, the constant this crate reports, and the
    /// package.json field the grammar generator reads. They must agree.
    ///
    /// The Vector release is checked the same way. It is the number a user
    /// recognises, so it is the one most worth not getting wrong.
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
        assert!(
            package.contains(&format!("\"vectorRelease\": \"{VECTOR_RELEASE}\"")),
            "package.json does not record Vector {VECTOR_RELEASE}",
        );
    }
}
