//! Dumps the VRL standard library as JSON, on stdout.
//!
//! `vrl::stdlib::all()` is the authoritative list: it is the same value the
//! compiler is handed when it resolves a call, so a function that exists here
//! exists in the language, with exactly these parameters. Nothing downstream —
//! the TextMate grammar's function regex today, hover and completion in phase 4
//! — may hand-write any of this.
//!
//! Usage: `cargo run -q -p vrl-check-core --bin vrl-stdlib`

use serde::Serialize;
use vrl_check_core::VRL_VERSION;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stdlib {
    vrl_version: &'static str,
    functions: Vec<Function>,
}

#[derive(Serialize)]
struct Function {
    name: &'static str,
    summary: &'static str,
    usage: &'static str,
    parameters: Vec<Parameter>,
    examples: Vec<Example>,
}

#[derive(Serialize)]
struct Parameter {
    keyword: &'static str,
    /// The accepted types, rendered the way the compiler describes them in its
    /// own error messages, e.g. "string" or "integer or float".
    kind: String,
    required: bool,
}

#[derive(Serialize)]
struct Example {
    title: &'static str,
    source: &'static str,
    /// `Ok` for an example that returns a value, `Err` for one that shows the
    /// error a misuse produces.
    result: Result<&'static str, &'static str>,
}

fn main() {
    let functions = vrl::stdlib::all()
        .iter()
        .map(|function| Function {
            name: function.identifier(),
            summary: function.summary(),
            usage: function.usage(),
            parameters: function
                .parameters()
                .iter()
                .map(|parameter| Parameter {
                    keyword: parameter.keyword,
                    kind: parameter.kind().to_string(),
                    required: parameter.required,
                })
                .collect(),
            examples: function
                .examples()
                .iter()
                .map(|example| Example {
                    title: example.title,
                    source: example.source,
                    result: example.result,
                })
                .collect(),
        })
        .collect();

    let stdlib = Stdlib {
        vrl_version: VRL_VERSION,
        functions,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&stdlib).expect("the dump serialises"),
    );
}
