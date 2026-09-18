# Changelog

Versions before 0.5.0 were never published; they exist as tags and as `.vsix`
files built locally. They are listed here because the history explains what the
extension is.

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
