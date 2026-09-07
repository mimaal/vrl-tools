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
| 3 | Real compiler diagnostics via WASM | done |
| 4 | Hover, completion and signature help, generated from the stdlib | done |
| 5 | Run a program against a sample event | done |

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

## Diagnostics

Open a `.vrl` file and the real compiler runs on every keystroke, debounced by
300 ms. What you see is what `vector validate` would tell you, in the place it
happens:

```vrl
.parsed = parse_json(.message)
#         ^^^^^^^^^^^^^^^^^^^^ E103 unhandled fallible assignment
```

- The compiler's own labels and notes arrive as related information rather than
  being glued onto the message, so the hover stays readable and each label
  keeps a position you can jump to.
- Error codes with a page on <https://errors.vrl.dev> link to it.
- The status bar shows the `vrl` version the diagnostics come from. When that
  disagrees with the Vector running in production, so do the diagnostics, and
  that is the first thing to check.

The checker is the `vrl` crate compiled to WebAssembly, shipped inside the
`.vsix`: no language server, no child process, no per-platform binaries. If the
module fails to load, the extension says so and leaves highlighting and
snippets working rather than falling back to a regex impression of a type
checker.

By default the compiler is told nothing about the shape of `.`, which is why
`parse_json(.message)` is fallible even when you know `.message` is a string.
Put a sample event next to the file and that changes — see below.

## The sample event

Put a JSON file next to a program, named after it with `.sample.json` appended,
and two things happen.

```
parsers/
  access-log.vrl
  access-log.vrl.sample.json     <- one event, as it arrives
```

**The program is typed against it.** `.message` is a string because the sample
says so, so `downcase(.message)` stops being fallible and the noise goes away.
`.count + 1` compiles because `.count` is a number. And a field the sample does
not have is no longer assumed to be anything at all, so the arithmetic on a
misspelled `.mesage` is caught.

**`VRL: Run on sample event`** (also the status bar item, and the editor
context menu) compiles the program, runs it on that event, and opens the result
beside the source: the event as Vector would emit it, with what happened in
comments above — what the program returned, what it did to the metadata,
whether it aborted. It is the vector.dev playground, except local, against your
own data, and on the exact compiler version your Vector runs.

The status bar says which of the two answers you are looking at: `VRL 0.29.0`
for an unknown event, `VRL 0.29.0 · sample` when a sample is in use. Editing
the sample re-checks the program, so the two can be worked on side by side.
Set `vrl-tools.useSampleEventForDiagnostics` to `false` to always check against
an unknown event.

Two things a sample deliberately does **not** do:

- **It does not close the shape of your events.** The fields the sample has get
  the types it gives them; every other field stays unknown, exactly as it is
  with no sample. Reading it as "these fields and no others" would make
  `.event.original = …` an error whenever the sample has no `.event` — and
  mapping an event into a new shape is most of what VRL is for.
- **It does not make your error handling wrong.** With every field present and
  typed, the compiler starts calling defensive code redundant: `string(.hostname)
  ?? "unknown"` earns *unnecessary error coalescing operation*, an error. But
  the coalesce is unnecessary for *that one event*, not for the next one. So
  when one of the three "your error handling is unnecessary" diagnostics (104,
  620, 651) appears only because of a sample — the compiler is asked both ways
  to find out — it is reported as a warning with a note saying why. Error
  handling that is redundant regardless of the event stays an error.

What a sample cannot fix is a function that fails on valid input:
`parse_json(.message)` is fallible however well-typed `.message` is, because
the string still might not be JSON. Only the runtime knows that, which is what
running it on the sample tells you.

## Hover, completion and signature help

The same wasm module that compiles your program also describes the standard
library, so what the editor tells you about a function and what it checks are
never two different versions of VRL.

- **Hover** a function name for its signature, what it returns, whether the
  call can fail, its parameters and its examples.
- **Completion** lists every function with its return type, marks the fallible
  ones, and expands into a call with the required parameters as tabstops —
  including the closure for `for_each`, `map_values` and friends. After a `.`
  it offers the event paths this file already uses, which is the cheapest
  protection there is against writing `.hostname` in one place and
  `.host_name` in another.
