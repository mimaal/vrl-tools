# Changelog

Versions before 0.5.0 were never published; they exist as tags and as `.vsix`
files built locally. They are listed here because the history explains what the
extension is.

## Unreleased

0.7.2 read the pipeline that exposed it correctly — 16 files given with
`--config-dir`, 26 components events pass through, 54 edges, three warnings,
all three intended — and drew it unreadably: the same config declares 372
`file` enrichment tables in three files, and every one was a box. What follows
was measured on `test-corpus/pipelines/fleet/`, a config directory of that
shape (two routers merged across files, an overlay named to sort first, 372
tables, a monitoring graph beside the main one), which `vector validate
--config-dir` 0.58.0 loads with exactly the three warnings the graph gives.
Each claim about Vector was checked against that binary and against Vector's
source at the pin.

- **Enrichment tables are listed, not drawn.** On the fixture the graph placed
  384 nodes, 372 of them tables joined to nothing; it now places 12, and the
  webview draws it in 24 ms instead of 198 (median of 15 redraws in headless
  Chrome; the analysis itself stays at ~27 ms). A table nothing flows through
  — every kind but a `memory` table with `inputs` or a `source_config` — is
  left out of the picture and collapsed into one "Enrichment tables (372)" row
  in the sidebar, grouped by the file that declares it. Each table says who
  reads it: "read by: X, Y" or "unused in this pipeline", from the literal
  table names in `find_enrichment_table_records` and
  `get_enrichment_table_record` calls, in a `remap`'s `source`, in the program
  its `file` or `files` names, and in any condition. **Show tables**, in the
  graph, draws only the tables the components on screen read, each joined to
  its readers by a dotted line; narrowed to one component, that is its tables.
  Unused is said, never warned about: Vector warns about the outputs of
  sources and transforms and a table has none, and table files are shared
  across pipelines. Where it cannot be certain the sidebar says why — Vector
  accepts a table named through a variable (`name = "hosts"`, then
  `get_enrichment_table_record!(name, …)` validates), which only the compiler
  can follow, and a program file may not be in the workspace.
- **A table shows its data, not its type.** When every table is a `file`
  table, 372 rows each saying `file` say nothing; the row gives the CSV path
  as written and, when the file is found in the workspace, its row count —
  records as Vector's CSV reader counts them, header excluded unless
  `include_headers` is off. Paths are written for Vector's working directory,
  so the file is looked for from the config's directory upwards, and the
  tooltip names the file that was counted.
- **An output is named whole: `component.output`.** An arrow labelled
  `_unmatched` or `dropped` does not say whose, and the fixture has two routers
  and three remaps that each have one. The Markdown export now writes
  `route_by_product.product_a` on the Mermaid arrow and adds an **Edges** list
  in the form the config itself uses — `` `normalize-router._unmatched -->
  unmatched` `` — which can be searched and diffed. The graph keeps the short
  label on the arrow, where the box it leaves is in sight, and says the whole
  name in its tooltip and in a text list of the edges for screen readers, to
  which the drawing was one unlabelled image. In the sidebar every output of
  a component with several is a row under it with where it goes —
  "→ time-diff", or "(unread)" — and a component with one output says it on
  its own row.
- **An unread `dropped` is a badge on the node.** It was only a line in the
  problems list; read by something, it is still an arrow.
- **Components come in the order events meet them.** They came in the order
  they were read — file by file, and within a file sources, transforms, sinks
  — so across sixteen files a router was listed after the remaps it feeds
  whenever its file sorted later. The order is now sources, then transforms
  with each one after everything that feeds it, then sinks, then tables, and
  by name wherever that leaves a choice. The sidebar, the Mermaid export and
  the graph's own nodes — which is the order Tab and a screen reader take —
  all use it, and it no longer changes when a component moves to another file.
  Edges are listed the same way, from the sources to the sinks.
