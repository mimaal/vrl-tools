# vrl-tools

A VS Code extension for **VRL** (Vector Remap Language), the transformation
language used by [Vector](https://vector.dev).

Unlike the existing VRL extensions, diagnostics here come from the **real VRL
compiler** — the `vrl` crate compiled to WebAssembly — not from regexes or
heuristics. If `vector validate` would reject your program, so will this.

## Planned features

| Phase | Feature | Status |
|---|---|---|
| 0 | Extension scaffolding, language registration | in progress |
| 1 | TextMate grammar + language configuration | in progress |
| 2 | Snippets, VRL injection into Vector YAML/TOML configs | not started |
| 3 | Real compiler diagnostics via WASM | not started |
| 4 | Hover, completion and signature help, generated from the stdlib | not started |
| 5 | Run a program against a sample event | not started |

## Status

Early development. Nothing is published to the Marketplace yet and there is no
installable `.vsix`. Phases 0 and 1 are being built now; the extension is not
usable for real work until phase 2.

## Design notes

- The checker lives in `crates/vrl-check-core`, which knows nothing about WASM
  or VS Code, so it can be reused behind an LSP server later.
- Stdlib function lists are **generated** from the `vrl` crate at build time,
  never hand-written.
- The `vrl` crate version is pinned exactly and shown in the status bar, since
  a mismatch with your production Vector produces false positives.

The full phased plan (in Spanish) lives in [`docs/PLAN.md`](docs/PLAN.md).

## Licence

MIT — see [LICENSE](LICENSE).

The `vrl` crate itself is MPL-2.0. It is consumed as an unmodified dependency,
so its terms apply to the crate's own files only. Third-party licence notices
are bundled in the `.vsix`.
