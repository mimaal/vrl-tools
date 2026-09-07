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

import { check, errorsIn, run, stdlib, vrlVersion } from './checker-harness.js';
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
function describe(source: string, lineOffset = 0): string {
  return check(source)
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

    if (regions.length === 0) {
      fail(`${file} has VRL to compile`, 'the injection grammar found no embedded region');
      continue;
    }

    for (const region of regions) {
      const result = check(region.source);
      if (result.diagnostics.length > 0) {
        fail(
          `${file}: the VRL block at line ${region.line} compiles clean`,
          describe(region.source, region.line - 1),
        );
        continue;
      }
      ok(`${file}: the VRL block at line ${region.line} compiles clean`);
    }
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

async function main(): Promise<void> {
  console.log(`checking against vrl ${vrlVersion()}\n`);

  await checkVersionAgrees();
  await checkCorpusPrograms();
  await checkSampleTypedPrograms();
  checkSamplePolicy();
  await checkEmbeddedPrograms();
  checkBoundary();
  checkStdlib();

  console.log(`\n${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((error: unknown) => {
  console.error(String(error));
  process.exit(1);
});