- **Graphs that share no arrow are drawn apart.** The fixture, like the
  pipeline it copies, is two: the events, and Vector's own metrics going to a
  Prometheus exporter. They were laid out as one, the second threaded through
  the first's columns with an arrow three columns long. Each connected part is
  now arranged on its own and drawn as a band, the biggest first, titled with
  the sources its events come from (`input-http`, `vector-metrics`); the
  Mermaid export frames them as subgraphs. Connected means by an arrow: a
  table two parts both read does not make them one.

## 0.7.2 — 2026-09-28

After 0.7.1 the pipeline that exposed it drew right: 16 files, 26 components,
54 edges, three findings, all three real. What was left were the things around
it — a directory that restates a declaration, an input Vector reads from the
environment, a route that could not say where it came from, a directory of
spare parts offered as a pipeline, and a scratch directory git ignores. Each
was checked against `vector validate --config-dir` 0.55.0 and against Vector's
source at the pin.

- **A component declared in two files of one directory is one component.**
  Vector reads the files of a `--config-dir` as one value before it builds a
  single component, so a product's file that restates
  `[transforms.r] type = "exclusive_route"` next to the route it adds is not a
  second `r`: it validates, and the graph said "2 components are called `r`",
  "`r` has no inputs" and "`r` has no output `two`". Declarations now merge
  like any other piece. Where two files set one field to different values,
  Vector keeps the later one silently, and the graph says so, once, at the
  file that wins: "`r` is `exclusive_route` in a.toml and `route` in b.toml;
  Vector keeps b.toml's" — an error for `type`, which fails validation on the
  first field that does not fit, and for two kinds of value Vector refuses to
  merge; a warning for anything else. Across two directories, or between
  files given with `--config`, Vector does not merge, and a shared name is
  still a duplicate. Telling unrelated configs apart stays where it was, in
  the grouping: two complete configs that share names in one directory are
  still two.
- **Routes merge in file-name order**, so `00-overlay.toml`'s route is tried
  first. Vector itself merges in the order the directory lists its files,
  which is not alphabetical on every filesystem; a config whose routes depend
  on it depends on the disk it is on.
- **An input read from an environment variable is a note, not an error.**
  `inputs = [${X}]` said "no component is called `${X}`", about text Vector
  never sees. It is now an `info` finding saying the value is on the machine
  Vector runs on, with no edge drawn, and still underlines `${X}` where it is
  written.
- **A route goes to the file that added it.** Every named output now carries
  where it comes from, so clicking `route_by_product.cloudflare-waf` — on the
  graph's arrow label, or under the router in the sidebar, which now lists its
  outputs — opens `cloudflare_waf.toml`, not the file that declares the
  router. Clicking the router still opens the file with its `type`.
- **A directory of parts is not offered as a pipeline.** `devices-available/`,
  the product files kept ready to be copied into `config/`, split off from the
  pipeline because its names clash with it, and was listed with 258 files and
  777 findings. A group with no sources and no sinks whose names belong to a
  pipeline that has them is now left out; its files still graph on their own
  when opened.
- **With nothing chosen, the pipeline shown is the main one**: the biggest
  that has both sources and sinks, not the first alphabetically, which was as
  likely to be `config-monitoring`.
- **The guess honours `files.exclude`, `search.exclude` and `.gitignore`.**
  `findFiles` with an exclude of its own applies none of them, so a
  `tmp.*` scratch directory ignored by git came up as a pipeline. Files named
  by `vrl-tools.vectorConfig` or `vectorConfigDir` are never filtered.
- **`vrl-tools.vectorConfigDir`, for configs Vector loads with
  `--config-dir`.** Vector merges the files at the top of a config directory
  into one, which is what lets a file add a route to a router another file
  declares; given with `--config`, each file is loaded on its own and such a
  file fails. The graph now reads each the way Vector would: files named by
  `vectorConfig` stand alone, files in a `vectorConfigDir` directory merge.
  Guessed files are read as a directory, as in 0.7.1.
- **A component without a `type` is reported, and the report says why.**
  Vector refuses one, and the graph used to draw it as an ordinary box. The
  finding now tells apart a `type` that is simply missing, a piece whose
  declaration is in another directory (Vector merges only within one), and a
  piece in a file given with `--config`.

## 0.7.1 — 2026-09-28

