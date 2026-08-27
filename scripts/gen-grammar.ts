/**
 * Generates editors/vscode/syntaxes/vrl.tmLanguage.json from the template.
 *
 * The stdlib function list is never hand-written. The authoritative source is
 * `vrl::stdlib::all()`, but that needs a Rust toolchain, which is not a
 * prerequisite for phase 1. Until Rust is in the picture, this reads the same
 * information out of the published crate source: every stdlib function is a
 * type implementing `Function`, and each one declares its VRL name in
 * `fn identifier(&self) -> &'static str`.
 *
 * Swap this for a Rust binary over `vrl::stdlib::all()` once Rust is available;
 * the rest of this script does not change.
 */

import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as tar from 'tar';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const CACHE = path.join(ROOT, '.cache');
const TEMPLATE = path.join(ROOT, 'editors/vscode/syntaxes/vrl.tmLanguage.template.json');
const OUTPUT = path.join(ROOT, 'editors/vscode/syntaxes/vrl.tmLanguage.json');
const PLACEHOLDER = '__VRL_STDLIB_FUNCTIONS__';

/** Functions we know must exist. If any is missing, the extraction is wrong. */
const CANARIES = ['parse_json', 'parse_syslog', 'to_int', 'del', 'exists', 'now', 'push'];

/** Below this, assume the extraction broke rather than that the stdlib shrank. */
const MIN_EXPECTED = 150;

class GenerationError extends Error {}

async function readPinnedVersion(): Promise<string> {
  const pkg = JSON.parse(await readFile(path.join(ROOT, 'package.json'), 'utf8'));
  const version = pkg?.vrl?.crateVersion;
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+$/.test(version)) {
    throw new GenerationError(
      'package.json has no valid "vrl.crateVersion". That field is the single ' +
        'source of truth for the pinned crate version.',
    );
  }
  return version;
}

async function downloadCrate(version: string): Promise<string> {
  const file = path.join(CACHE, `vrl-${version}.crate`);
  if (existsSync(file)) {
    return file;
  }

  const url = `https://static.crates.io/crates/vrl/vrl-${version}.crate`;
  process.stderr.write(`Downloading ${url}\n`);

  let response: Response;
  try {
    response = await fetch(url, { headers: { 'user-agent': 'vrl-tools-grammar-generator' } });
  } catch (cause) {
    throw new GenerationError(
      `Could not reach crates.io to fetch vrl ${version}. The stdlib function ` +
        `list cannot be generated offline, and must not be written by hand.\n  ${String(cause)}`,
    );
  }

  if (!response.ok) {
    throw new GenerationError(
      `crates.io returned HTTP ${response.status} for vrl ${version}. ` +
        'Check that the pinned version exists; note that Vector release numbers ' +
        'are not vrl crate versions.',
    );
  }

  const bytes = Buffer.from(await response.arrayBuffer());
  await mkdir(CACHE, { recursive: true });
  await writeFile(file, bytes);
  return file;
}

async function extractStdlibSources(crateFile: string, version: string): Promise<string> {
  const dir = path.join(CACHE, `vrl-${version}`);
  if (existsSync(path.join(dir, 'src/stdlib'))) {
    return path.join(dir, 'src/stdlib');
  }

  await mkdir(dir, { recursive: true });
  await tar.x({
    file: crateFile,
    cwd: dir,
    strip: 1,
    filter: (entry) => entry.includes('/src/stdlib/'),
  });

  const stdlib = path.join(dir, 'src/stdlib');
  if (!existsSync(stdlib)) {
    throw new GenerationError(
      `The vrl ${version} crate has no src/stdlib directory. The crate layout ` +
        'changed; the extraction needs updating.',
    );
  }
  return stdlib;
}

async function extractIdentifiers(stdlibDir: string): Promise<string[]> {
  const files = (await readdir(stdlibDir)).filter((f) => f.endsWith('.rs'));
  const identifier = /fn\s+identifier\s*\(\s*&self\s*\)\s*->\s*&'static\s+str\s*\{\s*"([a-z0-9_]+)"/g;

  const found = new Set<string>();
  for (const file of files) {
    const source = await readFile(path.join(stdlibDir, file), 'utf8');
    for (const match of source.matchAll(identifier)) {
      found.add(match[1]);
    }
  }

  const names = [...found].sort();

  if (names.length < MIN_EXPECTED) {
    throw new GenerationError(
      `Only ${names.length} stdlib functions were extracted, expected at least ` +
        `${MIN_EXPECTED}. The identifier() pattern probably no longer matches ` +
        'the crate source. Refusing to emit a grammar with a truncated list.',
    );
  }

  const missing = CANARIES.filter((c) => !found.has(c));
  if (missing.length > 0) {
    throw new GenerationError(
      `These stdlib functions are missing from the extraction: ${missing.join(', ')}. ` +
        'Refusing to emit a grammar built on incomplete data.',
    );
  }

  return names;
}

/**
 * Longest-first so that, e.g., `to_int` can never win over `to_int_or_default`
 * inside the alternation. The word boundaries in the template make this
 * belt-and-braces, but the ordering costs nothing.
 */
function buildAlternation(names: string[]): string {
  return [...names].sort((a, b) => b.length - a.length || a.localeCompare(b)).join('|');
}

async function main(): Promise<void> {
  const version = await readPinnedVersion();
  const crateFile = await downloadCrate(version);
  const stdlibDir = await extractStdlibSources(crateFile, version);
  const names = await extractIdentifiers(stdlibDir);

  const template = await readFile(TEMPLATE, 'utf8');
  if (!template.includes(PLACEHOLDER)) {
    throw new GenerationError(`The template no longer contains ${PLACEHOLDER}.`);
  }

  const grammar = JSON.parse(template.split(PLACEHOLDER).join(buildAlternation(names)));

  // Provenance, so nobody edits the generated file by hand and wonders why it reverts.
  grammar.information_for_contributors = [
    'DO NOT EDIT. Generated by scripts/gen-grammar.ts from',
    'vrl.tmLanguage.template.json plus the stdlib of the vrl crate.',
    `Pinned vrl crate version: ${version}`,
    `Stdlib functions: ${names.length}`,
    'Regenerate with: npm run gen:grammar',
  ];
  grammar.version = `vrl-${version}`;

  await writeFile(OUTPUT, JSON.stringify(grammar, null, 2) + '\n');

  const digest = createHash('sha256').update(names.join(',')).digest('hex').slice(0, 12);
  process.stderr.write(
    `Wrote ${path.relative(ROOT, OUTPUT)}\n` +
      `  vrl crate     ${version}\n` +
      `  stdlib funcs  ${names.length} (sha256 ${digest})\n`,
  );
}

main().catch((error: unknown) => {
  if (error instanceof GenerationError) {
    process.stderr.write(`\ngen-grammar failed: ${error.message}\n\n`);
  } else {
    process.stderr.write(`\ngen-grammar crashed: ${String(error)}\n\n`);
  }
  process.exit(1);
});
