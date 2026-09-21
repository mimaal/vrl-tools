# VRL Tools

VS Code support for **VRL** (Vector Remap Language), the transformation
language used by [Vector](https://vector.dev).

## What it does today

- **Diagnostics from the real compiler.** The `vrl` crate is compiled to
  WebAssembly and shipped inside the extension, so what gets underlined in the
  editor is what `vector validate` would reject — not a regex approximation.
  It runs as you type, the compiler's notes and labels come through as related
  information, and documented error codes link to
  [errors.vrl.dev](https://errors.vrl.dev).
- **Vector's own functions, not just the standard library.** A `remap`
  transform can call more than the `vrl` crate defines: the enrichment table
  lookups (`find_enrichment_table_records`, `get_enrichment_table_record`), the
  secrets (`get_secret`, `set_secret`, `remove_secret`) and
  `set_semantic_meaning`. They come from Vector itself, at the release the
  compiler is pinned to. An enrichment lookup is checked the way `vector
  validate` checks it: the table it names has to be declared under
  `enrichment_tables` in one of the workspace's Vector configs.
- **A graph of the pipeline.** Open a Vector config and a graph button appears
  in the editor's title bar. It draws where events go — sources, transforms,
  sinks — with named outputs such as a route's branches or a remap's `dropped`
  on their arrows. Hover a component to light up every path through it; click
  it to go to it in the config. Problems are marked where they are: an input
  naming nothing, a wildcard matching nothing, a component nobody reads, a
  loop. The graph redraws as you edit, and exports as Markdown with a Mermaid
  diagram to commit next to the config.

  ![The pipeline graph of a Vector config, with the path through one transform highlighted](https://raw.githubusercontent.com/mimaal/vrl-tools/main/editors/vscode/media/pipeline-graph.png)

- **Hover, completion and signature help, generated from the compiler.** Hover
  a function for its signature, return type, whether the call can fail, its
  parameters and its examples. Completion lists every function, marks the
  fallible ones and expands into a call with tabstops — closure included for
  `for_each` and friends — and after a `.` it offers the event paths this file
  already uses. Signature help follows the cursor from one argument to the
  next, named arguments included.
- **Run a program on a sample event.** Put `program.vrl.sample.json` next to
  `program.vrl` and `VRL: Run on sample event` compiles the program, runs it on
  that event and opens the result beside the source. The sample also types the
  program: `.message` is a string because the sample says so, so the "this
  might fail" noise goes away and a misspelled field starts being caught.
- **Syntax highlighting** built for the constructs that actually show up in
  production parsers: quoted path segments (`."@timestamp"`), metadata paths
  (`%vector.ingest_timestamp`), regex (`r'…'`), raw strings (`s'…'`),
  timestamps (`t'…'`), the fallible-call `!` told apart from negation, and
  error destructuring (`x, err = parse_json(.message)`).
- **Highlighting inside Vector configs**: a `source:` block scalar in a YAML
  config, or a `source = '''…'''` string in a TOML one, is coloured as VRL.
- **Snippets** for the recurring shapes: `pjson`, `psyslog`, `pkv`, `pgrok`,
  `pregex`, `ptime`, `coerce`, `foreach`, `mapvalues`, `ecs`, `abortif`,
  `ifelse`, `ifmatch`, `iferr`. Each one is compiled by the test suite, so a
  snippet cannot expand into code the compiler rejects.

## Which VRL

The extension tracks the `vrl` crate `0.35.0`, the version Vector 0.58.0
depends on, and shows it in the status bar. A mismatch with the Vector you run
in production is the usual source of disagreement between an editor and a
deployment, so it is worth having on screen.

## What it does not do yet

- With no sample event, the compiler is told nothing about the shape of your
  events, so `parse_json(.message)` is reported as fallible even when you know
  `.message` is a string. A sample fixes the typing; it cannot make a function
  that fails on valid input infallible.
- Diagnostics, hover and completion apply to `.vrl` files. VRL embedded in a
  Vector config is highlighted but not yet compiled.
- One sample event per program, not a set of them.
- `get_vector_metric`, `find_vector_metrics`, `aggregate_vector_metrics` and
  `parse_dnstap` are still reported as undefined: they need parts of Vector
  that do not belong inside an editor extension.
- An enrichment lookup is checked against the table's name, not its contents.
  Which fields a `geoip` or `file` table can be searched by depends on data
  that lives where Vector runs, so that part is left to Vector.
- The graph reads one config file. A pipeline split across a directory, the
  way `vector --config-dir` reads it, is drawn one file at a time.
- Completion does not add the `!` of a fallible call for you. Asserting turns
  a handled error into an aborted program, and that is your decision to make.

## Licence

MIT. The compiler inside it is the `vrl` crate, which is MPL-2.0 and is
consumed unmodified; its source is at <https://github.com/vectordotdev/vrl>
under the pinned tag. The terms of everything compiled into the WebAssembly
module ship with the extension, in `THIRD-PARTY-NOTICES.md`.
