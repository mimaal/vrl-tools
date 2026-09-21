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

/** Mirrors `vector_topology::Analysis`, or the error shape it answers with. */
export type TopologyResult = Topology | { readonly error: { readonly message: string } };

export interface Topology {
  readonly document: string;
  readonly components: readonly {
    readonly id: string;
    readonly role: 'source' | 'transform' | 'sink';
    readonly type: string;
    readonly namedOutputs: readonly string[];
  }[];
  readonly edges: readonly {
    readonly from: string;
    readonly output: string | null;
    readonly to: string;
  }[];
  readonly findings: readonly {
    readonly severity: 'error' | 'warning';
    readonly message: string;
    readonly range: { readonly start: { readonly line: number } };
  }[];
  readonly layout: {
    readonly components: readonly {
      readonly component: number;
      readonly column: number;
      readonly row: number;
    }[];
    readonly routes: readonly {
      readonly from: number;
      readonly to: number;
      readonly via: readonly { readonly column: number; readonly row: number }[];
    }[];
  };
}

/** Mirrors `vrl_check_core::Run`. */
export interface RunResult {
  readonly compiled: boolean;
  readonly vrlVersion: string;
  readonly diagnostics: readonly CheckDiagnostic[];
  readonly event: unknown;
  readonly metadata: unknown;
  readonly output: unknown;
  readonly error: string | null;
  readonly aborted: boolean;
  readonly sampleError: string | null;
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
  /** Softened from an error because only the sample event produced it. */
  readonly relaxedBySample: boolean;
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
  check(source: string, sampleEventJson?: string, enrichmentTables?: string[]): string;
  run(source: string, eventJson: string, enrichmentTables?: string[]): string;
  topology(source: string, fileName: string): string;
  enrichment_tables(source: string, fileName: string): string[] | undefined;
  topology_files(filesJson: string, title: string): string;
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

/**
 * Compiles `source` with the real VRL compiler, against the enrichment
 * tables a Vector config would declare.
 */
export function check(
  source: string,
  sampleEventJson?: string,
  enrichmentTables: readonly string[] = [],
): Check {
  return JSON.parse(load().check(source, sampleEventJson, [...enrichmentTables])) as Check;
}

/** Compiles `source` against `eventJson` and runs it. */
export function run(
  source: string,
  eventJson: string,
  enrichmentTables: readonly string[] = [],
): RunResult {
  return JSON.parse(load().run(source, eventJson, [...enrichmentTables])) as RunResult;
}

/** The enrichment table names a Vector config declares, or undefined if it does not parse. */
export function enrichmentTables(source: string, fileName: string): string[] | undefined {
  return load().enrichment_tables(source, fileName);
}

/** Reads a pipeline split across files, each a complete config, as one topology. */
export function topologyFiles(
  files: readonly { name: string; source: string }[],
  title: string,
): Topology & { readonly files: readonly string[]; readonly unreadable: readonly unknown[] } {
  return JSON.parse(load().topology_files(JSON.stringify(files), title)) as Topology & {
    readonly files: readonly string[];
    readonly unreadable: readonly unknown[];
  };
}

/** Reads a Vector configuration and resolves its topology. */
export function topology(source: string, fileName: string): TopologyResult {
  return JSON.parse(load().topology(source, fileName)) as TopologyResult;
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
export function errorsIn(
  source: string,
  sampleEventJson?: string,
  enrichmentTables: readonly string[] = [],
): CheckDiagnostic[] {
  return check(source, sampleEventJson, enrichmentTables).diagnostics.filter(
    (diagnostic) => diagnostic.severity === 'error' || diagnostic.severity === 'bug',
  );
}