- **Signature help** opens with the parenthesis and follows the cursor from one
  argument to the next, including named arguments such as `format:`.

Two details worth knowing:

- **Fallibility comes from the compiler, not from a table.** The `Function`
  trait has no such flag, because whether a call can fail depends on its
  arguments. So the dump compiles one probe call per function, with an
  argument of exactly the declared type for each parameter, and reads the
  answer off the result. `parse_json` comes back fallible, `downcase` does not,
  and `string(.foo)` does, because a path has no known type. Fourteen of the
  199 functions validate their arguments at compile time and reject a
  placeholder; those report nothing rather than a guess.
- **Completion never inserts the `!` for you.** Asserting is a decision —
  it turns a handled error into an aborted program — and the diagnostics
  already point at the line either way.

The crate documents prose for nine functions out of nearly two hundred; the
descriptions on the VRL website live in Vector's cue files, not in the crate.
Where there is no prose the hover leans on the examples, which are the ones the
crate itself tests.

## Snippets

Fourteen snippets covering the patterns that repeat in real parsers: `pjson`,
`psyslog`, `pkv`, `pgrok`, `pregex`, `ptime`, `coerce`, `foreach`, `mapvalues`,
`ecs`, `abortif`, `ifelse`, `ifmatch`, `iferr`. They keep the error rather than
suppressing it with `!`, since that is the habit worth having. Every one of
them is compiled by the test suite, so a snippet cannot ship code the compiler
rejects.

## Status

Early development, v0.4.0, and feature-complete against the plan. Nothing is
published to the Marketplace, but `npm run package` produces an installable
`.vsix`. Highlighting, snippets, config injection, compiler diagnostics, hover,
completion, signature help, sample-typed checking and running a program against
a sample event all work.

Pinned to the `vrl` crate `0.29.0`, which is what Vector 0.52.0 depends on.

## Development

Prerequisites:

- Node 20+
- Rust stable, `wasm-pack`, and the `wasm32-unknown-unknown` target
- **LLVM/clang**, because the VRL standard library includes `encode_zstd` and
  `decode_zstd`, and `zstd-sys` compiles C to reach the wasm target. Without it
  the build stops at `failed to find tool "clang"`. On Windows:
  `winget install LLVM.LLVM`.

```sh
npm install
npm run build        # grammar, then the wasm module, then the extension
npm test             # grammar, injection, snippet and diagnostic checks
npm run package      # build + test + a .vsix in editors/vscode/
cargo test           # the checker itself, plus every stdlib example
```

Install the result locally with:

```sh
code --install-extension editors/vscode/vrl-tools-0.4.0.vsix
```

`npm test` runs five suites. Three go through the same Oniguruma engine VS Code
uses — `test:grammar` (scopes and the documented pitfalls), `test:injection`
(where the embedded VRL region starts and, more importantly, stops) and
`test:snippets` (each body expands to VRL that tokenises cleanly) — and the
fourth, `test:diagnostics`, drives the compiled wasm module: the corpus
programs and every block the injection grammars paint as VRL have to compile
clean, positions have to survive the trip through JSON, and the standard
library dump has to say what the compiler says. `test:snippets` also compiles
each expansion, which is what caught two snippets that produced a fallible
predicate the moment they landed in the buffer. The fifth, `test:analysis`,
covers the cursor-position reading behind hover and completion, against
half-written lines that do not parse.

Then press `F5` in VS Code to open an Extension Development Host with
`test-corpus/` loaded.

The grammar is generated. Edit `syntaxes/vrl.tmLanguage.template.json` and run
`npm run gen:grammar`; never edit `vrl.tmLanguage.json` by hand.

## Design notes

- The checker lives in `crates/vrl-check-core`, which knows nothing about WASM
  or VS Code, so it can be reused behind an LSP server later.
- The grammar highlights the language as the pinned compiler defines it, not as
  older documentation describes it. Path coalescence (`.foo.(a | b)`) was
  removed in `vrl` 0.16.0 and bracketed string keys (`.["a-b"]`, whose real
  form is `."a-b"`) were never valid, so neither is highlighted: painting them
  as valid would contradict the error the compiler puts underneath.
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