Four ways the graph misread a real pipeline split across many files — one
router, each product's file adding a route to it, a `base.toml` with indented
tables and an unquoted variable, beside a subdirectory of its own. Of 69
findings, 66 were the extension's; 418 files were read as 413 pipelines.

- **A component written across files is one component.** Vector reads the
  files at the top of a `--config-dir` directory as one value, merging them key
  by key, so `[[transforms.route_by_product.routes]]` in each product's file
  adds a route to the router `base.toml` declares. The graph read every such
  piece as a second `route_by_product` with no type, no inputs and no outputs,
  and every route but the first went missing — and with them every input that
  named one. Pieces in one directory are now merged as Vector merges them
  (maps key by key, lists concatenated). What is a piece is decided by `type`:
  two files that both declare the component are still two components with one
  name, which is what unrelated configs side by side look like.
- **Environment variables are interpolated before parsing**, as Vector does.
  `max_size = ${BUFFER_SIZE_BYTES}`, unquoted, is not TOML until the variable
  is replaced, so the whole file was unreadable and its sources and sinks
  disappeared from the graph. A `${VAR:-default}` takes its default; a variable
  without one keeps its written form (quoted, where it is a bare TOML value),
  since its value is on the machine Vector runs on. Positions still point at
  the file as written.
- **Files directly in a directory are one pipeline candidate, not one each.**
  When a folder had to be split, every file at its top became a pipeline of its
  own, so a 16-file pipeline beside one subdirectory came apart into sixteen.
  They are now tried together, like any subdirectory, and split only if they
  clash among themselves. A piece of a component (no `type`) is no longer
  counted as a clash either.
- **Indented TOML tables are recognised as a Vector config.** The guess only
  looked for `[sources.x]` in column 0; TOML allows whitespace before a header,
  and a file whose only header is `[[transforms.x.routes]]` was not recognised
  at all.

## 0.7.0 — 2026-09-24

The graph's model of a Vector topology was checked, component by component,
against Vector's own source at the release the `vrl` pin comes from. Six things
it got wrong are fixed, and every claim it makes is now re-read from that
source by `cargo test` whenever a checkout of Vector is on the machine.

- **Sources with named outputs.** An `opentelemetry` source has `logs`,
  `metrics` and `traces` and **no default output**, and a `datadog_agent` with
  `multiple_outputs` has `logs`, `metrics`, `traces` and `llmobs` (minus
  whichever its `disable_*` flags turn off). The graph knew of no source with
  outputs, so it drew the one arrow Vector rejects — `inputs: [otel]` — and
  reported the ones it accepts, `inputs: [otel.logs]`, as naming nothing. Both
  are now right, and a disabled output is gone rather than empty.
- **An output nobody reads is a warning of its own.** Vector warns per output
  ("Transform \"split._unmatched\" has no consumers"); the graph only noticed a
  component nothing at all read. A `route` with three routes and one sink
  looked finished while two thirds of its events were built and dropped. Each
  unread output now says so, by name.
- **Enrichment tables are in the pipeline.** A `memory` table takes `inputs`
  like a sink, and with `source_config` it is also a source, under the separate
  name its `source_key` gives — with an `expired` output when
  `export_expired_items` is set. None of it was drawn, so the table's inputs
  were missing, whatever fed it looked orphaned, and a sink reading it back was
  reported as naming nothing.
- **Three checks from Vector's `check_shape` that were missing.** A transform
  or sink with no `inputs` at all; the same input named twice, which was also
  drawn as two arrows on top of each other; and a pipeline with no sources or
  no sinks. An empty file stays silent: that is every config for its first few
  seconds.
- **A component whose name contains a dot** is rejected by Vector before
  anything else, because a dot is how an input picks one output. It is now
  reported, and still drawn.
- **`wildcard_matching: relaxed`** is honoured. It is the config saying a
  pattern may match nothing, so reporting one was a false positive on a config
  Vector runs.
- **JSON configs.** Vector reads `.json` as well as `.yaml` and `.toml`. A
  JSON config now graphs like any other; it is not scanned for when guessing
  which files are Vector's, because a repository's other JSON files are many
  and are not configs.

