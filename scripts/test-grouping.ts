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

import { Exclusions, gitignoreRules, globToRegExp, settingRules } from '../editors/vscode/src/excludes.js';
import {
  commonDirectory,
  declaredNames,
  declaresVectorSection,
  group,
  mainPipeline,
  pipelinesOnly,
  shapeOf,
} from '../editors/vscode/src/grouping.js';
import type { Shape } from '../editors/vscode/src/grouping.js';

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

interface Spec {
  readonly name: string;
  readonly shape: Shape;
}

/**
 * A file, written as `path: componentName componentName ...` — the names it
 * declares with a `type`. Unless `rest` says otherwise it is a complete
 * config, with a source and a sink, which is what every file of the tests
 * written before shapes existed was.
 */
function file(spec: string, rest: Partial<Shape> = {}): Spec {
  const [name, names] = spec.split(':');
  return {
    name: name.trim(),
    shape: {
      declared: (names ?? '').trim().split(/\s+/).filter(Boolean),
      pieces: [],
      sources: 1,
      sinks: 1,
      ...rest,
    },
  };
}

/** A file with neither sources nor sinks: part of a config, not one. */
const part = (spec: string, rest: Partial<Shape> = {}): Spec =>
  file(spec, { sources: 0, sinks: 0, ...rest });

function grouped(specs: readonly (string | Spec)[]): { title: string; files: string[] }[] {
  return group(
    specs.map((spec) => (typeof spec === 'string' ? file(spec) : spec)),
    'workspace',
    (f) => f.shape,
  ).map((g) => ({
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
  grouped([
    file('config/base.toml: in split out'),
    part('config/a.toml: a', { pieces: ['split'] }),
    part('config/b.toml: b', { pieces: ['split'] }),
  ]).map((g) => g.title),
  ['config'],
);

check(
  'a shape is read from the components: typed names, pieces, sources and sinks',
  shapeOf([
    { id: 'in', type: 'stdin', role: 'source' },
    { id: 'split', type: '', role: 'transform' },
    { id: 'out', type: 'console', role: 'sink' },
    { id: 'geo', type: 'file', role: 'table' },
  ]),
  { declared: ['in', 'out', 'geo'], pieces: ['split'], sources: 1, sinks: 1 },
);

// ------------------------------------ a declaration repeated in one directory

check(
  // The second row of the table measured against `vector validate
  // --config-dir`: b.toml restates the router it adds a route to. Vector merges
  // it, so it is not a clash — the wasm module reads the two as one router.
  'a file restating a declaration of its directory is not a clash',
  grouped([file('config/a.toml: in r out'), part('config/b.toml: r')]).map((g) => g.title),
  ['config'],
);

check(
  'two complete configs sharing names in one directory are still two',
  grouped(['examples/a.yaml: app out', 'examples/b.yaml: app out']).map((g) => g.title),
  ['examples/a.yaml', 'examples/b.yaml'],
);

check(
  // Vector merges within one --config-dir only: across directories a shared
  // name is a duplicate, restated declaration or not.
  'a declaration restated in another directory is a clash',
  grouped([file('config/prod/a.toml: in r out'), part('config/staging/b.toml: r')]).map(
    (g) => g.title,
  ),
  ['config/prod/a.toml', 'config/staging/b.toml'],
);

// ------------------------------------------------- directories that are parts

{
  // The shape of `normalizer`: config/ is the pipeline, devices-available/
  // holds the same product files ready to be copied in. Their names clash, so
  // they split; devices-available has no sources or sinks, and every one of
  // its names is config's.
  const files = [
    file('config/base.toml: input out'),
    part('config/topology.toml: route_by_product'),
    part('config/product_a.toml: normalize-a', { pieces: ['route_by_product'] }),
    part('config/devices-available/product_a.toml: normalize-a', { pieces: ['route_by_product'] }),
    part('config/devices-available/product_b.toml: normalize-b', { pieces: ['route_by_product'] }),
    file('config/monitoring/vector.yaml: metrics out'),
  ];
  const groups = group(files, 'workspace', (f) => f.shape);
  check(
    'a directory of parts splits off from the pipeline it belongs to',
    groups.map((g) => g.title),
    ['config', 'config/devices-available', 'config/monitoring/vector.yaml'],
  );
  check(
    'and is not offered as a pipeline',
    pipelinesOnly(groups, (f: Spec) => f.shape).map((g) => g.title),
    ['config', 'config/monitoring/vector.yaml'],
  );
}

check(
  'a workspace of nothing but parts keeps them: there is nothing better to show',
  pipelinesOnly(
    group([part('transforms/a.toml: a'), part('transforms/b.toml: b')], 'w', (f) => f.shape),
    (f: Spec) => f.shape,
  ).map((g) => g.title),
  ['transforms'],
);

// ----------------------------------------------------------- the default one

check(
  // Not the first alphabetically: config-monitoring sorts before config-pipeline.
  'the default pipeline is the biggest one that can run',
  (() => {
    const groups = [
      { files: [file('config-monitoring/vector.yaml: metrics out')] },
      { files: [file('config-pipeline/base.toml: in a b c out')] },
      { files: [part('parts/x.toml: x y z w v u t')] },
    ];
    return mainPipeline(groups, (f: Spec) => f.shape);
  })(),
  1,
);

check('no pipelines, no default', mainPipeline([], (f: Spec) => f.shape), -1);

// ---------------------------------------------------- what the guess ignores

{
  const rules = new Exclusions([
    ...gitignoreRules('# scratch\ntmp.*\n/build/\n!keep.toml\nlogs/*.yaml\n'),
    ...settingRules({ '**/.history': true, '**/off': false, '**/when': { when: 'x' } }),
  ]);
  for (const [path, expected] of [
    ['tmp.scratch/vector.toml', true],
    ['config/tmp.old/vector.toml', true],
    ['build/vector.toml', true],
    ['config/build/vector.toml', false],
    ['logs/a.yaml', true],
    ['logs/deep/a.yaml', false],
    ['.history/vector.toml', true],
    ['off/vector.toml', false],
    ['when/vector.toml', false],
    ['config/vector.toml', false],
  ] as const) {
    check(`${expected ? 'ignores' : 'keeps'} ${path}`, rules.excludes(path), expected);
  }
}

check(
  'a directory-only pattern does not hide a file of that name',
  new Exclusions(gitignoreRules('out/\n')).excludes('config/out'),
  false,
);

check(
  'globs: braces, classes and double stars',
  [
    globToRegExp('**/{node_modules,.git}/**').test('a/node_modules/x/y.js'),
    globToRegExp('*.{yaml,yml}').test('a.yml'),
    globToRegExp('*.{yaml,yml}').test('dir/a.yml'),
    globToRegExp('file[0-9].toml').test('file7.toml'),
  ],
  [true, true, false, true],
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
