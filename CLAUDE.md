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

- Programs compile against `vector_vrl_functions::all()`, not
  `vrl::stdlib::all()`: the standard library plus the functions Vector adds
  (enrichment lookups, secrets, `set_semantic_meaning`), taken as git
  dependencies on Vector at tag `v<VECTOR_RELEASE>`. The tag moves with the
  pin, and `version_matches_the_pin` checks it. Metrics functions and
  `parse_dnstap` are left out on purpose (they need `vector-core` / the dnstap
  parser). Vector's workspace turns on `vrl`'s `cli`, `test`,
  `test_framework` and `arbitrary` features, which unify into ours: the
  linker drops that code (the wasm grew 63 KB) but the notices list the crates.
  Enrichment table names come from the `enrichment_tables` of every Vector
  config in the workspace (`editors/vscode/src/tables.ts`); a `.vrl` file does
  not say which config runs it. The stub table accepts any index, because
  which fields a table can search depends on data only Vector's machine has.

- The pipeline graph reads a pipeline, not a file: the files Vector is given
  with `--config`, each a complete config, joined (`analyse_files` in
  `crates/vector-topology`). Which files is `vrl-tools.vectorConfig`, the same
  patterns Vector is started with; left empty, every YAML/TOML file in the
  workspace folder with a top-level Vector section. JSON is a Vector config
  format too and is read (by the YAML parser — YAML is a superset of JSON, and
  it keeps the spans), but it is deliberately left out of the *guess*: a repo
  has hundreds of JSON files that are not configs and every one would be read
  to find out. Enrichment tables come from the same files. Duplicate names
  across files are errors, as in Vector's `check_shape`. The `--config-dir`
  subfolder layout (one component per file, named by the file) is not read yet.

- Which outputs a component has is decided by its `type`, in
  `crates/vector-topology/src/outputs.rs`, and nothing else can decide it: an
  `opentelemetry` source is told from a `file` source by its type alone. The
  table is **additive** — an unknown type keeps the plain default output — so a
  component newer than the pin draws as it always did instead of dissolving.
  Every rule in it names the file in Vector it was read from, and
  `tests/against_vector.rs` re-reads those names from the checkout under
  `CARGO_HOME/git/checkouts` that the `vector-vrl-functions` git dependency
  leaves behind, skipping when there is none. It cannot be generated: that
  would mean compiling Vector. Do not add a rule without the line of Vector
  it comes from, and do not let the two parsers decide anything themselves —
  they answer through the `outputs::Fields` trait so they cannot disagree.
  The non-obvious entries: `opentelemetry` and `datadog_agent`
  (`multiple_outputs`) have ports and **no default output**; `reroute_dropped`
  belongs to `remap` alone.

- `enrichment_tables` are components, not just names. Vector compiles a table
  into a sink (`as_sink`), and a `memory` table with `source_config` into a
  source as well, under the separate name `source_key` gives it — plus an
  `expired` output when `export_expired_items` is set. Both halves are
  `Role::Table`; which half a component is shows in whether it takes inputs or
  offers outputs. A table is exempt from the "has no inputs" check exactly as
  in Vector, whose `check_shape` runs before tables join the sinks.

- Vector warns about an **output** with no consumers, not a component
  (`validation::warnings` builds `OutputId`s). A `route` with three routes
  wired to one sink is not an orphan and is still throwing two thirds of its
  events away, which is the case the per-component check could not see.

- The graph's checks are Vector's, each named where it lives, in the module
  doc of `crates/vector-topology/src/graph.rs`. Two that are easy to get
  wrong: a name containing a dot is fatal (`check_names`) — so the comment
  that used to justify the resolution order by "a component may legally be
  named with a dot" was simply false — and `wildcard_matching: relaxed` makes
  a pattern matching nothing legal, so reporting it is a false positive.
  Typechecking edges (log/metric/trace) is Vector's `graph.typecheck()` and is
  deliberately not done here.

- A workspace holds more than one pipeline often enough to matter, and reading
  every config file in it as one config produced nonsense (on this repo's own
  corpus: 18 files, 73 components, 44 "two components are called `app_logs`").
  They are split by **Vector's rule, not folder names**: files whose component
  names collide cannot be one config, because `check_shape` refuses to start on
  that. The whole folder is tried first and split only where names clash — next
  directory down, then file by file — so `--config 'config/**/*.toml'` stays
  whole. The algorithm is `editors/vscode/src/grouping.ts`, deliberately free
  of both `vscode` and the wasm module so `npm run test:grouping` can exercise
  it; like `analysis.ts`, it is a hand-written reader the compiler cannot keep
  honest. Configured `vectorConfig` patterns are never split: they are the
  files Vector is started with, whatever the names do.

