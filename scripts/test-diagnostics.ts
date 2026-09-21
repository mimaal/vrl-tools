/**
 * Exercises the diagnostics pipeline end to end: the wasm module the .vsix
 * ships, loaded from Node exactly as the extension loads it.
 *
 * The Rust tests already cover the checker itself. What only this side can
 * cover is the boundary — that positions and notes survive the trip through
 * JSON — and the corpus, where four questions matter:
 *
 *   - every `.vrl` file in test-corpus/ compiles clean, so a version bump that
 *     changes the language fails here rather than in someone's editor;
 *   - every region the injection grammars paint as VRL inside a Vector config
 *     compiles clean too. Colouring something as VRL that the compiler would
 *     reject is a promise the extension cannot keep;
 *   - every program with a sample event next to it still compiles when typed
 *     against that sample, and actually runs on it;
 *   - the standard library dump says what the compiler says.
 *
 * Run with: npm run test:diagnostics
 */

import { readdir, readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';

import {
  check,
  enrichmentTables,
  errorsIn,
  run,
  stdlib,
  topology,
  topologyFiles,
  vrlVersion,
} from './checker-harness.js';
import { CORPUS, embeddedRegions, loadGrammar, ROOT } from './grammar-harness.js';

let failed = 0;

function ok(what: string): void {
  console.log(`ok    ${what}`);
}

function fail(what: string, detail: string): void {
  console.error(`FAIL  ${what}\n      ${detail.split('\n').join('\n      ')}`);
  failed++;
}

/** Describes diagnostics compactly enough to read in a test failure. */
function describe(source: string, lineOffset = 0, tables: readonly string[] = []): string {
  return check(source, undefined, tables)
    .diagnostics.map(
      (d) =>
        `line ${d.range.start.line + 1 + lineOffset}: ${d.severity} E${d.code} ${d.message}`,
    )
    .join('\n');
}

async function checkVersionAgrees(): Promise<void> {
  const pkg = JSON.parse(await readFile(path.join(ROOT, 'package.json'), 'utf8')) as {
    vrl?: { crateVersion?: string };
  };
  const pinned = pkg.vrl?.crateVersion;

  if (vrlVersion() !== pinned) {
    fail(
      'the module reports the pinned crate version',
      `package.json pins ${String(pinned)}, the module was built against ${vrlVersion()}.\n` +
        'Rebuild it with: npm run build:wasm',
    );
    return;
  }
  ok(`the module is built against the pinned vrl ${vrlVersion()}`);
}

async function checkCorpusPrograms(): Promise<void> {
  const files = (await readdir(CORPUS)).filter((f) => f.endsWith('.vrl'));

  if (files.length === 0) {
    fail('the corpus has programs to compile', 'test-corpus/ has no .vrl files');
    return;
  }

  for (const file of files) {
    const source = await readFile(path.join(CORPUS, file), 'utf8');
    const result = check(source);

    // Warnings count here as well as errors. The corpus is what the extension
    // is judged against, so it has to be VRL worth copying.
    if (result.diagnostics.length > 0) {
      fail(`corpus ${file} compiles clean`, describe(source));
      continue;
    }
    ok(`corpus ${file} compiles clean`);
  }
}

/**
 * The corpus programs against the sample events next to them: the phase 5
 * path, end to end through the shipped module.
 *
 * Three things have to hold, and each of them was broken at some point while
 * this was written:
 *
 *   - a sample must not turn a working parser into a broken one. A parser is
 *     defensive by design, and a sample that happens to have every field makes
 *     the compiler call that defensiveness redundant;
 *   - a sample must not forbid building a new shape. `.event.original = …` is
 *     most of what a mapping program does, and a sample with no `.event` must
 *     not make that an error;
 *   - what the editor says and what running the program does have to agree. If
 *     no errors are shown, it runs.
 */
async function checkSampleTypedPrograms(): Promise<void> {
  const files = (await readdir(CORPUS)).filter((f) => f.endsWith('.vrl'));
  let sampled = 0;

  for (const file of files) {
    const samplePath = path.join(CORPUS, `${file}.sample.json`);
    let sample: string;
    try {
      sample = await readFile(samplePath, 'utf8');
    } catch {
      continue;
    }
    sampled++;

    const source = await readFile(path.join(CORPUS, file), 'utf8');
    const result = check(source, sample);

    if (!result.typedWithSample) {
      fail(`${file} is typed against its sample`, `sampleError: ${String(result.sampleError)}`);
      continue;
    }

    const errors = result.diagnostics.filter((d) => d.severity === 'error' || d.severity === 'bug');
    if (errors.length > 0) {
      fail(
        `${file} still compiles when typed against its sample`,
        errors.map((e) => `line ${e.range.start.line + 1}: E${e.code} ${e.message}`).join('\n'),
      );
      continue;
    }
    ok(`${file} compiles when typed against its sample`);

    // Every diagnostic left has to be one the sample itself provoked;
    // anything else would be a regression the no-sample check already covers.
    const unexplained = result.diagnostics.filter((d) => !d.relaxedBySample);
    if (unexplained.length > 0) {
      fail(
        `${file}: every remaining diagnostic is explained by the sample`,
        unexplained
          .map((d) => `line ${d.range.start.line + 1}: ${d.severity} E${d.code} ${d.message}`)
          .join('\n'),
      );
    }

    const ran = run(source, sample);
    if (!ran.compiled || ran.error || ran.aborted) {
      fail(
        `${file} runs on its sample event`,
        `compiled: ${ran.compiled}, aborted: ${ran.aborted}, error: ${String(ran.error)}`,
      );
      continue;
    }
    if (!ran.event || typeof ran.event !== 'object') {
      fail(`${file} produces an event`, `got ${JSON.stringify(ran.event)}`);
      continue;
    }
    ok(`${file} runs on its sample and produces an event`);
  }

  if (sampled === 0) {
    fail('the corpus exercises sample events', 'no .vrl.sample.json next to any corpus program');
  }
}

/**
 * The rule that keeps a sample from doing more harm than good, checked
 * directly rather than only through the corpus.
 */
function checkSamplePolicy(): void {
  const defensive = '.host = string(.hostname) ?? "unknown"\n';
  const withSample = check(defensive, '{"hostname":"web-01"}');

  if (!withSample.compiled) {
    fail('a defensive program survives a sample', describe(defensive));
  } else if (!withSample.diagnostics.some((d) => d.relaxedBySample && d.severity === 'warning')) {
    fail(
      'redundant error handling is reported as a warning',
      JSON.stringify(withSample.diagnostics),
    );
  } else {
    ok('a defensive program survives a sample, with a warning rather than an error');
  }

  // Redundant whatever the event looks like: the sample had nothing to do
  // with it, so it stays an error.
  const always = check('.x = 1 ?? 2\n', '{"message":"hello"}');
  if (always.diagnostics.some((d) => d.code === 651 && d.severity === 'error')) {
    ok('error handling that is always redundant stays an error');
  } else {
    fail('error handling that is always redundant stays an error', JSON.stringify(always.diagnostics));
  }

  // Building a new shape is most of what VRL is for.
  const mapping = check('.event.original = string!(.message)\n', '{"message":"hello"}');
  if (mapping.compiled) {
    ok('a sample does not forbid creating a field it does not have');
  } else {
    fail(
      'a sample does not forbid creating a field it does not have',
      JSON.stringify(mapping.diagnostics),
    );
  }

  // And the headline: a field the sample does have gets its real type.
  const typed = check('.level = downcase(.message)\n', '{"message":"HELLO"}');
  const untyped = check('.level = downcase(.message)\n');
  if (typed.compiled && !untyped.compiled) {
    ok('a known field type turns a fallible call into an infallible one');
  } else {
    fail(
      'a known field type turns a fallible call into an infallible one',
      `with sample: ${typed.compiled}, without: ${untyped.compiled}`,
    );
  }
}

async function checkEmbeddedPrograms(): Promise<void> {
  const configs: readonly { file: string; scope: string }[] = [
    { file: 'vector.yaml', scope: 'vrl.injection.yaml' },
    { file: 'vector.toml', scope: 'vrl.injection.toml' },
  ];

  for (const { file, scope } of configs) {
    const grammar = await loadGrammar(scope);
    const config = await readFile(path.join(CORPUS, file), 'utf8');
    const regions = embeddedRegions(grammar, config);

    // Compiled the way Vector compiles them: against the tables this config
    // declares, read by the same module the extension reads them with.
    const tables = enrichmentTables(config, file);
    if (!tables?.includes('services')) {
      fail(`${file} declares its enrichment table`, `read: ${JSON.stringify(tables)}`);
      continue;
    }

    if (regions.length === 0) {
      fail(`${file} has VRL to compile`, 'the injection grammar found no embedded region');
      continue;
    }

    for (const region of regions) {
      const result = check(region.source, undefined, tables);
      if (result.diagnostics.length > 0) {
        fail(
          `${file}: the VRL block at line ${region.line} compiles clean`,
          describe(region.source, region.line - 1, tables),
        );
        continue;
      }
      ok(`${file}: the VRL block at line ${region.line} compiles clean`);
    }
  }
}

/**
 * The functions Vector adds to the standard library. Without them every
 * enrichment lookup was "call to undefined function", in a tool whose point is
 * to have no false positives.
 */
function checkVectorFunctions(): void {
  const lookup = '.owner = get_enrichment_table_record!("hosts", {"host": .host})\n';

  const declared = errorsIn(lookup, undefined, ['hosts']);
  if (declared.length > 0) {
    fail('a lookup in a declared table compiles', declared.map((d) => d.message).join('\n'));
  } else {
    ok('a lookup in a declared table compiles');
  }

  const undeclared = errorsIn(lookup)[0];
  if (!undeclared) {
    fail('a lookup in an undeclared table is an error', 'the compiler accepted it');
  } else if (!undeclared.notes.some((note) => note.includes('enrichment_tables'))) {
    fail(
      'with no tables declared, the error says where they come from',
      JSON.stringify(undeclared.notes),
    );
  } else {
    ok('a lookup in an undeclared table is an error that says where tables come from');
  }

  const secrets = errorsIn('.key = get_secret("api_key")\n');
  if (secrets.length > 0) {
    fail('get_secret is defined', secrets.map((d) => d.message).join('\n'));
  } else {
    ok('get_secret is defined');
  }

  const names = new Set(stdlib().functions.map((f) => f.name));
  const missing = [
    'find_enrichment_table_records',
    'get_enrichment_table_record',
    'get_secret',
  ].filter((name) => !names.has(name));
  if (missing.length > 0) {
    fail('hover and completion know the functions Vector adds', `missing: ${missing.join(', ')}`);
  } else {
    ok('hover and completion know the functions Vector adds');
  }
}

function checkBoundary(): void {
  const unhandled = errorsIn('.parsed = parse_json(.message)\n');
  const first = unhandled[0];

  if (!first) {
    fail('an unhandled fallible assignment is an error', 'the compiler accepted it');
  } else if (first.code !== 103) {
    fail('an unhandled fallible assignment is E103', `got E${first.code} ${first.message}`);
  } else if (first.range.start.line !== 0 || first.range.start.character !== 10) {
    fail(
      'the error underlines the call, not the whole line',
      `expected 0:10, got ${first.range.start.line}:${first.range.start.character}`,
    );
  } else if (first.notes.length === 0) {
    fail('the compiler notes survive the boundary', 'the diagnostic arrived with no notes');
  } else if (first.documentationUrl !== 'https://errors.vrl.dev/103') {
    fail('E103 carries its documentation link', `got ${String(first.documentationUrl)}`);
  } else {
    ok('an unhandled fallible assignment arrives as E103, underlined at the call');
  }

  // The line below the accents is what matters: if columns were byte offsets
  // the squiggle would land four characters to the right of the call, and an
  // emoji outside the BMP would push it further still.
  const wide = '.mensaje = "café con leña 🚀"\n.y = definitely_not_a_function(.mensaje)\n';
  const unknown = errorsIn(wide)[0];

  if (!unknown) {
    fail('an unknown function is an error', 'the compiler accepted it');
  } else if (unknown.range.start.line !== 1 || unknown.range.start.character !== 5) {
    fail(
      'columns are UTF-16, not bytes',
      `expected 1:5, got ${unknown.range.start.line}:${unknown.range.start.character}`,
    );
  } else {
    ok('columns stay UTF-16 across accents and an astral-plane emoji');
  }

  // A half-written program is the normal state of a file being typed in. The
  // module has to answer, not trap.
  try {
    const partial = check('.foo = \n');
    if (partial.compiled || partial.diagnostics.length === 0) {
      fail('a syntax error is reported', 'the compiler accepted an incomplete assignment');
    } else {
      ok('a half-typed program answers with a diagnostic instead of trapping');
    }
  } catch (error) {
    fail('a half-typed program does not trap the module', String(error));
  }
}

/**
 * The standard library dump, which hover, completion and signature help are
 * built out of. Everything asserted here is something the editor would show a
 * person, so a wrong answer is a wrong answer on screen.
 */
function checkStdlib(): void {
  const dump = stdlib();

  if (dump.functions.length < 150) {
    fail('the dump carries the whole standard library', `only ${dump.functions.length} functions`);
    return;
  }
  ok(`the dump carries ${dump.functions.length} functions`);

  const byName = new Map(dump.functions.map((f) => [f.name, f]));
  const missing = ['parse_json', 'parse_syslog', 'to_int', 'del', 'exists', 'now'].filter(
    (name) => !byName.has(name),
  );
  if (missing.length > 0) {
    fail('the functions everyone uses are present', `missing: ${missing.join(', ')}`);
  } else {
    ok('the functions everyone uses are present');
  }

  // The one thing an editor must not get backwards.
  const cases: readonly [string, boolean][] = [
    ['parse_json', true],
    ['to_int', true],
    ['string', true],
    ['downcase', false],
    ['now', false],
  ];
  const wrong = cases.filter(([name, fallible]) => byName.get(name)?.fallible !== fallible);
  if (wrong.length > 0) {
    fail(
      'fallibility comes through as the compiler sees it',
      wrong.map(([name]) => `${name}: ${String(byName.get(name)?.fallible)}`).join(', '),
    );
  } else {
    ok('fallibility comes through as the compiler sees it');
  }

  const forEach = byName.get('for_each');
  if (forEach?.closure?.variables.join(', ') !== 'key, value') {
    fail(
      'a closure function suggests usable variables',
      `for_each closure: ${JSON.stringify(forEach?.closure)}`,
    );
  } else {
    ok('a closure function suggests usable variables');
  }

  // 190 functions inherit the trait's "TODO" prose. Passing it along would put
  // the word TODO in a hover.
  const todo = dump.functions.filter((f) => f.summary === 'TODO' || f.usage === 'TODO');
  if (todo.length > 0) {
    fail('the placeholder prose never reaches a hover', `${todo.length} functions carry "TODO"`);
  } else {
    ok('the placeholder prose never reaches a hover');
  }

  const unanswered = dump.functions.filter((f) => f.fallible === null).length;
  ok(`${dump.functions.length - unanswered} of ${dump.functions.length} functions probed`);
}

/**
 * A wasm trap must not end the session.
 *
 * Roughly 800 nested brackets overflow the stack inside the parser. A trap is
 * not recoverable from the inside: the instance is finished, and every later
 * call into it throws the same `memory access out of bounds`, including
 * `vrl_version`. One module is loaded per session, so before `VrlChecker`
 * learned to replace it, a single such file ended diagnostics, hover and
 * completion until the window was reloaded.
 *
 * This drives the extension's own class rather than the harness, because
 * replacing the module is exactly what is under test.
 */
/**
 * The topology of the configs in `test-corpus/topology/`, across the boundary.
 *
 * The Rust tests already cover the resolving, in far more detail than this.
 * What only this side can cover is that the answer survives the trip: that a
 * null output stays null rather than arriving as the string "null", that the
 * camelCase field names are what the extension reads, and that the document
 * comes through with its Mermaid block intact.
 */
async function checkTopology(): Promise<void> {
  const dir = path.join(CORPUS, 'topology');

  const straight = topology(
    await readFile(path.join(dir, 'straight.yaml'), 'utf8'),
    'straight.yaml',
  );
  if ('error' in straight) {
    fail('the corpus topology reads', straight.error.message);
    return;
  }

  if (straight.findings.length === 0) {
    ok('a working config produces no findings');
  } else {
    fail(
      'a working config produces no findings',
      straight.findings.map((f) => f.message).join('; '),
    );
  }

  const outputs = straight.edges.map((edge) => edge.output);
  if (outputs.every((output) => output === null)) {
    ok('a default output crosses the boundary as null');
  } else {
    fail('a default output crosses the boundary as null', JSON.stringify(outputs));
  }

  const named = topology(
    await readFile(path.join(dir, 'named-outputs.yaml'), 'utf8'),
    'named-outputs.yaml',
  );
  if ('error' in named) {
    fail('named outputs read', named.error.message);
    return;
  }

  // What the graph panel draws from. Every component has a place, and every
  // arrow points right: a panel receiving anything else would draw arrows
  // through boxes or boxes on top of each other.
  const placed = named.layout.components;
  const column = (id: string): number => {
    const index = named.components.findIndex((c) => c.id === id);
    return placed.find((p) => p.component === index)?.column ?? -1;
  };
  const places = new Set(placed.map((p) => `${p.column}:${p.row}`));
  if (placed.length !== named.components.length || places.size !== placed.length) {
    fail('the layout places every component once', JSON.stringify(named.layout));
  } else if (named.edges.some((edge) => column(edge.from) >= column(edge.to))) {
    fail('every arrow in the layout points right', JSON.stringify(named.layout));
  } else if (named.layout.routes.length === 0) {
    fail(
      'an arrow skipping columns crosses in a lane',
      `strict.dropped skips a column but no route came back: ${JSON.stringify(named.layout)}`,
    );
  } else {
    ok('the layout crosses the boundary: every component placed, arrows right, lanes for long ones');
  }

  // A pipeline split across files, the way `vector -c 'config/**/*.toml'`
  // reads it: the transform's input names a source in another file.
  const split = topologyFiles(
    [
      { name: 'config/vector.toml', source: ['[sources.app]', 'type = "file"', ''].join('\n') },
      {
        name: 'config/nginx/parse.toml',
        source: [
          '[transforms.parse]',
          'type = "remap"',
          'inputs = ["app"]',
          '',
          '[sinks.out]',
          'type = "console"',
          'inputs = ["parse"]',
          '',
        ].join('\n'),
      },
    ],
    'config',
  );
  if (split.findings.length > 0 || split.edges.length !== 2 || split.unreadable.length > 0) {
    fail('a pipeline split across files is one graph', JSON.stringify(split.findings));
  } else {
    ok('a pipeline split across files is one graph, with no false "no component" errors');
  }

  const dropped = named.edges.find((edge) => edge.output === 'dropped');
  if (dropped && dropped.from === 'strict' && dropped.to === 'leftovers') {
    ok('a named output arrives with its edge');
  } else {
    fail('a named output arrives with its edge', JSON.stringify(named.edges));
  }

  const route = named.components.find((component) => component.id === 'split');
  if (route && route.namedOutputs.includes('_unmatched')) {
    ok('namedOutputs arrives camelCased');
  } else {
    fail('namedOutputs arrives camelCased', JSON.stringify(route));
  }

  if (named.document.includes('```mermaid') && named.document.includes('flowchart LR')) {
    ok('the document carries a Mermaid diagram');
  } else {
    fail('the document carries a Mermaid diagram', named.document.slice(0, 200));
  }

  // A TOML config goes through the other parser and has to arrive the same.
  const wildcards = topology(
    await readFile(path.join(dir, 'wildcards.toml'), 'utf8'),
    'wildcards.toml',
  );
  if ('error' in wildcards) {
    fail('a TOML config reads', wildcards.error.message);
  } else if (wildcards.edges.filter((edge) => edge.to === 'out').length === 2) {
    ok('a wildcard expands to one edge per match, in TOML');
  } else {
    fail('a wildcard expands to one edge per match, in TOML', JSON.stringify(wildcards.edges));
  }

  const broken = topology('sinks: [oops', 'vector.yaml');
  if ('error' in broken) {
    ok('a config that does not parse answers rather than throwing');
  } else {
    fail('a config that does not parse answers rather than throwing', 'it returned a graph');
  }
}

async function checkTrapRecovery(): Promise<void> {
  const { VrlChecker } = (await import(
    pathToFileURL(path.join(ROOT, 'editors/vscode/out/checker.js')).href
  )) as typeof import('../editors/vscode/src/checker.js');

  const checker = VrlChecker.load(path.join(ROOT, 'editors/vscode'));
  const deep = `.a = ${'['.repeat(800)}${']'.repeat(800)}`;

  if (!checker.check('.a = 1').compiled) {
    fail('a fresh checker compiles', 'the trivial program did not compile');
    return;
  }

  // The trap is allowed to surface here: what matters is what comes after it.
  try {
    checker.check(deep);
  } catch {
    // Expected on the way in, and swallowed on the retry inside `call`.
  }

  try {
    if (checker.check('.a = 1').compiled) {
      ok('a trap does not end the session');
    } else {
      fail('a trap does not end the session', 'the checker answered but did not compile');
    }
  } catch (error) {
    fail('a trap does not end the session', `the module stayed dead: ${String(error)}`);
  }

  try {
    checker.stdlib();
    ok('the standard library survives a trap');
  } catch (error) {
    fail('the standard library survives a trap', String(error));
  }
}

async function main(): Promise<void> {
  console.log(`checking against vrl ${vrlVersion()}\n`);

  await checkVersionAgrees();
  await checkCorpusPrograms();
  await checkSampleTypedPrograms();
  checkSamplePolicy();
  await checkEmbeddedPrograms();
  checkVectorFunctions();
  checkBoundary();
  checkStdlib();
  await checkTopology();
  await checkTrapRecovery();

  console.log(`\n${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((error: unknown) => {
  console.error(String(error));
  process.exit(1);
});
