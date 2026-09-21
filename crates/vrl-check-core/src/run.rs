//! Runs a program against a sample event and reports what came out.
//!
//! This is the VRL playground, except local, against your own data and against
//! the exact compiler version your Vector runs. It is also the only part of
//! this crate that executes anything, which is worth stating plainly: checking
//! a program is inert, running one is not. The program is your own, it runs in
//! the same sandbox as the rest of the module, and it cannot reach the network
//! or the filesystem from there — the functions that would, `http_request` and
//! friends, are compiled into the wasm but abort when called.

use serde::{Deserialize, Serialize};
use vrl::compiler::runtime::{Runtime, Terminate};
use vrl::compiler::state::RuntimeState;
use vrl::compiler::{TargetValue, TimeZone};
use vrl::value::{Secrets, Value};

use crate::sample::{event_from_json, external_env};
use crate::{convert, functions, vector, Diagnostic, LineIndex, VRL_VERSION};

/// What happened when the program ran.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// `false` when the program did not compile, in which case nothing ran and
    /// only `diagnostics` is worth reading.
    pub compiled: bool,
    pub vrl_version: String,
    pub diagnostics: Vec<Diagnostic>,
    /// The event after the program, which is what Vector would emit.
    pub event: Option<serde_json::Value>,
    /// The event's metadata, `%vector` and the rest, after the program.
    pub metadata: Option<serde_json::Value>,
    /// The value the program itself returned — the last expression, which for
    /// a `remap` transform is ignored, and for a `filter` condition is the
    /// answer.
    pub output: Option<serde_json::Value>,
    /// The runtime error, if the program stopped.
    pub error: Option<String>,
    /// `true` when the program stopped because it called `abort`, which is a
    /// decision rather than a failure: Vector drops the event.
    pub aborted: bool,
    /// Why the sample event could not be used, when it could not.
    pub sample_error: Option<String>,
}

impl Run {
    fn failed(diagnostics: Vec<Diagnostic>, sample_error: Option<String>) -> Self {
        Self {
            compiled: false,
            vrl_version: VRL_VERSION.to_owned(),
            diagnostics,
            event: None,
            metadata: None,
            output: None,
            error: None,
            aborted: false,
            sample_error,
        }
    }
}

/// The timezone a program runs in.
///
/// UTC, deliberately and always. The machine an editor happens to run on says
/// nothing about the machine Vector runs on, and a `format_timestamp` that
/// renders one way here and another way in production is a bug this tool would
/// be creating rather than catching. Pass a timezone explicitly in the program
/// when it matters.
fn timezone() -> TimeZone {
    TimeZone::parse("UTC").expect("UTC is a valid timezone name")
}

/// Compiles `source` with the shape of `event_json`, runs it against that
/// event, and reports both.
///
/// `enrichment_tables` are what [`crate::check`] takes, for the same reason.
/// The tables have names here but no rows, so a lookup that runs stops with
/// Vector's "table not loaded" error.
#[must_use]
pub fn run(source: &str, event_json: &str, enrichment_tables: &[String]) -> Run {
    let index = LineIndex::new(source);

    let event = match event_from_json(event_json) {
        Ok(event) => event,
        Err(error) => return Run::failed(Vec::new(), Some(error)),
    };

    // The same verdict the editor shows, so "no errors on screen" and "it
    // runs" can never disagree.
    let checked = crate::check(source, Some(event_json), enrichment_tables);
    if checked.diagnostics.iter().any(crate::is_error) {
        return Run::failed(checked.diagnostics, None);
    }

    // Compile for execution, preferring the sample's types. When the only
    // objection to those was redundant error handling — which `check` has
    // already decided is a warning — compile again with the event unknown.
    // Types are a compile-time matter; the program behaves identically either
    // way, and this is what lets a defensive parser still be run.
    let result = match vrl::compiler::compile_with_external(
        source,
        functions(),
        &external_env(&event),
        vector::compile_config(enrichment_tables),
    ) {
        Ok(result) => result,
        Err(_) => match vrl::compiler::compile_with_external(
            source,
            functions(),
            &crate::sample::unknown_env(),
            vector::compile_config(enrichment_tables),
        ) {
            Ok(result) => result,
            Err(diagnostics) => return Run::failed(convert(&diagnostics, &index), None),
        },
    };

    vector::finish_loading(&result.config);

    let mut target = TargetValue {
        value: event,
        metadata: Value::Object(std::collections::BTreeMap::new()),
        secrets: Secrets::default(),
    };

    let mut runtime = Runtime::new(RuntimeState::default());
    let outcome = runtime.resolve(&mut target, &result.program, &timezone());

    let (output, error, aborted) = match outcome {
        Ok(value) => (json(value), None, false),
        // The event is reported either way: seeing how far the program got
        // before it stopped is most of the reason to run it.
        Err(Terminate::Abort(error)) => (None, Some(error.to_string()), true),
        Err(Terminate::Error(error)) => (None, Some(error.to_string()), false),
    };

    Run {
        compiled: true,
        vrl_version: VRL_VERSION.to_owned(),
        diagnostics: convert(&result.warnings, &index),
        event: json(target.value),
        metadata: json(target.metadata),
        output,
        error,
        aborted,
        sample_error: None,
    }
}