And the guess about which files are one pipeline, which was wrong in a way
nothing caught because the tests always passed files explicitly:

- **A workspace can hold more than one pipeline.** Every config file with a
  Vector section in it was read as one config — so `config/prod` and
  `config/staging`, or a folder of examples, came out as a single topology
  nobody runs, buried under "two components are called `app_logs`". On this
  repository's own test corpus that was 18 files, 73 components and 58
  problems, 44 of them that error.

  They are now told apart by **Vector's rule, not a guess about folder
  names**: files whose component names collide cannot be one config, because
  `check_shape` refuses to start on exactly that. The whole folder is tried
  first and split only where the names actually clash — by the next directory
  down, then file by file. A pipeline genuinely spread across subdirectories,
  which is what `--config 'config/**/*.toml'` is for, stays whole. The same 18
  files now come out as 18 pipelines, each saying only what its own file says.
- **Choosing between them.** When a workspace has more than one, the sidebar's
  first row says which you are looking at and opens a list of the others.
  The graph follows the same choice, and it is remembered per workspace.

And it keeps up with a large pipeline:

- **Loops are found with Tarjan's algorithm**, once over the whole graph,
  instead of asking each component in turn whether it can reach itself. That
  question is the same answer for cubic work: a chain of 400 transforms took
  212ms, which is longer than the gap between two keystrokes, and it now takes
  13ms. It is iterative too, so a long pipeline can no longer overflow the
  stack — in wasm that is a trap the whole module has to be rebuilt from. A
  chain of 6,000 components is now read without complaint.
- Resolving inputs, finding duplicate names, spotting unread outputs, drawing
  the Mermaid diagram and narrowing to one component all looked things up by
  walking a list. They use an index built once per read.
- The graph and the sidebar each re-read every config in the workspace when
  one of them redraws. What each file declares is now remembered against its
  own text, so typing in one config re-parses that one rather than all of
  them.

And a way in that does not depend on the file in front:

- **A Vector view in the activity bar.** The graph button only ever appeared
  while a Vector config was the open file, which is the wrong moment — the file
  in front is usually the `.vrl` program whose transform you are writing. The
  new view lists the pipeline's sources, transforms, sinks and enrichment
  tables, with each component's outputs and its own problems under it, and
  every row opens the graph narrowed to that component and goes to where it is
  declared. **Show pipeline graph** and **Export Markdown** sit in its title
  bar, and both commands now find the pipeline themselves rather than needing a
  config open.

## 0.6.3 — 2026-09-21

- **Large pipelines are navigable.** The wheel scrolls the graph and
  Ctrl+wheel (or a pinch) zooms, as elsewhere in the editor, instead of every
  wheel turn zooming. `Ctrl+F` finds a component by name, type or file and
  `Enter` steps through the matches. Clicking a component keeps its paths lit
  and shows its type, where it is declared and how much is upstream and
  downstream of it; **Show only its paths** redraws just that part, laid out
  on its own, and **Show all** or `Esc` goes back. Double-click opens the
  component in the config. Zoomed out, boxes show only their names, in a
  size that can be read, and output labels only on the lit path. A minimap
  appears when the graph does not fit. `+`, `-`, `0` and the arrow keys work
  too.
- Arrows that skip columns take the room of lines, not of boxes: a bundle of
  them no longer doubles the height of the graph.
- **Input patterns are resolved the way Vector resolves them.** Vector
  matches every input as a glob against the outputs of every component,
  written `id` or `id.port`, so `*_route.errors` takes the `errors` output of
  every router ending in `_route`, and `app*` takes `app.dropped` along with
  `app`. The graph matched patterns against component names only, so the
  first was reported as matching nothing. `?` and `[...]` classes work too.
- A `route` or `exclusive_route` read by its bare name is now an error, as it
  is in Vector: neither has a default output. The message lists the outputs
  it does have.
- `exclusive_route` is understood: its `routes` list names its outputs, and it
  always has `_unmatched`. Before, reading one of its outputs was reported as
  an error.

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
