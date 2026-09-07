//! Dumps the VRL standard library as JSON, on stdout.
//!
//! `vrl::stdlib::all()` is the authoritative list: it is the same value the
//! compiler is handed when it resolves a call, so a function that exists here
//! exists in the language, with exactly these parameters. Nothing downstream —
//! the TextMate grammar's function regex, hover and completion — may
//! hand-write any of this.
//!
//! The dump itself lives in `vrl_check_core::stdlib`, so the extension can get
//! the same JSON out of the wasm module at runtime rather than shipping a
//! generated file that could fall out of step with it. This binary exists for
//! the build-time consumer: `scripts/gen-grammar.ts`.
//!
//! Usage: `cargo run -q -p vrl-check-core --bin vrl-stdlib`

fn main() {
    let stdlib = vrl_check_core::stdlib();

    println!(
        "{}",
        serde_json::to_string_pretty(&stdlib).expect("the dump serialises"),
    );
}
