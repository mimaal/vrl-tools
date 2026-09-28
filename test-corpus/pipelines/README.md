# Pipelines corpus

A workspace in miniature, for the code that decides **which files are a
pipeline** — the guess at which files are Vector's, the grouping that keeps
unrelated configs apart, and the merging of one component written across
files. `npm run test:pipelines` walks this directory the way the extension
walks a workspace folder and checks what comes out.

Synthetic, like the rest of `test-corpus/`: no third-party pipelines, no real
logs. The shape is taken from a real one:

- `normalizer/config/` is one pipeline, in miniature: a
  `base.toml` with indented tables and an unquoted environment variable, a
  router it declares, and one file per product adding a route to that router
  (`[[transforms.route_by_product.routes]]`, no `type`) — in both formats,
  since a `--config-dir` mixes them.
- `normalizer/config/monitoring/` is a pipeline of its own beside it, whose
  names clash with the one above.
- `normalizer/deploy/` holds YAML that is not a Vector config: a compose file,
  and Helm values with a `sources:` that is nested, not top-level.
- `examples/` is two standalone configs that both declare `app`.

The pieces only work the way Vector's `--config-dir` loads them. Given with
`--config`, each file is loaded on its own, and the product files fail — which
the test checks too. `vector validate` needs `BUFFER_SIZE_BYTES` set to read
`base.toml`, exactly as Vector does.
