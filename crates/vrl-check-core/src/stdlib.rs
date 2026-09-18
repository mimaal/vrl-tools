//! The standard library, described by the compiler rather than by hand.
//!
//! Everything an editor wants to say about a function — its parameters, what
//! it returns, whether it can fail, how to call it with a closure — comes from
//! `vrl::stdlib::all()` and from compiling a probe call with the real
//! compiler. Nothing here is a table that has to be kept in step with a
//! version bump.
//!
//! Two things the `Function` trait does not carry are the return type and
//! fallibility: both are properties of a *call*, not of a function, because
//! they depend on the argument types. So this module makes a call. For each
//! function it builds `name(arg, …)`, where every argument is an expression
//! whose static type is exactly what the parameter declares, compiles it, and
//! reads `TypeDef::is_fallible` and `TypeDef::kind` off the result.
//!
//! That answers the question a person actually has ("do I have to handle an
//! error here?") for the ordinary case, and where the probe does not compile —
//! a handful of functions validate their arguments at compile time and reject
//! a placeholder — the answer is `None` rather than a guess.

use serde::{Deserialize, Serialize};
use vrl::compiler::function::closure::{self, VariableKind};
use vrl::value::Kind;

use crate::{functions, VRL_VERSION};

/// The whole standard library, as an editor wants it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stdlib {
    pub vrl_version: String,
    pub functions: Vec<Function>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Function {
    pub name: String,
    /// One line. This is what a completion list shows next to the name.
    pub summary: String,
    /// The long form, several paragraphs of it for some functions.
    pub usage: String,
    pub parameters: Vec<Parameter>,
    /// Present when the function takes a closure, e.g. `for_each`.
    pub closure: Option<Closure>,
    /// What a call returns, as the compiler renders it ("string", "integer or
    /// float", "any"). `None` when the probe did not compile.
    pub returns: Option<String>,
    /// Whether a call with well-typed arguments can fail, so the caller has to
    /// handle the error, coalesce it or assert with `!`. `None` when the probe
    /// did not compile.
    pub fallible: Option<bool>,
    pub examples: Vec<Example>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub keyword: String,
    /// The accepted types, rendered the way the compiler describes them in its
    /// own error messages, e.g. "string" or "integer or float".
    pub kind: String,
    pub required: bool,
}

/// The closure a function takes, e.g. `-> |key, value| { … }`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Closure {
    /// Names for the closure's variables. VRL lets the caller name these, so
    /// these are suggestions derived from what each variable holds.
    pub variables: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub title: String,
    pub source: String,
    /// What the example produces, or the error it demonstrates.
    pub result: Result<String, String>,
}

/// Walks `vrl::stdlib::all()` and describes every function in it.
#[must_use]
pub fn stdlib() -> Stdlib {
    let functions = functions()
        .iter()
        .map(|function| {
            let closure = function.closure().as_ref().map(describe_closure);
            let probe = probe(function.identifier(), function.parameters(), closure.as_ref());

            Function {
                name: function.identifier().to_owned(),
                summary: prose(function.summary()),
                usage: prose(function.usage()),
                parameters: function
                    .parameters()
                    .iter()
                    .map(|parameter| Parameter {
                        keyword: parameter.keyword.to_owned(),
                        kind: parameter.kind().to_string(),
                        required: parameter.required,
                    })
                    .collect(),
                closure,
                returns: probe.as_ref().map(|p| p.returns.clone()),
                fallible: probe.map(|p| p.fallible),
                examples: examples(function.as_ref()),
            }
        })
        .collect();

    Stdlib {
        vrl_version: VRL_VERSION.to_owned(),
        functions,
    }
}

/// Prose, or nothing.
///
/// `Function::summary` and `Function::usage` default to the literal string
/// "TODO", and 190 of the 199 functions never override it: the descriptions on
/// the VRL website live in Vector's cue files, not in this crate. Passing
/// "TODO" along would put it in a hover, so it comes out empty and the
/// signature, types, fallibility and examples carry the hover instead.
fn prose(text: &str) -> String {
    if text.trim() == "TODO" {
        return String::new();
    }
    text.to_owned()
}

/// The examples a function documents, or none when asking would panic.
fn examples(function: &dyn vrl::compiler::Function) -> Vec<Example> {
    if NOT_INTROSPECTABLE.contains(&function.identifier()) {
        return Vec::new();
    }

    function
        .examples()
        .iter()
        .map(|example| Example {
            title: example.title.to_owned(),
            source: example.source.to_owned(),
            result: example
                .result
                .map(std::borrow::ToOwned::to_owned)
                .map_err(std::borrow::ToOwned::to_owned),
        })
        .collect()
}

/// The standard library as JSON, which is the shape every wrapper wants.
///
/// # Errors
///
/// Only if the dump fails to serialise, which would be a bug in this crate.
pub fn stdlib_json() -> serde_json::Result<String> {
    serde_json::to_string(&stdlib())
}

