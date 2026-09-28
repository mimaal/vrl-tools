/**
 * Walks test-corpus/pipelines/ the way the extension walks a workspace folder,
 * and checks every step between "a folder of files" and "the findings".
 *
 * Each step has tests of its own: `test:grouping` for the grouping, the Rust
 * tests for merging and interpolation. What none of them tests is the path
 * that finds the input — which files are guessed to be Vector's, which of
 * those end up together, and what the wasm module says about each group once
 * they do. That path read a whole workspace as one config for three releases,
 * and read one real pipeline as 413, without any test noticing, because every
 * test was handed its files. This one is handed a folder.
 *
 * Only `vscode.workspace.findFiles` is replaced, by a walk of the disk.
 *
 * Run with: npm run test:pipelines (after npm run build:wasm)
 */

import { readdirSync, readFileSync } from 'node:fs';
import * as path from 'node:path';

import {
  declaresVectorSection,
  EMPTY,
  group,
  mainPipeline,
  pipelinesOnly,
  shapeOf,
} from '../editors/vscode/src/grouping.js';
import type { Shape } from '../editors/vscode/src/grouping.js';
import { errorsIn, topology, topologyFiles } from './checker-harness.js';
import { ROOT } from './grammar-harness.js';

const CORPUS = path.join(ROOT, 'test-corpus', 'pipelines');

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

interface File {
  readonly name: string;
  readonly source: string;
}

/** Every file under `dir`, named relative to the corpus with `/`. */
function walk(dir: string): File[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      return walk(full);
    }
    const name = path.relative(CORPUS, full).split(path.sep).join('/');
    return [{ name, source: readFileSync(full, 'utf8') }];
  });
}

const all = walk(CORPUS).sort((a, b) => a.name.localeCompare(b.name));

// ------------------------------------------------------------------ the guess

// `GUESS_GLOB` in pipeline.ts, then `declaresVectorSection`.
const guessed = all.filter(
  (file) => /\.(ya?ml|toml)$/.test(file.name) && declaresVectorSection(file.source),
);
check(
  'the guess keeps the configs and leaves the compose file and the Helm values out',
  guessed.map((file) => file.name),
  [
    'examples/one.yaml',
    'examples/two.yaml',
    'normalizer/config/00-overlay.toml',
    'normalizer/config/base.toml',
    'normalizer/config/devices-available/product_a.toml',
    'normalizer/config/devices-available/product_b.yaml',
    'normalizer/config/monitoring/vector.yaml',
    'normalizer/config/normalize-router.toml',
    'normalizer/config/product_a.toml',
    'normalizer/config/product_b.yaml',
    'normalizer/config/topology.toml',
  ],
);

// --------------------------------------------------------------- the grouping

// `componentNames` in topology.ts: the wasm module's reading of one file.
const shapeOfFile = (file: File): Shape => {
  const read = topology(file.source, file.name);
  return 'error' in read ? EMPTY : shapeOf(read.components);
};
const groups = pipelinesOnly(group(guessed, 'pipelines', shapeOfFile), shapeOfFile);
check(
  // devices-available/ splits off, because its product files repeat config's,
  // and is then dropped: it has no sources or sinks of its own.
  'the pipelines are the ones a person would name, and devices-available is not one',
  groups.map((found) => [found.title, found.files.map((file) => file.name)]),
  [
    ['examples/one.yaml', ['examples/one.yaml']],
    ['examples/two.yaml', ['examples/two.yaml']],
    [
      'normalizer/config',
      [
        'normalizer/config/00-overlay.toml',
        'normalizer/config/base.toml',
        'normalizer/config/normalize-router.toml',
        'normalizer/config/product_a.toml',
        'normalizer/config/product_b.yaml',
        'normalizer/config/topology.toml',
      ],
    ],
    ['normalizer/config/monitoring/vector.yaml', ['normalizer/config/monitoring/vector.yaml']],
  ],
);
check(
  'with nothing chosen, the one shown is the real pipeline, not the first',
  groups[mainPipeline(groups, shapeOfFile)]?.title,
  'normalizer/config',
);

