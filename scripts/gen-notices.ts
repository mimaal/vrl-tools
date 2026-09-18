/**
 * Generates editors/vscode/THIRD-PARTY-NOTICES.md from the dependency graph
 * that actually ships.
 *
 * `cargo about` walks `vrl-check-wasm` for `wasm32-unknown-unknown` and fails
 * on any licence not accepted in `about.toml`, which is the point: the file is
 * a claim about what is inside the `.vsix`, and the extension cannot lawfully
 * be distributed without it.
 *
 * The only thing this script adds to that is line endings, and it is not a
 * detail. Licence texts are copied verbatim out of the crates, and some of
 * them — `generic-array`'s, for one — are stored with CRLF. Written straight
 * out, the file has different bytes depending on the host that generated it,
 * which is how CI came to reject a file whose diff showed two identical
 * halves. Normalising to LF here makes the output a function of the
 * dependency graph alone, so the check that it matches what is committed says
 * something true on every machine.
 */

import { execFile } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const TEMPLATE = path.join(ROOT, 'about.hbs');
const MANIFEST = path.join(ROOT, 'crates/vrl-check-wasm/Cargo.toml');
const OUTPUT = path.join(ROOT, 'editors/vscode/THIRD-PARTY-NOTICES.md');

/** Below this, assume the walk broke rather than that the tree shrank. */
const MIN_EXPECTED_LICENCES = 8;

async function main(): Promise<void> {
  let stdout: string;
  try {
    ({ stdout } = await execFileAsync(
      'cargo',
      ['about', 'generate', TEMPLATE, '--manifest-path', MANIFEST],
      { cwd: ROOT, maxBuffer: 64 * 1024 * 1024 },
    ));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(
      `cargo about failed. It is a prerequisite of the build:\n` +
        `  cargo install cargo-about --locked --features cli\n\n${detail}`,
    );
  }

  const notices = stdout.replace(/\r\n/g, '\n');

  const licences = (notices.match(/^## /gm) ?? []).length;
  if (licences < MIN_EXPECTED_LICENCES) {
    throw new Error(`only ${licences} licence sections were produced; the walk looks truncated`);
  }
  if (!notices.includes('Mozilla Public License 2.0')) {
    throw new Error('the MPL-2.0 section is missing, and the vrl crate is MPL-2.0');
  }

  await writeFile(OUTPUT, notices, 'utf8');
  console.log(`wrote ${path.relative(ROOT, OUTPUT)}: ${licences} licence sections`);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
