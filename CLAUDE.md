# vrl-tools

VS Code extension for VRL (Vector Remap Language) from Vector.dev.
Free and open source. Full phased plan lives in @docs/PLAN.md.

## Settled decisions (don't reopen without discussing)

- Diagnostics use the REAL VRL compiler via the `vrl` crate, compiled to WASM.
  Never heuristics or regex for semantic validation.
- Core lives in `crates/vrl-check-core`, knowing nothing about WASM or VS Code,
  so it can be wrapped in an LSP server later.
- Monorepo: `crates/` for Rust, `editors/vscode` for the extension. This layout
  wins over the flat one sketched in phase 0 of the plan.
- The `vrl` crate version is pinned exactly (`=0.29.0`) and surfaced in the
  status bar. That is the version Vector 0.52.0 depends on, confirmed in
  Vector's `Cargo.lock` at tag `v0.52.0`. Note that Vector 0.50 and 0.51
  consume `vrl` from git `branch = main`, so they have no pinnable crate
  version; do not "upgrade" the pin by following Vector's release numbers.
  Vector release number != `vrl` crate version. The crate is on 0.x and has
  never published a 0.52.0.

- The pinned compiler defines the language the grammar paints. Two constructs
  that older VRL documentation still shows are gone and must not come back:
  path coalescence (`.foo.(a | b)`, removed in `vrl` 0.16.0) and bracketed
  string keys (`.["a-b"]`, never valid — the quoted form is `."a-b"`; brackets
  only take integers). `npm run test:diagnostics` compiles the corpus, so
  reintroducing either shows up as a failure.

- VRL highlighting inside Vector configs is a TextMate injection
  (`injectTo: source.yaml` / `source.toml`). VS Code has no supported way to
  disable an injection through a setting, so the `vrl-tools.injectIntoYaml`
  setting sketched in phase 2 of the plan does not exist and should not be
  added as a no-op. The trigger is kept narrow instead: an *indented* `source:`
  block scalar in YAML, a `source` (or dotted `*.source`) multi-line string in
  TOML.

## Working rules

- NEVER hand-write stdlib function lists. Generate them.
  - `scripts/gen-grammar.ts` shells out to `cargo run -p vrl-check-core --bin
    vrl-stdlib`, which walks `vrl::stdlib::all()`. That is the authoritative
    source: the same value the compiler resolves calls against. Phase 4 (hover,
    completion) consumes the same dump, which already carries parameters and
    examples.
  - The plan mentions `docs/generated` in the `vectordotdev/vrl` repo. That
    directory does not exist at the pinned tag. Don't go looking for it.
- Building the wasm needs LLVM/clang on PATH: the stdlib pulls in `zstd`, whose
  `zstd-sys` compiles C for `wasm32-unknown-unknown`. Dropping the zstd
  functions to avoid it is not an option — the compiler would then reject
  `encode_zstd` as unknown, which is exactly the false positive this project
  exists to avoid. `winget install LLVM.LLVM`.
- `wasm-pack`'s bundled wasm-opt (binaryen 117) validates with the wasm
  proposals off and rejects what rustc now emits. The `--enable-*` flags in
  `crates/vrl-check-wasm/Cargo.toml` are not opt-ins, they are what makes the
  module build at all. Don't remove them.
- Compiler spans are BYTE offsets. VS Code expects UTF-16 columns. All
  conversion goes through the shared helper and is covered by tests with
  accented characters, CJK, and astral-plane emoji.
- Before using any `vrl` crate API, verify the signature on docs.rs for the
  pinned version. The crate surface still changes.

## Verification

- `cargo test` across the workspace before closing out a phase.
- `npm test` runs four suites; `test:snippets` compiles every expansion and
  `test:diagnostics` compiles the corpus, including every block the injection
  grammars paint as VRL inside a Vector config. Anything this repo shows as
  valid VRL has to be accepted by the pinned compiler.
- Test the extension against the real parsers in `test-corpus/`, never against
  toy examples. That corpus is generated from the `vrl` crate's stdlib examples
  at test time, plus synthetic parsers built on public log formats.
  Never commit real-world parsers or sample logs from any third party.

## Communication

- Reply to me in Spanish.