struct Probe {
    fallible: bool,
    returns: String,
}

/// Functions that cannot be asked about, for two separate reasons, both of
/// them landmines in the crate rather than choices here:
///
/// - their examples are built at first use from `CARGO_MANIFEST_DIR` and
///   `unwrap` it (`vrl-0.35.0/src/stdlib/encode_proto.rs:17`). That variable
///   exists while cargo is running and nowhere else, so merely *reading*
///   `examples()` panics inside the shipped wasm module;
/// - compiling a call to them opens a descriptor or schema file named by an
///   argument, and panics when it is not there. A placeholder path is exactly
///   that case.
///
/// This is a list of questions to skip, not a list of functions: they still
/// come from `vrl::stdlib::all()` and appear in the dump, with no examples and
/// with `fallible: None`.
const NOT_INTROSPECTABLE: &[&str] = &["encode_proto", "parse_proto", "validate_json_schema"];

/// Compiles one call to `name` and reports what the compiler makes of it.
fn probe(
    name: &str,
    parameters: &[vrl::compiler::function::Parameter],
    closure: Option<&Closure>,
) -> Option<Probe> {
    if NOT_INTROSPECTABLE.contains(&name) {
        return None;
    }

    let mut arguments = Vec::new();
    for parameter in parameters.iter().filter(|p| p.required) {
        arguments.push(argument(&parameter.kind())?);
    }

    // The call is written with `!` on purpose. A bare fallible expression is a
    // compile error ("unhandled error"), so probing `f(…)` would fail for
    // every fallible function — the exact ones worth asking about. Asserting
    // with `!` compiles either way, and the compiler then answers the question
    // itself: applying `!` to something that cannot fail earns warning 620,
    // "can't abort infallible function".
    let mut source = format!("{name}!({})", arguments.join(", "));
    if let Some(closure) = closure {
        // The body has to be *something*; `null` is accepted wherever the
        // closure's output kind is not constrained, and where it is, the probe
        // fails and the answer is honestly unknown.
        source.push_str(&format!(" -> |{}| {{ null }}", closure.variables.join(", ")));
    }

    let result = vrl::compiler::compile(&source, functions()).ok()?;
    let infallible = result
        .warnings
        .iter()
        .any(|diagnostic| diagnostic.code == CANT_ABORT_INFALLIBLE_FUNCTION);

    Some(Probe {
        fallible: !infallible,
        // `!` strips the error from the type, not the value, so this is the
        // kind an ordinary call returns.
        returns: render_kind(result.program.final_type_info().result.kind()),
    })
}

/// The compiler's own wording for a type, with one exception.
///
/// `Display` expands a collection it knows exactly into its members, so
/// `push` comes out as `[string, integer, float, …]` — the kinds an element
/// may hold, which reads like a return type and is not one. A return type of
/// "array" is what a person wants there.
fn render_kind(kind: &Kind) -> String {
    if kind.is_exact() {
        if kind.contains_object() {
            return "object".to_owned();
        }
        if kind.contains_array() {
            return "array".to_owned();
        }
    }

    kind.to_string()
}

/// Warning 620, "can't abort infallible function": the compiler telling us the
/// `!` in the probe was pointless, which is precisely the answer we want.
const CANT_ABORT_INFALLIBLE_FUNCTION: usize = 620;

/// An expression whose static type is exactly `kind`.
///
/// For a parameter that accepts anything, that is a path: the compiler knows
/// nothing about an event field, which is the situation every real call is in.
/// It is also what makes `string(.foo)` come back fallible, as it should,
/// while `downcase("x")` does not.
fn argument(kind: &Kind) -> Option<String> {
    if kind.is_any() {
        return Some(".probe".to_owned());
    }

    let literal = if kind.contains_bytes() {
        "\"probe\""
    } else if kind.contains_integer() {
        "1"
    } else if kind.contains_float() {
        "1.0"
    } else if kind.contains_boolean() {
        "true"
    } else if kind.contains_timestamp() {
        "t'2020-01-01T00:00:00Z'"
    } else if kind.contains_regex() {
        "r'probe'"
    } else if kind.contains_object() {
        "{}"
    } else if kind.contains_array() {
        "[]"
    } else if kind.contains_null() {
        "null"
    } else {
        return None;
    };

    Some(literal.to_owned())
}

