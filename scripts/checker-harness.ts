/**
 * Loads the compiled `vrl-check-wasm` module the way the extension does, for
 * the test scripts.
 *
 * The module is a build artefact, not a source file: it exists only after
 * `npm run build:wasm`. Rather than silently skipping the checks that need it —
 * which would let a broken snippet through on a machine where the wasm was
 * never built — loading it fails loudly and says which command produces it.
 */

import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as path from 'node:path';

import { ROOT } from './grammar-harness.js';

/** Mirrors `vrl_check_core::Check`; see editors/vscode/src/checker.ts. */
export interface Check {
  readonly compiled: boolean;
  readonly vrlVersion: string;
  readonly diagnostics: readonly CheckDiagnostic[];
}

export interface CheckDiagnostic {
  readonly severity: 'bug' | 'error' | 'warning' | 'note';
  readonly code: number;
  readonly message: string;
  readonly range: {
    readonly start: { readonly line: number; readonly character: number };
    readonly end: { readonly line: number; readonly character: number };
  };
  readonly labels: readonly { readonly message: string; readonly primary: boolean }[];
  readonly notes: readonly string[];
  readonly documentationUrl: string | null;
}

/** Mirrors `vrl_check_core::Stdlib`; see editors/vscode/src/checker.ts. */
export interface Stdlib {
  readonly vrlVersion: string;
  readonly functions: readonly StdlibFunction[];
}

export interface StdlibFunction {
  readonly name: string;
  readonly summary: string;
  readonly usage: string;
  readonly parameters: readonly {
    readonly keyword: string;
    readonly kind: string;
    readonly required: boolean;
  }[];
  readonly closure: { readonly variables: readonly string[] } | null;
  readonly returns: string | null;
  readonly fallible: boolean | null;
  readonly examples: readonly { readonly title: string; readonly source: string }[];
}

interface WasmModule {
  check(source: string, sampleEventJson?: string): string;
  stdlib(): string;
  vrl_version(): string;
}

const ENTRY = path.join(ROOT, 'editors/vscode/wasm/vrl_check_wasm.js');

/**
 * `--target nodejs` output is CommonJS, and these scripts are ES modules, so
 * the module is pulled in through `createRequire` rather than `import`. The
 * extension itself compiles to CommonJS and uses a plain `require`; both reach
 * the same file.
 */
const require = createRequire(import.meta.url);

let wasm: WasmModule | undefined;

export class CheckerUnavailableError extends Error {}

function load(): WasmModule {
  if (wasm) {
    return wasm;
  }
  if (!existsSync(ENTRY)) {
    throw new CheckerUnavailableError(
      `The VRL compiler module is missing at ${path.relative(ROOT, ENTRY)}.\n` +
        'Build it with: npm run build:wasm',
    );
  }

  wasm = require(ENTRY) as WasmModule;
  return wasm;
}

/** Compiles `source` with the real VRL compiler. */
export function check(source: string): Check {
  return JSON.parse(load().check(source)) as Check;
}

/** The pinned `vrl` crate version the module was built against. */
export function vrlVersion(): string {
  return load().vrl_version();
}

/**
 * The standard library the module describes, cached: building it compiles a
 * probe call per function.
 */
export function stdlib(): Stdlib {
  cachedStdlib ??= JSON.parse(load().stdlib()) as Stdlib;
  return cachedStdlib;
}

let cachedStdlib: Stdlib | undefined;

/** Only the diagnostics that stop a program from compiling. */
export function errorsIn(source: string): CheckDiagnostic[] {
  return check(source).diagnostics.filter(
    (diagnostic) => diagnostic.severity === 'error' || diagnostic.severity === 'bug',
  );
}