/// Runs `source` and returns the answer as JSON.
///
/// # Errors
///
/// Only if the answer fails to serialise, which would be a bug in this crate.
pub fn run_json(
    source: &str,
    event_json: &str,
    enrichment_tables: &[String],
) -> serde_json::Result<String> {
    serde_json::to_string(&run(source, event_json, enrichment_tables))
}

/// A VRL value as JSON.
///
/// Not every VRL value has a JSON shape — a timestamp becomes a string, a
/// regex becomes its pattern — which is the same lossiness Vector's own JSON
/// codec has, so what is shown here is what would be emitted.
fn json(value: Value) -> Option<serde_json::Value> {
    TryInto::<serde_json::Value>::try_into(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(run: &Run) -> &serde_json::Value {
        run.event.as_ref().expect("the program produced an event")
    }

    #[test]
    fn a_program_transforms_the_event() {
        let result = run(".status = to_int!(.status)\n", r#"{"status":"200"}"#, &[]);

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(event(&result)["status"], serde_json::json!(200));
    }

    #[test]
    fn the_sample_gives_the_compiler_real_types() {
        // With no sample this is E103: `.message` might not be a string. With
        // one, the compiler knows it is, and the program compiles untouched.
        let source = ".parsed = parse_json!(.message)\n.len = length(string!(.message))\n";
        let result = run(source, r#"{"message":"{\"a\":1}"}"#, &[]);

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(event(&result)["parsed"], serde_json::json!({"a": 1}));
    }

    #[test]
    fn a_field_the_sample_does_not_have_is_null_not_a_guess() {
        let result = run(".copy = .missing\n", r#"{"message":"hello"}"#, &[]);

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(event(&result)["copy"], serde_json::Value::Null);
    }

    #[test]
    fn the_programs_own_return_value_is_reported() {
        // What a `filter` condition is judged on.
        let result = run(".status == 200\n", r#"{"status":200}"#, &[]);

        assert_eq!(result.output, Some(serde_json::json!(true)));
    }

    #[test]
    fn an_abort_is_reported_as_a_decision_not_a_failure() {
        let result = run("abort\n", r#"{"message":"hello"}"#, &[]);

        assert!(result.compiled);
        assert!(
            result.aborted,
            "abort has to be distinguishable from an error"
        );
        assert!(result.error.is_some());
    }

    #[test]
    fn a_runtime_error_is_reported_with_the_event_as_it_stands() {
        let result = run(
            ".first = \"done\"\n.n = to_int!(.message)\n",
            r#"{"message":"nope"}"#,
            &[],
        );

        assert!(result.compiled);
        assert!(!result.aborted);
        assert!(result.error.is_some(), "to_int! on 'nope' fails at runtime");
        assert_eq!(
            event(&result)["first"],
            serde_json::json!("done"),
            "the work done before the error is worth seeing",
        );
    }

    /// A defensive program does not compile *against the sample's types*, but
    /// the editor calls that a warning, so running it has to work too —
    /// otherwise "no errors on screen" and "it will not run" would disagree.
    #[test]
    fn a_defensive_program_still_runs() {
        let result = run(
            ".host = string(.hostname) ?? \"unknown\"\n",
            r#"{"hostname":"web-01"}"#,
            &[],
        );

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(event(&result)["host"], serde_json::json!("web-01"));
    }

    #[test]
    fn a_program_that_does_not_compile_runs_nothing() {
        let result = run(
            ".parsed = parse_json(.message)\n",
            r#"{"message":"hi"}"#,
            &[],
        );

        assert!(!result.compiled);
        assert!(!result.diagnostics.is_empty());
        assert!(result.event.is_none(), "nothing ran, so there is no event");
    }

    #[test]
    fn a_broken_sample_is_reported_as_such() {
        let result = run(".x = 1\n", "not json at all", &[]);

        assert!(!result.compiled);
        assert!(result.sample_error.is_some());
        assert!(
            result.diagnostics.is_empty(),
            "the program was never the problem"
        );
    }

    /// Metadata is a separate document from the event, and a program can write
    /// to it. Note the shape: a nested write such as `%vector.origin` is
    /// rejected at compile time, because metadata is of unknown shape and
    /// `%vector` might not be an object — the same answer the compiler gives
    /// with no sample at all, since a sample says nothing about metadata.
    #[test]
    fn metadata_survives_the_program() {
        let result = run("%origin = \"editor\"\n", r#"{"message":"hello"}"#, &[]);

        assert!(result.compiled, "{:?}", result.diagnostics);
        assert_eq!(
            result.metadata.expect("metadata")["origin"],
            serde_json::json!("editor"),
        );
    }

    #[test]
    fn timestamps_come_out_the_way_vector_writes_them() {
        let result = run(
            ".at = t'2024-01-01T00:00:00Z'\n",
            r#"{"message":"hello"}"#,
            &[],
        );

        assert_eq!(
            event(&result)["at"],
            serde_json::json!("2024-01-01T00:00:00Z"),
        );
    }

    #[test]
    fn the_answer_serialises_as_camel_case_json() {
        let json = run_json(".x = 1\n", "{}", &[]).expect("serialises");

        assert!(json.contains("\"vrlVersion\""), "{json}");
        assert!(json.contains("\"sampleError\":null"), "{json}");
    }
}