/// Names for a closure's variables, taken from what each one holds.
///
/// VRL lets the caller name these however they like, so this is a suggestion,
/// not a signature: `for_each(…) -> |key, value|`, `map_values(…) -> |value|`.
fn describe_closure(definition: &closure::Definition) -> Closure {
    let variables = definition
        .inputs
        .first()
        .map(|input| {
            input
                .variables
                .iter()
                .map(|variable| match variable.kind {
                    VariableKind::TargetInnerKey => "key",
                    VariableKind::Exact(ref kind) if kind.is_bytes() => "key",
                    _ => "value",
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Two variables that would both be called "value" get numbered, so the
    // suggestion is always a valid parameter list.
    let mut seen = 0;
    let variables = variables
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if variables[..i].contains(name) {
                seen += 1;
                format!("{name}{}", seen + 1)
            } else {
                (*name).to_owned()
            }
        })
        .collect();

    Closure { variables }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(name: &str) -> Function {
        stdlib()
            .functions
            .into_iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("{name} is in the standard library"))
    }

    #[test]
    fn the_dump_covers_the_whole_standard_library() {
        let dump = stdlib();

        assert_eq!(dump.vrl_version, VRL_VERSION);
        assert_eq!(dump.functions.len(), functions().len());
        assert!(
            dump.functions.len() > 150,
            "only {} functions; the dump broke rather than the stdlib shrinking",
            dump.functions.len(),
        );
    }

    #[test]
    fn parameters_come_out_as_the_compiler_describes_them() {
        let parse_json = function("parse_json");
        let value = parse_json
            .parameters
            .iter()
            .find(|p| p.keyword == "value")
            .expect("parse_json takes a value");

        assert_eq!(value.kind, "string");
        assert!(value.required);
    }

    #[test]
    fn parsing_is_fallible_and_case_folding_is_not() {
        // The distinction the `!` is about: parse_json can fail on perfectly
        // well-typed input, downcase cannot.
        assert_eq!(function("parse_json").fallible, Some(true));
        assert_eq!(function("downcase").fallible, Some(false));
    }

    #[test]
    fn a_function_over_an_untyped_argument_is_fallible() {
        // `string(.foo)` is the single most common place a `!` is needed, and
        // the probe has to reproduce it: the parameter accepts anything, so
        // the probe passes a path, whose type the compiler cannot verify.
        assert_eq!(function("string").fallible, Some(true));
    }

    #[test]
    fn return_types_come_from_the_compiler() {
        assert_eq!(function("downcase").returns.as_deref(), Some("string"));
        assert_eq!(function("now").returns.as_deref(), Some("timestamp"));
        assert_eq!(function("merge").returns.as_deref(), Some("object"));
    }

    #[test]
    fn a_collection_return_type_is_named_not_expanded() {
        // `push` returns an array whose element kinds the compiler knows
        // exactly. Printing all of them reads like a return type and is not
        // one.
        assert_eq!(function("push").returns.as_deref(), Some("array"));
    }

    #[test]
    fn closures_are_described_with_usable_variable_names() {
        assert_eq!(
            function("for_each").closure,
            Some(Closure {
                variables: vec!["key".to_owned(), "value".to_owned()],
            }),
        );
        assert_eq!(
            function("map_values").closure,
            Some(Closure {
                variables: vec!["value".to_owned()],
            }),
        );
        assert_eq!(function("downcase").closure, None);
    }

    /// Not a curiosity: reading `encode_proto`'s examples panics off cargo,
    /// which took down the whole dump inside the wasm module. The dump keeps
    /// the function and drops the questions that cannot be asked.
    #[test]
    fn the_crates_todo_placeholder_never_reaches_a_hover() {
        let dump = stdlib();

        assert!(
            dump.functions
                .iter()
                .all(|f| f.summary != "TODO" && f.usage != "TODO"),
            "the trait's default prose leaked into the dump",
        );
        assert_eq!(function("parse_cbor").summary, "parse a string to a CBOR type");
    }

    #[test]
    fn a_function_that_cannot_be_introspected_still_appears() {
        let encode_proto = function("encode_proto");

        assert!(encode_proto.examples.is_empty());
        assert_eq!(encode_proto.fallible, None);
        assert_eq!(encode_proto.returns, None);
        assert!(!encode_proto.parameters.is_empty(), "parameters are still real");
    }

    #[test]
    fn examples_survive_with_their_documented_result() {
        let example = function("parse_json")
            .examples
            .into_iter()
            .next()
            .expect("parse_json documents an example");

        assert!(!example.source.is_empty());
        assert!(example.result.is_ok() || example.result.is_err());
    }

    /// The probe is allowed to fail, but only for a few functions. If a
    /// version bump makes most of them unanswerable, the probe broke.
    #[test]
    fn the_probe_answers_for_the_vast_majority_of_functions() {
        let dump = stdlib();
        let unanswered: Vec<&str> = dump
            .functions
            .iter()
            .filter(|f| f.fallible.is_none())
            .map(|f| f.name.as_str())
            .collect();

        assert!(
            unanswered.len() * 10 < dump.functions.len(),
            "{} of {} functions unanswered: {unanswered:?}",
            unanswered.len(),
            dump.functions.len(),
        );
    }

    #[test]
    fn the_dump_serialises_as_camel_case_json() {
        let json = stdlib_json().expect("serialises");

        assert!(json.contains("\"vrlVersion\""), "the version is in the dump");
        assert!(json.contains("\"parse_json\""), "parse_json is in the dump");
    }
}
