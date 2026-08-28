# VRL Tools

VS Code support for **VRL** (Vector Remap Language), the transformation
language used by [Vector](https://vector.dev).

## What it does today

- **Syntax highlighting** built for the constructs that actually show up in
  production parsers: quoted path segments (`."@timestamp"`), path coalescence
  (`.foo.(a | b)`), metadata paths (`%vector.ingest_timestamp`), regex (`r'…'`),
  raw strings (`s'…'`), timestamps (`t'…'`), the fallible-call `!` told apart
  from negation, and error destructuring (`x, err = parse_json(.message)`).
- **Highlighting inside Vector configs**: a `source:` block scalar in a YAML
  config, or a `source = '''…'''` string in a TOML one, is coloured as VRL.
- **Snippets** for the recurring shapes: `pjson`, `psyslog`, `pkv`, `pgrok`,
  `pregex`, `ptime`, `coerce`, `foreach`, `mapvalues`, `ecs`, `abortif`,
  `ifelse`, `ifmatch`, `iferr`.

## What it does not do yet

No diagnostics. When they arrive they will come from the real VRL compiler —
the `vrl` crate compiled to WebAssembly — rather than from regexes, so that
what the editor rejects is exactly what `vector validate` rejects.

The extension will track the `vrl` crate `0.29.0`, the version Vector 0.52.0
depends on. A mismatch with the Vector you run in production is the usual source
of disagreement between an editor and a deployment.

## Licence

MIT. The `vrl` crate is MPL-2.0 and is consumed unmodified.
