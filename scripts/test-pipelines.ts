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

import { declaredNames, declaresVectorSection, group } from '../editors/vscode/src/grouping.js';
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
    'normalizer/config/base.toml',
    'normalizer/config/monitoring/vector.yaml',
    'normalizer/config/product_a.toml',
    'normalizer/config/product_b.yaml',
  ],
);

// --------------------------------------------------------------- the grouping

// `componentNames` in topology.ts: the wasm module's reading of one file.
const namesOf = (file: File): string[] => {
  const read = topology(file.source, file.name);
  return 'error' in read ? [] : declaredNames(read.components);
};
const groups = group(guessed, 'pipelines', namesOf);
check(
  'the pipelines are the ones a person would name',
  groups.map((found) => [found.title, found.files.map((file) => file.name)]),
  [
    ['examples/one.yaml', ['examples/one.yaml']],
    ['examples/two.yaml', ['examples/two.yaml']],
    [
      'normalizer/config',
      [
        'normalizer/config/base.toml',
        'normalizer/config/product_a.toml',
        'normalizer/config/product_b.yaml',
      ],
    ],
    ['normalizer/config/monitoring/vector.yaml', ['normalizer/config/monitoring/vector.yaml']],
  ],
);

// ---------------------------------------------------------------- the reading

for (const found of groups) {
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

const normalizer = groups.find((found) => found.title === 'normalizer/config');
if (normalizer) {
  const merged = topologyFiles(normalizer.files, normalizer.title);
  check(
    'the router has the routes of every product, from both formats',
    merged.components.filter((c) => c.id === 'route_by_product').map((c) => [c.type, c.namedOutputs]),
    [['exclusive_route', ['product_a', 'product_b', '_unmatched']]],
  );
  check(
    'the unquoted variable does not cost base.toml its sources and sinks',
    merged.components.map((c) => c.id).filter((id) => ['input-http', 'out', 'unrouted'].includes(id)),
    ['input-http', 'out', 'unrouted'],
  );

  // The same files, as Vector reads them when given with `--config`: each on
  // its own, so the product files are pieces with nowhere to go.
  const alone = topologyFiles(
    normalizer.files.map((file) => ({ ...file, standalone: true })),
    normalizer.title,
  );
  check(
    'given with --config, each product file is reported, and says why',
    alone.findings
      .filter((finding) => finding.message.includes('has no `type`'))
      .map((finding) => [
        alone.files[(finding as { file?: number }).file ?? -1],
        finding.message.includes('`--config` is loaded on its own'),
      ]),
    [
      ['normalizer/config/product_a.toml', true],
      ['normalizer/config/product_b.yaml', true],
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
check('every remap in the corpus is found to compile', programs.length, 3);
for (const { file, program } of programs) {
  check(`${file}: \`${program.trim()}\` compiles`, errorsIn(program).map((e) => e.message), []);
}

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
