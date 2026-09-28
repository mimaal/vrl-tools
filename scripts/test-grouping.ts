/**
 * Exercises editors/vscode/src/grouping.ts, which tells a workspace's config
 * files apart into pipelines.
 *
 * Like `analysis.ts`, this is a hand-written reader the compiler cannot keep
 * honest: no amount of compiling VRL says whether `config/prod` and
 * `config/staging` are one pipeline or two. The rule it applies is Vector's —
 * files whose component names collide cannot be one config, because
 * `check_shape` refuses to start on exactly that — and these cases are the
 * shapes a real workspace comes in.
 *
 * Run with: npm run test:grouping
 */

import {
  commonDirectory,
  declaredNames,
  declaresVectorSection,
  group,
} from '../editors/vscode/src/grouping.js';

let failed = 0;

function check(what: string, actual: unknown, expected: unknown): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a === b) {
    console.log(`ok    ${what}`);
    return;
  }
  console.error(`FAIL  ${what}\n      expected: ${b}\n      actual:   ${a}`);
  failed++;
}

/** A file, written as `path: componentName componentName ...`. */
function file(spec: string): { name: string; names: string[] } {
  const [name, rest] = spec.split(':');
  return { name: name.trim(), names: (rest ?? '').trim().split(/\s+/).filter(Boolean) };
}

function grouped(specs: string[]): { title: string; files: string[] }[] {
  return group(specs.map(file), 'workspace', (f) => f.names).map((g) => ({
    title: g.title,
    files: g.files.map((f) => f.name),
  }));
}

// ---------------------------------------------------------------- one pipeline

check(
  'files that do not clash are one pipeline, whatever their folders',
  grouped([
    'config/sources.toml: app_logs',
    'config/nginx/parse.toml: parse',
    'config/sinks.toml: out',
  ]),
  [
    {
      title: 'config',
      files: ['config/sources.toml', 'config/nginx/parse.toml', 'config/sinks.toml'],
    },
  ],
);

check(
  // Named for the file, not the directory: a directory that comes apart
  // yields several groups, and three rows all saying `corpus` would be no
  // choice at all.
  'a pipeline of one file is named after the file',
  grouped(['config/vector.yaml: app out']),
  [{ title: 'config/vector.yaml', files: ['config/vector.yaml'] }],
);

check(
  'files at the workspace root fall back to the folder name',
  grouped(['vector.yaml: app', 'extra.toml: parse out']),
  [{ title: 'workspace', files: ['vector.yaml', 'extra.toml'] }],
);

// -------------------------------------------------------------- two pipelines

check(
  'prod and staging clash, so they are split by their directories',
  grouped([
    'config/prod/all.yaml: app parse out',
    'config/staging/all.yaml: app parse out',
  ]),
  // Each half is one file, so each is named after that file — which is the
  // more useful of the two here anyway, since `config/prod` and the file in
  // it are the same thing.
  [
    { title: 'config/prod/all.yaml', files: ['config/prod/all.yaml'] },
    { title: 'config/staging/all.yaml', files: ['config/staging/all.yaml'] },
  ],
);

check(
  'a split goes only as deep as it has to: each half stays whole',
  grouped([
    'config/prod/sources.yaml: app',
    'config/prod/sinks.yaml: out',
    'config/staging/sources.yaml: app',
    'config/staging/sinks.yaml: out',
  ]),
  [
    { title: 'config/prod', files: ['config/prod/sources.yaml', 'config/prod/sinks.yaml'] },
    {
      title: 'config/staging',
      files: ['config/staging/sources.yaml', 'config/staging/sinks.yaml'],
    },
  ],
);

check(
  'clashing files in one directory come apart file by file',
  grouped(['corpus/a.yaml: app out', 'corpus/b.yaml: app out', 'corpus/c.yaml: app out']),
  [
    { title: 'corpus/a.yaml', files: ['corpus/a.yaml'] },
    { title: 'corpus/b.yaml', files: ['corpus/b.yaml'] },
    { title: 'corpus/c.yaml', files: ['corpus/c.yaml'] },
  ],
);

check(
  'one real pipeline beside a folder of clashing examples',
  grouped([
    'config/sources.yaml: app',
    'config/sinks.yaml: out',
    'examples/one.yaml: app out',
    'examples/two.yaml: app out',
  ]),
  [
    { title: 'config', files: ['config/sources.yaml', 'config/sinks.yaml'] },
    { title: 'examples/one.yaml', files: ['examples/one.yaml'] },
    { title: 'examples/two.yaml', files: ['examples/two.yaml'] },
  ],
);

