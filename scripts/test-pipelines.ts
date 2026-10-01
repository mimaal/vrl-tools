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

import { existsSync, readdirSync, readFileSync } from 'node:fs';
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
import { consumers, destination, outputsOf, written } from '../editors/vscode/src/outputs.js';
import { candidates, countRows } from '../editors/vscode/src/tablefiles.js';
import { errorsIn, pipeline, topology, topologyFiles } from './checker-harness.js';
import type { PipelineTopology } from './checker-harness.js';
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
    'fleet/config/00-module-demo-firewall.toml',
    'fleet/config/base.toml',
    'fleet/config/monitoring.toml',
    'fleet/config/normalize-router.toml',
    'fleet/config/product_a.toml',
    'fleet/config/product_b.yaml',
    'fleet/config/tables-assets.toml',
    'fleet/config/tables-geo.toml',
    'fleet/config/tables-threat.yaml',
    'fleet/config/topology.toml',
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
      // Two graphs that share nothing — the pipeline and Vector watching
      // itself — and still one config: no name clashes, so nothing splits.
      'fleet/config',
      [
        'fleet/config/00-module-demo-firewall.toml',
        'fleet/config/base.toml',
        'fleet/config/monitoring.toml',
        'fleet/config/normalize-router.toml',
        'fleet/config/product_a.toml',
        'fleet/config/product_b.yaml',
        'fleet/config/tables-assets.toml',
        'fleet/config/tables-geo.toml',
        'fleet/config/tables-threat.yaml',
        'fleet/config/topology.toml',
      ],
    ],
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
  'with nothing chosen, the one shown is the biggest real pipeline, not the first',
  groups[mainPipeline(groups, shapeOfFile)]?.title,
  'fleet/config',
);

// ---------------------------------------------------------------- the reading

const findingsOf = (title: string) => {
  const found = groups.find((entry) => entry.title === title);
  const analysis = found ? topologyFiles(found.files, found.title) : undefined;
  return analysis;
};

