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
| 2 | Snippets, VRL injection into Vector YAML/TOML configs | done |
| 3 | Real compiler diagnostics via WASM | not started |
| 4 | Hover, completion and signature help, generated from the stdlib | not started |
| 5 | Run a program against a sample event | not started |

## VRL inside Vector configs

Most VRL is not written in a `.vrl` file, it is written inside a Vector config,
so the extension injects VRL highlighting there too:

```yaml
transforms:
  parse:
    type: remap
    source: |-          # highlighted as VRL from here…
      parsed, err = parse_json(.message)
      . = merge(., object!(parsed))
sinks:                  # …to here, where the indentation drops
```

```toml
[transforms.parse]
type = "remap"
source = '''
  parsed, err = parse_json(.message)
'''
```

In YAML it fires on an **indented** `source:` key introducing a block scalar
(`|`, `|-`, `>`, `|2-`), which covers remap transforms and `type: vrl`
conditions. In TOML it fires on a `source` key — including dotted forms such as
`condition.source` — holding a `'''` or `"""` multi-line string.

Two honest caveats:

- TextMate has no semantic context, so this cannot check that the block really
  sits under `type: remap`. Any indented `source:` block scalar in any YAML file
  gets VRL colours. Requiring indentation keeps a top-level `source:` out, which
  is what most unrelated YAML uses.
- VS Code has no supported way to turn a grammar injection off with a setting;
  injections are static manifest contributions. So the plan's
  `vrl-tools.injectIntoYaml` setting does not exist — there is nothing it could
  do at runtime. The trigger is kept narrow instead.

## Snippets

Fourteen snippets covering the patterns that repeat in real parsers: `pjson`,
`psyslog`, `pkv`, `pgrok`, `pregex`, `ptime`, `coerce`, `foreach`, `mapvalues`,
`ecs`, `abortif`, `ifelse`, `ifmatch`, `iferr`. They keep the error rather than
suppressing it with `!`, since that is the habit worth having.

## Status

Early development, v0.1.0. Nothing is published to the Marketplace, but
`npm run package` produces an installable `.vsix`. Highlighting, snippets and
config injection work; diagnostics do not exist yet.

Pinned to the `vrl` crate `0.29.0`, which is what Vector 0.52.0 depends on.

## Development

```sh
npm install
npm run build        # generate the grammar, then compile the extension
npm test             # grammar, injection and snippet checks
npm run package      # build + test + a .vsix in editors/vscode/
```

Install the result locally with:

```sh
code --install-extension editors/vscode/vrl-tools-0.1.0.vsix
```

`npm test` runs three suites, all through the same Oniguruma engine VS Code
uses: `test:grammar` (scopes and the documented pitfalls), `test:injection`
(where the embedded VRL region starts and, more importantly, stops) and
`test:snippets` (each body expands to VRL that tokenises cleanly).

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
