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
- **Hover, completion and signature help, generated from the compiler.** Hover
  a function for its signature, return type, whether the call can fail, its
  parameters and its examples. Completion lists every function, marks the
  fallible ones and expands into a call with tabstops — closure included for
  `for_each` and friends — and after a `.` it offers the event paths this file
  already uses. Signature help follows the cursor from one argument to the
  next, named arguments included.
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

The extension tracks the `vrl` crate `0.29.0`, the version Vector 0.52.0
depends on, and shows it in the status bar. A mismatch with the Vector you run
in production is the usual source of disagreement between an editor and a
deployment, so it is worth having on screen.

## What it does not do yet

- The compiler is not told the shape of your events, so `.message` has an
  unknown type and `parse_json(.message)` is reported as fallible even when you
  know it is a string. Handle the error, or use `!` where you are sure.
  Running a program against a sample event, which is what would fix this, is
  the next step.
- Diagnostics, hover and completion apply to `.vrl` files. VRL embedded in a
  Vector config is highlighted but not yet compiled.
- Completion does not add the `!` of a fallible call for you. Asserting turns
  a handled error into an aborted program, and that is your decision to make.

## Licence

MIT. The `vrl` crate is MPL-2.0 and is consumed unmodified.
