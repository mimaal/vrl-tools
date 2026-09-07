//! `wasm-bindgen` wrapper around [`vrl_check_core`].
//!
//! This is the whole reason the extension needs no native binary, no child
//! process and no cross-compilation matrix: one `.wasm` inside the `.vsix`
//! works on Windows, macOS, Linux and vscode.dev alike.
//!
//! There is deliberately no logic here. Everything this file does is cross the
//! boundary; anything worth testing lives in the core crate.

use wasm_bindgen::prelude::wasm_bindgen;

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
    fn escaping_covers_the_characters_that_break_json() {
        assert_eq!(serde_json_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
