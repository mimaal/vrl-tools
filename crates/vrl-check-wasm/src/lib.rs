//! `wasm-bindgen` wrapper around [`vrl_check_core`].
//!
//! This is the whole reason the extension needs no native binary, no child
//! process and no cross-compilation matrix: one `.wasm` inside the `.vsix`
//! works on Windows, macOS, Linux and vscode.dev alike.
//!
//! There is deliberately no logic here. Everything this file does is cross the
//! boundary; anything worth testing lives in the core crate.

use wasm_bindgen::prelude::wasm_bindgen;

/// Makes a panic inside the module arrive in JavaScript with its message and
/// backtrace instead of as a bare `unreachable` trap.
///
/// `wasm-bindgen` calls this once, when the module is instantiated.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
}

/// Compiles `source` and returns the answer as JSON.
///
/// The shape is `vrl_check_core::Check`: `compiled`, `vrlVersion` and a list of
/// diagnostics whose ranges are already in the editor's coordinates (zero-based
/// lines, UTF-16 columns).
///
/// `sample_event_json` types the program; `enrichment_tables` are the table
/// names the Vector config declares, which enrichment lookups are checked
/// against. Leaving it out means none are declared. See
/// `vrl_check_core::check` for both.
#[wasm_bindgen]
#[must_use]
pub fn check(
    source: &str,
    sample_event_json: Option<String>,
    enrichment_tables: Option<Vec<String>>,
) -> String {
    let tables = enrichment_tables.unwrap_or_default();
    vrl_check_core::check_json(source, sample_event_json.as_deref(), &tables).unwrap_or_else(
        |error| {
            // Serialising our own types cannot realistically fail, but returning a
            // parseable answer beats trapping inside the wasm module.
            format!(
            "{{\"compiled\":false,\"vrlVersion\":\"{}\",\"diagnostics\":[],\"internalError\":{}}}",
            vrl_check_core::VRL_VERSION,
            serde_json_string(&error.to_string()),
        )
        },
    )
}

/// Compiles `source` against `event_json` and runs it, returning JSON.
///
/// The shape is `vrl_check_core::Run`: the event after the program, its
/// metadata, the value the program returned, and the runtime error if it
/// stopped.
///
/// This executes the user's own program inside the same sandbox as everything
/// else here. It has no filesystem and no network to reach: the functions that
/// would want them are compiled in but abort when called.
///
/// `enrichment_tables` is what [`check`] takes.
#[wasm_bindgen]
#[must_use]
pub fn run(source: &str, event_json: &str, enrichment_tables: Option<Vec<String>>) -> String {
    let tables = enrichment_tables.unwrap_or_default();
    vrl_check_core::run_json(source, event_json, &tables).unwrap_or_else(|error| {
        format!(
            "{{\"compiled\":false,\"vrlVersion\":\"{}\",\"diagnostics\":[],\"internalError\":{}}}",
            vrl_check_core::VRL_VERSION,
            serde_json_string(&error.to_string()),
        )
    })
}

/// The standard library as JSON, for hover, completion and signature help.
///
/// It comes out of the same module that compiles programs, so what the editor
/// describes and what the editor checks can never be two different versions of
/// the language.
///
/// The shape is `vrl_check_core::Stdlib`. Building it walks every function and
/// compiles a probe call for each, so a client should ask once and keep the
/// answer.
#[wasm_bindgen]
#[must_use]
pub fn stdlib() -> String {
    vrl_check_core::stdlib_json().unwrap_or_else(|error| {
        format!(
            "{{\"vrlVersion\":\"{}\",\"functions\":[],\"internalError\":{}}}",
            vrl_check_core::VRL_VERSION,
            serde_json_string(&error.to_string()),
        )
    })
}

/// The topology of a Vector configuration, as JSON.
///
/// The shape is `vector_topology::Analysis`: the Markdown document to open,
/// the components with their positions, the resolved edges, and the findings.
/// A file that does not parse at all comes back as `{"error": {…}}`, so the
/// caller has one shape to handle rather than two.
///
/// `file_name` chooses the parser and titles the document. A Vector config
/// carries no marker saying whether it is YAML or TOML.
#[wasm_bindgen]
#[must_use]
pub fn topology(source: &str, file_name: &str) -> String {
    vector_topology::analyse_file_json(source, file_name)
}

/// The topology of a pipeline split across several files, as JSON.
///
/// `files_json` is an array of `{"name": …, "source": …}`, each file a
/// complete config the way Vector reads the files given to `--config`. The
/// shape is `vector_topology::Analysis`, whose `files` and `unreadable` say
/// which file each component and finding is in, and which files did not
/// parse. See `vector_topology::analyse_files`.
#[wasm_bindgen]
#[must_use]
pub fn topology_files(files_json: &str, title: &str) -> String {
    vector_topology::analyse_files_json(files_json, title)
}

/// The enrichment table names a Vector config declares, which is what
/// [`check`] and [`run`] take.
///
/// `undefined` when the file is not a config or does not parse, so the caller
/// can keep what it last read. See `vector_topology::enrichment_tables`.
#[wasm_bindgen]
#[must_use]
pub fn enrichment_tables(source: &str, file_name: &str) -> Option<Vec<String>> {
    vector_topology::enrichment_tables(source, file_name)
}

/// The pinned `vrl` crate version this module was built against.
#[wasm_bindgen]
#[must_use]
pub fn vrl_version() -> String {
    vrl_check_core::VRL_VERSION.to_owned()
}

/// The Vector release that ships exactly that `vrl` version.
#[wasm_bindgen]
#[must_use]
pub fn vector_release() -> String {
    vrl_check_core::VECTOR_RELEASE.to_owned()
}

/// Minimal JSON string escaping, so the fallback above cannot itself produce
/// invalid JSON.
fn serde_json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_returns_the_cores_json() {
        let json = check(".x = 1\n", None, None);

        assert!(json.contains("\"compiled\":true"), "{json}");
        assert!(json.contains("\"vrlVersion\""), "{json}");
    }

    #[test]
    fn a_rejected_program_still_answers() {
        let json = check(".parsed = parse_json(.message)\n", None, None);

        assert!(json.contains("\"compiled\":false"), "{json}");
        assert!(json.contains("\"severity\":\"error\""), "{json}");
    }

    #[test]
    fn running_a_program_crosses_the_boundary() {
        let json = run(".status = to_int!(.status)\n", "{\"status\":\"200\"}", None);

        assert!(json.contains("\"compiled\":true"), "{json}");
        assert!(json.contains("\"status\":200"), "{json}");
    }

    #[test]
    fn a_run_against_a_broken_sample_still_answers() {
        let json = run(".x = 1\n", "not json", None);

        assert!(json.contains("\"sampleError\""), "{json}");
    }

    #[test]
    fn the_stdlib_dump_crosses_the_boundary() {
        let json = stdlib();

        assert!(json.contains("\"parse_json\""), "the stdlib is in the dump");
        assert!(json.contains("\"vrlVersion\""), "so is the version");
    }

    #[test]
    fn escaping_covers_the_characters_that_break_json() {
        assert_eq!(serde_json_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