- The graph is rebuilt between keystrokes, so the checks are written to scale.
  Cycle detection is **Tarjan's algorithm, iterative**, not "can each component
  reach itself?" — that reading was cubic (a chain of 400 took 212ms, now
  13ms) and recursive, and deep recursion in wasm is a trap that costs the
  whole module. Resolution, duplicate names, unread outputs and the Mermaid
  render all index once instead of scanning per item. Measure before changing
  any of it; the bench that found this is a fan-out and a chain at 25..400
  components.

- The way into the graph is the **activity bar**, not the editor title. The
  title button only exists while a config is the open file, and the open file
  is usually the `.vrl` program whose transform is being written. The view
  (`editors/vscode/src/sidebar.ts`) is a tree, not a second drawing: a sidebar
  is 300px wide and a left-to-right pipeline will not fit in one. It activates
  the extension by `onView`, so the icon costs nothing until it is opened, and
  `showPipelineGraph` finds the pipeline itself rather than requiring a config
  in front of it.

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

- Releases stop at the GitHub release, and the `.vsix` is uploaded to the
  Marketplace by hand. This is not an omission. `vsce publish` needs an Azure
  DevOps PAT scoped to "All accessible organizations" — a global PAT — and
  global PATs are retired on 1 December 2026. The replacement is Entra ID with
  workload identity federation and `vsce publish --azure-credential`, which is
  a service connection, a federated credential and an identity enrolled in the
  publisher. Do not add a `VSCE_PAT` secret and a publish step: it would work
  for a few weeks and then fail, at the worst moment. If publishing becomes
  frequent enough to automate, build the federation.

## Working rules

- **Read Vector's source, not Vector's docs, and never your memory of it.**
  Building anything here leaves a full checkout of Vector at the pinned tag
  under `CARGO_HOME/git/checkouts/vector-*/<commit>/`, because
  `vector-vrl-functions` is a git dependency. It is the same code the pin is
  against, it is on the machine already, and it answers questions the published
  reference does not: which ports `fn outputs` really returns, what
  `check_shape` really rejects, what a flag really defaults to. Every claim
  this repo makes about Vector was settled that way, and the ones about the
  graph were settled that way *after* being wrong when they were not.
  `crates/vector-topology/tests/against_vector.rs` shows how to find the
  checkout; the version in its `Cargo.toml` is what identifies the right one.

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
- `npm test` runs six suites; `test:snippets` compiles every expansion and
  `test:diagnostics` compiles the corpus, including every block the injection
  grammars paint as VRL inside a Vector config. Anything this repo shows as
  valid VRL has to be accepted by the pinned compiler. `test:analysis` covers
  `editors/vscode/src/analysis.ts`, which is the only hand-written reader of
  VRL in the project — it answers "where is the cursor", never "is this
  valid" — and is therefore the only one the compiler cannot keep honest.
  `test:grouping` covers `editors/vscode/src/grouping.ts` for the same reason:
  no amount of compiling VRL says whether two folders are one pipeline.
- **Test the path that finds the input, not only the path that takes it.**
  Every topology test passed its files explicitly, so the code that decides
  *which* files are a pipeline had no test at all — and read an entire
  workspace as one config for three releases without anything noticing. The
  same shape of hole exists wherever this project guesses: which files are
  Vector's, which document a graph belongs to, which config declares a table.
  If a function takes what it works on as an argument, ask who builds that
  argument, and whether anything tests them.

- Test the extension against the real parsers in `test-corpus/`, never against
  toy examples. That corpus is generated from the `vrl` crate's stdlib examples
  at test time, plus synthetic parsers built on public log formats.
  Never commit real-world parsers or sample logs from any third party.

## Skills

`.claude/skills/` holds the two procedures that are rare enough to be
forgotten and costly enough to get wrong:

- **`release`** — bump, changelog, package, tag, push. The tag has to match the
  manifest or the workflow refuses, and `vsce` packs the working directory
  rather than the index.
- **`move-the-pin`** — onto a newer `vrl` crate and Vector release. Read it
  before touching `VRL_VERSION`: the hard part is finding a *pinnable pair*,
  not picking a number.

## Communication

- Reply to me in Spanish.
