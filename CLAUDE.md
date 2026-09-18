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
- The `vrl` crate version is pinned exactly (`=0.35.0`) and surfaced in the
  status bar. That is the version Vector 0.58.0 depends on, confirmed in
  Vector's `Cargo.lock` at tag `v0.58.0`. The pair lives in one place —
  `VRL_VERSION` and `VECTOR_RELEASE` in `crates/vrl-check-core/src/lib.rs` —
  and `version_matches_the_pin` checks both against the manifests. The
  extension asks the wasm module for them rather than keeping its own copy.
  Vector release number != `vrl` crate version, and not every Vector release
  is a candidate: 0.57.0, like 0.50 and 0.51 before it, consumes `vrl` from
  git `branch = main` and has no pinnable crate version. Confirm the pair in
  Vector's `Cargo.lock` at the tag before moving the pin; never follow
  Vector's release numbers. The crate is on 0.x and has never published a
  0.58.0.
  The map as of Vector 0.58.0: 0.58.0 -> 0.35.0, 0.57.0 -> git, 0.56.0 ->
  0.33.1, 0.55.0 -> 0.32.0, 0.54.0 -> 0.31.0, 0.53.0 -> 0.30.0, 0.52.0 ->
  0.29.0.

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

- Fallibility and return types are NOT static properties of a `vrl` function;
  the `Function` trait has no such fields, because both depend on the argument
  types. `crates/vrl-check-core/src/stdlib.rs` gets them by compiling one probe
  call per function and reading `TypeDef`. The probe writes `f!(...)` on
  purpose: a bare fallible expression does not compile, and warning 620 ("can't
  abort infallible function") is the compiler telling us the function cannot
  fail. Don't replace this with a table.
- Three stdlib functions — `encode_proto`, `parse_proto`, `validate_json_schema`
  — build their examples from `CARGO_MANIFEST_DIR` and `unwrap` it, so merely
  calling `examples()` on them panics anywhere cargo is not running, the
  shipped wasm included. `NOT_INTROSPECTABLE` in `stdlib.rs` keeps the dump
  away from them. This was a real crash, not a hypothetical.
- Completion does not insert the `!` of a fallible call, and should not start:
  asserting turns a handled error into an aborted program, which is the user's
  decision, and the diagnostics point at the line either way. The plan's
  "sufijo `!` automático" is deliberately not implemented.

- A sample event (`program.vrl.sample.json`) types the program, and the shape it
  builds is deliberately OPEN: the fields it has get its types, every other
  field stays `any`. Reading it as a closed shape makes `.event.original = ...`
  an error whenever the sample has no `.event`, which breaks every mapping
  program. Don't "tighten" this.
- A sample also makes the compiler call defensive error handling redundant
  (104, 620, 651). Those three, and only when the same program compiled against
  an unknown event does NOT raise them, are reported as warnings with a note.
  The test is the compiler asked twice, not a guess. See `OVER_DEFENSIVE`.
- `check` and `run` must agree: if no errors are on screen, running has to
  work. That is why `run` falls back to compiling against an unknown event when
  the only objection to the sample's types was redundant error handling.
- Programs run in UTC, never the machine's timezone. The machine an editor runs
  on says nothing about the machine Vector runs on.

## Working rules

- NEVER hand-write stdlib function lists. Generate them.
  - `scripts/gen-grammar.ts` shells out to `cargo run -p vrl-check-core --bin
    vrl-stdlib`, which walks `vrl::stdlib::all()`. That is the authoritative
    source: the same value the compiler resolves calls against. Phase 4 (hover,
    completion) consumes the same dump, which already carries parameters and
    examples.
  - The plan mentions `docs/generated` in the `vectordotdev/vrl` repo. That
    directory does not exist at the pinned tag. Don't go looking for it.
- The third-party licence notices are generated too. The `.vsix` ships a wasm
  module that statically links 243 crates, one of them (`vrl`) MPL-2.0, so
  `editors/vscode/THIRD-PARTY-NOTICES.md` is part of what makes distributing it
  lawful. `npm run gen:notices` walks the graph that actually ships
  (`vrl-check-wasm` for `wasm32-unknown-unknown`) with `cargo about`, and fails
  on any licence not accepted in `about.toml`. It runs inside `npm run build`,
  and CI fails if a build changes it. Never hand-edit it, and never widen
  `accepted` to make a build pass without reading what arrived.
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
- `npm test` runs five suites; `test:snippets` compiles every expansion and
  `test:diagnostics` compiles the corpus, including every block the injection
  grammars paint as VRL inside a Vector config. Anything this repo shows as
  valid VRL has to be accepted by the pinned compiler. `test:analysis` covers
  `editors/vscode/src/analysis.ts`, which is the only hand-written reader of
  VRL in the project — it answers "where is the cursor", never "is this
  valid" — and is therefore the only one the compiler cannot keep honest.
- Test the extension against the real parsers in `test-corpus/`, never against
  toy examples. That corpus is generated from the `vrl` crate's stdlib examples
  at test time, plus synthetic parsers built on public log formats.
  Never commit real-world parsers or sample logs from any third party.

## Communication

- Reply to me in Spanish.
