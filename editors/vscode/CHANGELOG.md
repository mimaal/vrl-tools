# Changelog

Versions before 0.5.0 were never published; they exist as tags and as `.vsix`
files built locally. They are listed here because the history explains what the
extension is.

## 0.6.2 — 2026-09-21

- **A pipeline split across several files is drawn whole.** Vector started
  with `--config 'config/**/*.toml'` reads every file as a complete config and
  joins them, so a transform in one file reads a source in another. The graph
  read only the open file, and marked every such input as naming nothing. It
  now reads the pipeline the open file belongs to: the files
  `vrl-tools.vectorConfig` names, the same patterns Vector is started with, or
  when that is not set, every YAML or TOML file in the workspace that declares
  a Vector section.
- Clicking a component opens the file it is declared in, and each component
  and problem says which file that is.
- A name declared in two files is an error in both, as it is in Vector
  ("More than one component with name ...").
- Editing any file of the pipeline redraws the graph. A file that does not
  parse while it is being typed leaves the last complete graph on screen.
- Enrichment lookups are checked against the tables of the same files, so
  setting `vrl-tools.vectorConfig` also decides which tables exist.
- Paths shown in the graph are relative to the workspace folder. On Windows
  they fell back to the bare file name, because the drive letter's case
  differs between the APIs that hand out file URIs.

## 0.6.1 — 2026-09-21

- **The pipeline graph works on a `vector.toml` without a TOML extension.**
  VS Code has YAML built in but not TOML, so with no TOML extension installed
  a `.toml` file opens as plain text, and 0.6.0 recognised a config by its
  language: no graph button, and the command refused the file. A config is now
  recognised by its file name, `.yaml`, `.yml` or `.toml`, and the extension
  activates for a plain-text file so the button can appear.

## 0.6.0 — 2026-09-21

- **A graph of the pipeline, from the Vector config.** A config open in the
  editor gets a graph button in the title bar (only a config: a Kubernetes
  manifest does not). The panel draws sources, transforms and sinks in the
  theme's colours, with named outputs — a route's branches, `_unmatched`, a
  remap's `dropped` — on their arrows. Hovering a component lights up every
  path events take through it; clicking one goes to it in the config. It
  redraws as the config is edited and keeps the last good graph while the file
  does not parse.
- The graph is read, not guessed: every transform and sink declares its
  `inputs`, and the work is resolving them, wildcards and dotted outputs
  included. Nothing needs the `vector` binary.
- **Topology problems are found while the config is written**, which `vector
  graph` cannot do because it only runs on a config Vector accepted: an input
  naming no component, a wildcard matching nothing, a named output the
  component does not have, a component nobody reads, a loop. Each is marked on
  its component and listed under the graph, one click from its line.
- "Export Markdown" opens the same graph as a Mermaid diagram in a Markdown
  document, to commit next to the config; GitHub renders it too.
- The layout is computed in the wasm module: columns follow the flow, sinks
  line up on the right, and an arrow that skips columns runs in a lane between
  the boxes instead of through them.
- **Vector's own VRL functions are no longer "undefined".** A `remap`
  transform compiles against the standard library plus what Vector adds to it,
  and the checker only knew the first half, so every
  `find_enrichment_table_records` and `get_enrichment_table_record` was a false
  error, and so were `get_secret`, `set_secret`, `remove_secret` and
  `set_semantic_meaning`. They are now taken from Vector itself, at the tag the
  `vrl` pin comes from, and appear in hover and completion like the rest.
- **Enrichment lookups are checked against the config's tables**, as
  `vector validate` checks them: the table a lookup names has to be declared
  under `enrichment_tables` in one of the workspace's Vector configs. Adding a
  table to a config, saved or not, re-checks every open program. What the
  checker cannot see is the table's data, so whether a `geoip` table accepts
  the fields a condition searches is still Vector's to say.
- Left out on purpose: `get_vector_metric`, `find_vector_metrics`,
  `aggregate_vector_metrics` and `parse_dnstap`. They need Vector's runtime and
  the dnstap parser, which do not belong in the module the extension ships, so
  they are still reported as undefined.

## 0.5.0 — 2026-09-18

First release on the Marketplace, and the first to track a current Vector.