for (const found of groups.filter(
  (entry) => !['normalizer/config', 'fleet/config'].includes(entry.title),
)) {
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

// ------------------------------------------------------------------ at scale

/**
 * `readPipeline` in reading.ts, with the disk in place of the workspace: read
 * once to learn which programs the config keeps in files, find them the way
 * the extension does, and read again with them.
 */
function readWhole(
  files: readonly File[],
  title: string,
  options: Record<string, unknown> = {},
): PipelineTopology {
  const first = pipeline(files, title, options);
  const programs = first.programs.flatMap((wanted) => {
    const found = candidates(wanted.path, first.files[wanted.file] ?? '')
      .map((candidate) => path.join(CORPUS, candidate))
      .find((candidate) => existsSync(candidate));
    return found ? [{ path: wanted.path, source: readFileSync(found, 'utf8') }] : [];
  });
  return programs.length > 0 ? pipeline(files, title, { ...options, programs }) : first;
}

/** The component each placement of the layout is of. */
const placed = (analysis: PipelineTopology): string[] =>
  analysis.layout.components.map((placement) => analysis.components[placement.component]?.id ?? '?');

// fleet/ is the shape of the pipeline 0.8.0 was measured against: two routers
// merged across files, an overlay named to sort first, 372 `file` tables in
// three files, a monitoring graph beside the main one, and three outputs that
// end on purpose. `vector validate --config-dir config` 0.58.0, run from
// fleet/, loads it with exactly the three warnings below.
const fleetFiles = groups.find((entry) => entry.title === 'fleet/config')?.files ?? [];
const fleet = readWhole(fleetFiles, 'fleet/config');
const UNREAD = [
  'warning: nothing reads `route_by_product._unmatched`, so the events it produces go nowhere',
  'warning: nothing reads `normalize-router.imposible`, so the events it produces go nowhere',
  'warning: nothing reads `dropped-handler`, so the events it produces go nowhere',
];
check(
  'fleet/config: every file reads, and the findings are the three warnings Vector gives',
  {
    unreadable: fleet.unreadable,
    findings: fleet.findings.map((finding) => `${finding.severity}: ${finding.message}`).sort(),
  },
  { unreadable: [], findings: [...UNREAD].sort() },
);
check(
  'fleet/config: 372 tables are listed, by the file that declares them',
  Object.fromEntries(
    [...new Set(fleet.tables.map((table) => table.file))].map((file) => [
      fleet.files[file],
      fleet.tables.filter((table) => table.file === file).length,
    ]),
  ),
  {
    'fleet/config/tables-assets.toml': 124,
    'fleet/config/tables-geo.toml': 124,
    'fleet/config/tables-threat.yaml': 124,
  },
);
check(
  'fleet/config: the initial graph draws the pipeline and none of its tables',
  {
    nodes: placed(fleet).length,
    atMostThirty: placed(fleet).length <= 30,
    tables: placed(fleet).filter((id) => fleet.tables.some((table) => table.id === id)),
  },
  { nodes: 12, atMostThirty: true, tables: [] },
);
check(
  'fleet/config: the nodes come in the order events meet them, whatever file each is in',
  placed(fleet),
  [
    'input-http',
    'vector-metrics',
    'route_by_product',
    'firewall-demo-normalizer',
    'product-a-normalizer',
    'product-b-normalizer',
    'dropped-handler',
    'normalize-router',
    'time-diff',
    'metrics-out',
    'out',
    'unmatched',
  ],
);
check(
  'fleet/config: the monitoring graph is a band of its own, after the main one, each named for its source',
  fleet.layout.clusters.map((cluster) => [
    cluster.title,
    cluster.components.map((component) => fleet.components[component]?.id),
  ]),
  [
    [
      'input-http',
      [
        'input-http',
        'route_by_product',
        'firewall-demo-normalizer',
        'product-a-normalizer',
        'product-b-normalizer',
        'dropped-handler',
        'normalize-router',
        'time-diff',
        'out',
        'unmatched',
      ],
    ],
    ['vector-metrics', ['vector-metrics', 'metrics-out']],
  ],
);
check(
  'fleet/config: the export frames the two graphs apart',
  [
    fleet.document.includes('  subgraph c0["input-http"]\n'),
    fleet.document.includes('  subgraph c1["vector-metrics"]\n'),
  ],
  [true, true],
);
check(
  'fleet/config: the program a remap keeps in a file is found and read',
  fleet.programs,
  [{ component: 'product-a-normalizer', path: 'programs/product_a.vrl', file: 4, read: true }],
);
check(
  'fleet/config: each table says who reads it — inline, from a file, by keyword — and the rest are unused',
  Object.fromEntries(
    fleet.tables.filter((table) => table.readers.length > 0).map((table) => [table.id, table.readers]),
  ),
  {
    assets_001: ['firewall-demo-normalizer'],
    geo_001: ['product-a-normalizer'],
    geo_002: ['product-a-normalizer'],
    threat_001: ['product-b-normalizer'],
  },
);
check(
  'fleet/config: every table gives its CSV, and the rows are counted where it is found',
  [...new Set(fleet.tables.map((table) => `${table.type} ${table.path}`))].map((line) => {
    const table = fleet.tables.find((entry) => `${entry.type} ${entry.path}` === line);
    const found = candidates(table?.path ?? '', fleet.files[table?.file ?? 0] ?? '').find((candidate) =>
      existsSync(path.join(CORPUS, candidate)),
    );
    return [
      line,
      found,
      found && countRows(readFileSync(path.join(CORPUS, found), 'utf8'), table?.csvHeaders ?? true),
    ];
  }),
  [
    ['file tables/assets.csv', 'fleet/tables/assets.csv', 3],
    ['file tables/geo.csv', 'fleet/tables/geo.csv', 2],
    ['file tables/threat.csv', 'fleet/tables/threat.csv', 2],
  ],
);

const withTables = readWhole(fleetFiles, 'fleet/config', { showTables: true });
check(
  'fleet/config: asked for, the tables drawn are the four that are read, each joined to its reader',
  {
    nodes: placed(withTables).length,
    lookups: withTables.lookups.map((lookup) => `${lookup.table} -> ${lookup.reader}`).sort(),
  },
  {
    nodes: 16,
    lookups: [
      'assets_001 -> firewall-demo-normalizer',
      'geo_001 -> product-a-normalizer',
      'geo_002 -> product-a-normalizer',
      'threat_001 -> product-b-normalizer',
    ],
  },
);
const narrowed = readWhole(fleetFiles, 'fleet/config', { showTables: true, focus: 'product-b-normalizer' });
check(
  'fleet/config: narrowed to one remap, only its table is drawn, and the list is still whole',
  {
    tablesDrawn: placed(narrowed).filter((id) => narrowed.tables.some((table) => table.id === id)),
    listed: narrowed.tables.length,
  },
  { tablesDrawn: ['threat_001'], listed: 372 },
);

// What the sidebar says under a router: every output by the name an input
// uses for it, and who reads it.
const readers = consumers(fleet.edges);
const outputLines = (id: string): string[] => {
  const component = fleet.components.find((entry) => entry.id === id);
  return component
    ? outputsOf(component).map(
        (output) => `${written(id, output)} ${destination(readers.get(written(id, output)))}`,
      )
    : [];
};
check(
  'fleet/config: each output of a router says where it goes, or that nothing reads it',
  outputLines('normalize-router'),
  [
    'normalize-router.firewall-demo → time-diff',
    'normalize-router.imposible (unread)',
    'normalize-router.product_a → time-diff',
    'normalize-router.product_b → time-diff',
    'normalize-router._unmatched → unmatched',
  ],
);
check(
  'fleet/config: a default output is named by the component alone',
  [destination(readers.get('time-diff')), destination(readers.get('dropped-handler'))],
  ['→ out', '(unread)'],
);
check(
  'fleet/config: the Markdown export still draws Mermaid, and lists the edges as `a.out --> b`',
  [
    fleet.document.includes('```mermaid\nflowchart LR\n'),
    fleet.document.includes('-->|"route_by_product.product_a"| '),
    fleet.document.includes('\n- `route_by_product.product_a --> product-a-normalizer`\n'),
    fleet.document.includes('\n- `product-a-normalizer.dropped --> dropped-handler`\n'),
    fleet.document.includes('\n- `time-diff --> out`\n'),
    fleet.document.split('\n').filter((line) => / --> .*`$/.test(line)).length,
  ],
  [true, true, true, true, true, fleet.edges.length],
);

// ------------------------------------------------------------ the VRL in them

// What this corpus shows as VRL has to be VRL the pinned compiler accepts,
// against the tables its own pipeline declares.
const tablesOf = (file: string): string[] =>
  file.startsWith('fleet/') ? fleet.tables.map((table) => table.id) : [];
const programs = [
  ...guessed.flatMap((file) =>
    [
      ...file.source.matchAll(/^\s*source\s*=\s*'''\n([\s\S]*?)'''/gm),
      ...file.source.matchAll(/^(\s*)source:\s*\|\n((?:\1\s+.*\n?)+)/gm),
    ].map((match) => ({ file: file.name, program: match[match.length - 1] })),
  ),
  ...all
    .filter((file) => file.name.endsWith('.vrl'))
    .map((file) => ({ file: file.name, program: file.source })),
];
check('every remap in the corpus is found to compile', programs.length, 11);
for (const { file, program } of programs) {
  check(
    `${file}: \`${program.trim().split('\n')[0]}\` compiles`,
    errorsIn(program, undefined, tablesOf(file)).map((e) => e.message),
    [],
  );
}

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