// ---------------------------------------------------------------- the reading

const findingsOf = (title: string) => {
  const found = groups.find((entry) => entry.title === title);
  const analysis = found ? topologyFiles(found.files, found.title) : undefined;
  return analysis;
};

for (const found of groups.filter((entry) => entry.title !== 'normalizer/config')) {
  const analysis = topologyFiles(found.files, found.title);
  check(
    `${found.title}: every file reads, and Vector would start on it`,
    {
      unreadable: analysis.unreadable,
      findings: analysis.findings.map((finding) => finding.message),
    },
    { unreadable: [], findings: [] },
  );
}

const merged = findingsOf('normalizer/config');
if (merged) {
  check(
    // Nothing is there to read the routers' unmatched events: that is a real
    // warning, and the only kind this pipeline should get.
    'normalizer/config: every file reads, and the only findings are the real unread outputs',
    {
      unreadable: merged.unreadable,
      findings: merged.findings.map((finding) => `${finding.severity}: ${finding.message}`),
    },
    {
      unreadable: [],
      findings: [
        'warning: nothing reads `route_by_product._unmatched`, so the events it produces go nowhere',
        'warning: nothing reads `normalize-router._unmatched`, so the events it produces go nowhere',
      ],
    },
  );
  const routes = (id: string) => merged.components.find((c) => c.id === id)?.namedOutputs;
  check(
    'the routes come in file-name order, the overlay first, from both formats',
    [routes('route_by_product'), routes('normalize-router')],
    [
      ['overlay', 'product_a', 'product_b', '_unmatched'],
      ['product_overlay', 'product_a', 'product_b', '_unmatched'],
    ],
  );
  const router = merged.components.find((c) => c.id === 'route_by_product') as
    | { file: number; outputOrigins?: { file: number }[] }
    | undefined;
  check(
    'each route goes to the file that added it, the router to the one with its type',
    router && {
      router: merged.files[router.file],
      routes: router.outputOrigins?.map((origin) => merged.files[origin.file]),
    },
    {
      router: 'normalizer/config/topology.toml',
      routes: [
        'normalizer/config/00-overlay.toml',
        'normalizer/config/product_a.toml',
        'normalizer/config/product_b.yaml',
        'normalizer/config/topology.toml',
      ],
    },
  );
  check(
    'the unquoted variable does not cost base.toml its sources and sinks',
    merged.components.map((c) => c.id).filter((id) => ['input-http', 'out'].includes(id)),
    ['input-http', 'out'],
  );

  // The same files, as Vector reads them when given with `--config`: each on
  // its own, so every file adding a route is a file of pieces with nowhere to go.
  const found = groups.find((entry) => entry.title === 'normalizer/config');
  const alone = topologyFiles(
    (found?.files ?? []).map((file) => ({ ...file, standalone: true })),
    'normalizer/config',
  );
  check(
    'given with --config, each file adding a route is reported, and says why',
    [
      ...new Set(
        alone.findings
          .filter((finding) => finding.message.includes('`--config` is loaded on its own'))
          .map((finding) => alone.files[(finding as { file?: number }).file ?? -1]),
      ),
    ],
    [
      'normalizer/config/00-overlay.toml',
      'normalizer/config/product_a.toml',
      'normalizer/config/product_b.yaml',
    ],
  );
} else {
  check('normalizer/config is found', false, true);
}

// ------------------------------------------------------------ the VRL in them

// What this corpus shows as VRL has to be VRL the pinned compiler accepts.
const programs = guessed.flatMap((file) =>
  [
    ...file.source.matchAll(/^\s*source\s*=\s*'''\n([\s\S]*?)'''/gm),
    ...file.source.matchAll(/^(\s*)source:\s*\|\n((?:\1\s+.*\n?)+)/gm),
  ].map((match) => ({ file: file.name, program: match[match.length - 1] })),
);
check('every remap in the corpus is found to compile', programs.length, 6);
for (const { file, program } of programs) {
  check(`${file}: \`${program.trim()}\` compiles`, errorsIn(program).map((e) => e.message), []);
}

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
