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
/// `sample_event_json` is accepted and ignored for now — see
/// `vrl_check_core::check`.
#[wasm_bindgen]
#[must_use]
pub fn check(source: &str, sample_event_json: Option<String>) -> String {
    vrl_check_core::check_json(source, sample_event_json.as_deref()).unwrap_or_else(|error| {
        // Serialising our own types cannot realistically fail, but returning a
        // parseable answer beats trapping inside the wasm module.
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

/// The pinned `vrl` crate version this module was built against.
#[wasm_bindgen]
#[must_use]
pub fn vrl_version() -> String {
    vrl_check_core::VRL_VERSION.to_owned()
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
        let json = check(".x = 1\n", None);

        assert!(json.contains("\"compiled\":true"), "{json}");
        assert!(json.contains("\"vrlVersion\""), "{json}");
    }

    #[test]
    fn a_rejected_program_still_answers() {
        let json = check(".parsed = parse_json(.message)\n", None);

        assert!(json.contains("\"compiled\":false"), "{json}");
        assert!(json.contains("\"severity\":\"error\""), "{json}");
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