check(
  // The shape that came apart into one pipeline per file: a directory of
  // configs, with a subdirectory of its own that clashes with them. The files
  // directly in `config` are one bucket, like any subdirectory.
  'files directly in a directory stay together beside a clashing subdirectory',
  grouped([
    'config/base.toml: input set_defaults out',
    'config/product_a.toml: normalize_a',
    'config/product_b.toml: normalize_b',
    'config/monitoring/vector.toml: input out',
  ]),
  [
    {
      title: 'config',
      files: ['config/base.toml', 'config/product_a.toml', 'config/product_b.toml'],
    },
    { title: 'config/monitoring/vector.toml', files: ['config/monitoring/vector.toml'] },
  ],
);

check(
  'loose files that clash among themselves still come apart',
  grouped(['config/a.yaml: app', 'config/b.yaml: app', 'config/sub/c.yaml: app']),
  [
    { title: 'config/a.yaml', files: ['config/a.yaml'] },
    { title: 'config/b.yaml', files: ['config/b.yaml'] },
    { title: 'config/sub/c.yaml', files: ['config/sub/c.yaml'] },
  ],
);

// -------------------------------------------------------- what a file declares

check(
  // `[[transforms.route_by_product.routes]]` in each product's file, with the
  // router declared once: the pieces have no type, and are not a clash.
  'a piece of a component declared elsewhere is not a name the file declares',
  declaredNames([
    { id: 'route_by_product', type: '' },
    { id: 'normalize_a', type: 'remap' },
  ]),
  ['normalize_a'],
);

check(
  'pieces added from every file of a directory keep it one pipeline',
  group(
    [
      { name: 'config/base.toml', components: [{ id: 'split', type: 'exclusive_route' }] },
      { name: 'config/a.toml', components: [{ id: 'split', type: '' }, { id: 'a', type: 'remap' }] },
      { name: 'config/b.toml', components: [{ id: 'split', type: '' }, { id: 'b', type: 'remap' }] },
    ],
    'workspace',
    (f) => declaredNames(f.components),
  ).map((g) => g.title),
  ['config'],
);

// ------------------------------------------------------ which files are Vector's

for (const [what, text, expected] of [
  ['a YAML section', 'sources:\n  app:\n    type: file\n', true],
  ['a TOML table', '[sources.app]\ntype = "file"\n', true],
  ['a TOML section table', '[sinks]\n', true],
  ['an indented TOML header', '# base\n  [sources.input-http]\n  type = "http_server"\n', true],
  ['a TOML array of tables alone', '[[transforms.route_by_product.routes]]\nname = "a"\n', true],
  ['an indented JSON key', '{\n  "sinks": {}\n}\n', true],
  ['an enrichment table', 'enrichment_tables:\n  geo: {}\n', true],
  ['a nested YAML key', 'customConfig:\n  sources:\n    app: {}\n', false],
  ['an unrelated TOML file', '[package]\nname = "x"\n[dependencies]\n', false],
  ['a word that only starts like a section', '[sourcesx]\nsinks_count = 1\n', false],
] as const) {
  check(`${expected ? 'is' : 'is not'} a Vector config: ${what}`, declaresVectorSection(text), expected);
}

// ------------------------------------------------------------------ the edges

check(
  'a name repeated inside one file does not split anything',
  // A source and a sink both called `app` is a mistake in that file, which
  // the graph reports; it says nothing about which pipeline the file is in.
  grouped(['config/a.yaml: app app', 'config/b.yaml: out']),
  [{ title: 'config', files: ['config/a.yaml', 'config/b.yaml'] }],
);

check(
  'a file that declares nothing joins whatever it is next to',
  // What a config mid-edit looks like: it parses, and has no components yet.
  grouped(['config/a.yaml: app out', 'config/half-typed.yaml:']),
  [{ title: 'config', files: ['config/a.yaml', 'config/half-typed.yaml'] }],
);

check('no files, no pipelines', grouped([]), []);

check(
  'a clash between a root file and a nested one splits them apart',
  grouped(['vector.yaml: app out', 'examples/demo.yaml: app out']),
  [
    { title: 'vector.yaml', files: ['vector.yaml'] },
    { title: 'examples/demo.yaml', files: ['examples/demo.yaml'] },
  ],
);

// -------------------------------------------------------------------- helpers

check('the common directory of files that share one', commonDirectory([
  { name: 'config/prod/a.yaml' },
  { name: 'config/prod/b.yaml' },
]), 'config/prod');

check('the common directory stops where they diverge', commonDirectory([
  { name: 'config/prod/a.yaml' },
  { name: 'config/staging/b.yaml' },
]), 'config');

check('files at the root share no directory', commonDirectory([
  { name: 'a.yaml' },
  { name: 'b.yaml' },
]), '');

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
