# vrl-tools

A VS Code extension for **VRL** (Vector Remap Language), the transformation
language used by [Vector](https://vector.dev).

Unlike the existing VRL extensions, diagnostics here come from the **real VRL
compiler** — the `vrl` crate compiled to WebAssembly — not from regexes or
heuristics. If `vector validate` would reject your program, so will this.

## Planned features

| Phase | Feature | Status |
|---|---|---|
| 0 | Extension scaffolding, language registration | done |
| 1 | TextMate grammar + language configuration | done |
| 2 | Snippets, VRL injection into Vector YAML/TOML configs | not started |
| 3 | Real compiler diagnostics via WASM | not started |
| 4 | Hover, completion and signature help, generated from the stdlib | not started |
| 5 | Run a program against a sample event | not started |

## Status

Early development. Nothing is published to the Marketplace yet and there is no
installable `.vsix`. Highlighting works; diagnostics do not exist yet.

Pinned to the `vrl` crate `0.29.0`, which is what Vector 0.52.0 depends on.

## Development

```sh
npm install
npm run build        # generate the grammar, then compile the extension
npm test             # tokenise VRL through vscode-textmate and assert scopes
```

Then press `F5` in VS Code to open an Extension Development Host with
`test-corpus/` loaded.

The grammar is generated. Edit `syntaxes/vrl.tmLanguage.template.json` and run
`npm run gen:grammar`; never edit `vrl.tmLanguage.json` by hand.

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
