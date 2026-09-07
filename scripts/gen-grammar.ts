/**
 * Generates editors/vscode/syntaxes/vrl.tmLanguage.json from the template.
 *
 * The stdlib function list is never hand-written. It comes from
 * `vrl::stdlib::all()` by way of the `vrl-stdlib` binary in vrl-check-core —
 * the same value the compiler resolves calls against, so the grammar cannot
 * drift from the language.
 *
 * This needs a Rust toolchain. That is a deliberate trade: from phase 3 on,
 * Rust is required to build the extension anyway, and the previous route (scrape
 * `identifier()` out of the published crate source) was a stand-in for exactly
 * this.
 */

import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const TEMPLATE = path.join(ROOT, 'editors/vscode/syntaxes/vrl.tmLanguage.template.json');
const OUTPUT = path.join(ROOT, 'editors/vscode/syntaxes/vrl.tmLanguage.json');
const PLACEHOLDER = '__VRL_STDLIB_FUNCTIONS__';

/** Functions we know must exist. If any is missing, the dump is wrong. */
const CANARIES = ['parse_json', 'parse_syslog', 'to_int', 'del', 'exists', 'now', 'push'];

/** Below this, assume the dump broke rather than that the stdlib shrank. */
const MIN_EXPECTED = 150;

interface StdlibDump {
  readonly vrlVersion: string;
  readonly functions: readonly { readonly name: string }[];
}

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

/**
 * `cargo`, wherever it happens to be. A shell opened before rustup was
 * installed has no ~/.cargo/bin on its PATH, and failing the build over that
 * would send people looking in the wrong place.
 */
function cargoCandidates(): string[] {
  const candidates = [process.env.CARGO, 'cargo', path.join(homedir(), '.cargo', 'bin', 'cargo')];
  return candidates.filter((c): c is string => typeof c === 'string' && c.length > 0);
}

async function dumpStdlib(): Promise<StdlibDump> {
  const args = ['run', '-q', '-p', 'vrl-check-core', '--bin', 'vrl-stdlib'];
  const failures: string[] = [];

  for (const cargo of cargoCandidates()) {
    let stdout: string;
    try {
      ({ stdout } = await execFileAsync(cargo, args, {
        cwd: ROOT,
        maxBuffer: 64 * 1024 * 1024,
      }));
    } catch (cause) {
      failures.push(`${cargo}: ${String(cause).split('\n')[0]}`);
      continue;
    }

    try {
      return JSON.parse(stdout) as StdlibDump;
    } catch (cause) {
      throw new GenerationError(
        `${cargo} ran but did not print JSON. The vrl-stdlib binary must write ` +
          `only the dump to stdout.\n  ${String(cause)}`,
      );
    }
  }

  throw new GenerationError(
    'Could not run the vrl-stdlib binary. The stdlib list is generated from ' +
      '`vrl::stdlib::all()` and must never be written by hand, so this is fatal.\n' +
      'Install Rust (https://rustup.rs) or set CARGO to its path.\n' +
      failures.map((f) => `  tried ${f}`).join('\n'),
  );
}

function namesFrom(dump: StdlibDump, pinnedVersion: string): string[] {
  if (dump.vrlVersion !== pinnedVersion) {
    throw new GenerationError(
      `The compiled crate reports vrl ${dump.vrlVersion} but package.json pins ` +
        `${pinnedVersion}. Bring VRL_VERSION, the workspace Cargo.toml and ` +
        'package.json back into agreement before generating anything.',
    );
  }

  const names = [...new Set(dump.functions.map((f) => f.name))].sort();

  if (names.length < MIN_EXPECTED) {
    throw new GenerationError(
      `Only ${names.length} stdlib functions came back, expected at least ` +
        `${MIN_EXPECTED}. Refusing to emit a grammar with a truncated list.`,
    );
  }

  const missing = CANARIES.filter((c) => !names.includes(c));
  if (missing.length > 0) {
    throw new GenerationError(
      `These stdlib functions are missing from the dump: ${missing.join(', ')}. ` +
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
  const names = namesFrom(await dumpStdlib(), version);

  const template = await readFile(TEMPLATE, 'utf8');
  if (!template.includes(PLACEHOLDER)) {
    throw new GenerationError(`The template no longer contains ${PLACEHOLDER}.`);
  }

  const grammar = JSON.parse(template.split(PLACEHOLDER).join(buildAlternation(names)));

  // Provenance, so nobody edits the generated file by hand and wonders why it reverts.
  grammar.information_for_contributors = [
    'DO NOT EDIT. Generated by scripts/gen-grammar.ts from',
    'vrl.tmLanguage.template.json plus vrl::stdlib::all().',
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