- **The pinned compiler moves from `vrl` 0.29.0 to 0.35.0**, which is what
  Vector 0.58.0 depends on. 0.29.0 was Vector 0.52.0, nine months and six
  minor versions back. The standard library goes from 199 functions to 203,
  and the grammar, the hovers and the completion list are regenerated from the
  new compiler as always.
- The Vector release the pin corresponds to is now a constant next to the
  crate version, exported by the wasm module and read by the status bar, so
  the tooltip cannot name a Vector the checker was not built against.

The rest of this release is what was missing around the extension rather than
in it.

A security review before the release turned up four things, all of them
availability rather than exposure. The sandbox itself holds: a program in a
`.vrl` file cannot read environment variables, reach the network, resolve a
name or open a file, because none of those reach out of WebAssembly.

- **A single trap no longer ends the session.** Around 800 nested brackets
  overflow the stack inside the parser, and a wasm trap is final: every later
  call into that instance throws, including the one that reports the version.
  One module is loaded per session, so this quietly ended diagnostics, hover
  and completion until the window was reloaded. The module is now replaced and
  the call retried once.
- A sample event is no longer read at all beyond 4 MB. It is read whole,
  synchronously, on the keystroke path; a production capture saved next to a
  program by accident used to stall the editor with nothing on screen to say
  why. It now reports itself as unusable, like any other sample the compiler
  cannot take.
- Offering to create a sample no longer overwrites a file that exists but
  could not be read.
- The run output document builds its URI instead of parsing an interpolated
  one, so a program whose name contains `#` or `?` no longer collides with
  another file's results.

- Third-party licence notices now ship inside the `.vsix`, as
  `THIRD-PARTY-NOTICES.md`. The checker is the `vrl` crate compiled to
  WebAssembly and `vrl` is MPL-2.0, so the terms and the pointer to the source
  travel with the binary.
- An icon, so the Marketplace listing is not a grey square.
- A release is built by GitHub Actions from a tag, not by hand on one laptop
  that happens to have LLVM installed.

## 0.4.0 — 2026-09-07

- **Run a program on a sample event.** `program.vrl.sample.json` next to
  `program.vrl`, and `VRL: Run on sample event` compiles the program, runs it
  and opens the result beside the source — the event as Vector would emit it,
  with what happened in comments above.
- **The sample also types the program.** `.message` is a string because the
  sample says so, so the "this might fail" noise goes away and a misspelled
  field starts being caught. The shape stays open: fields the sample does not
  have remain unknown rather than becoming errors.
- Error handling the sample makes redundant (104, 620, 651) is reported as a
  warning rather than an error, and only when the same program checked against
  an unknown event does not raise it. The compiler is asked both ways.
- The status bar says which answer you are reading, `VRL 0.29.0` or
  `VRL 0.29.0 · sample`, and `vrl-tools.useSampleEventForDiagnostics` turns the
  typing off.
- Programs run in UTC. The machine an editor runs on says nothing about the
  machine Vector runs on.

## 0.3.0 — 2026-09-07

- **Hover, completion and signature help**, generated from the same wasm module
  that compiles your program, so what the editor says and what it checks are
  never two different versions of VRL.
- Fallibility and return types come from compiling a probe call per function,
  not from a table — neither is a static property of a `vrl` function, because
  both depend on the argument types.
- After a `.`, completion offers the event paths the file already uses.
- Completion does not insert the `!` of a fallible call. Asserting turns a
  handled error into an aborted program, and that is the user's decision.

## 0.2.0 — 2026-09-07

- **Diagnostics from the real VRL compiler**, the `vrl` crate compiled to
  WebAssembly and shipped in the `.vsix`. No language server, no child process,
  no per-platform binaries.
- The compiler's labels and notes arrive as related information; documented
  error codes link to errors.vrl.dev.
- The status bar shows the `vrl` version the diagnostics come from, because a
  mismatch with the Vector in production is the first thing to check.
- If the module fails to load the extension says so and leaves highlighting and
  snippets working, rather than falling back to a regex impression of a type
  checker.

## 0.1.0 — 2026-08-28

- Fourteen snippets for the shapes that repeat in real parsers.
- VRL highlighting inside Vector configs: an indented `source:` block scalar in
  YAML, a `source` multi-line string in TOML.

## 0.0.1 — 2026-08-27

- The `vrl` language, a TextMate grammar generated from the pinned compiler's
  standard library, and the language configuration.
