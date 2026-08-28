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

- VRL highlighting inside Vector configs is a TextMate injection
  (`injectTo: source.yaml` / `source.toml`). VS Code has no supported way to
  disable an injection through a setting, so the `vrl-tools.injectIntoYaml`
  setting sketched in phase 2 of the plan does not exist and should not be
  added as a no-op. The trigger is kept narrow instead: an *indented* `source:`
  block scalar in YAML, a `source` (or dotted `*.source`) multi-line string in
  TOML.

## Working rules

- NEVER hand-write stdlib function lists. Generate them.
  - Target route: a Rust binary over `vrl::stdlib::all()`, which is the
    authoritative source.
  - Current route (phase 1, temporary): Rust is not installed yet, so the
    generator downloads the published `.crate` for the pinned version from
    `static.crates.io` and extracts each function's `identifier()` from
    `src/stdlib/*.rs`. Replace this with the `vrl::stdlib::all()` route as
    soon as Rust is available.
  - The plan mentions `docs/generated` in the `vectordotdev/vrl` repo. That
    directory does not exist at the pinned tag. Don't go looking for it.
- Compiler spans are BYTE offsets. VS Code expects UTF-16 columns. All
  conversion goes through the shared helper and is covered by tests with
  accented characters, CJK, and astral-plane emoji.
- Before using any `vrl` crate API, verify the signature on docs.rs for the
  pinned version. The crate surface still changes.

## Verification

- `cargo test` across the workspace before closing out a phase.
- Snippets are only tokenised today, which proves they are well formed but not
  that they compile. When phase 3 lands, run the same expansions through
  `vrl-check-core` in `scripts/test-snippets.ts`.
- Test the extension against the real parsers in `test-corpus/`, never against
  toy examples. That corpus is generated from the `vrl` crate's stdlib examples
  at test time, plus synthetic parsers built on public log formats.
  Never commit real-world parsers or sample logs from any third party.

## Communication

- Reply to me in Spanish.
