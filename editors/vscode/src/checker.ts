import * as path from 'node:path';

/**
 * The answer `vrl-check-core` produces, mirrored on this side of the wasm
 * boundary. Ranges are already zero-based lines with UTF-16 columns, which is
 * what `vscode.Range` means by a position, so nothing here recomputes offsets.
 */
export interface Check {
  readonly compiled: boolean;
  readonly vrlVersion: string;
  readonly diagnostics: readonly VrlDiagnostic[];
}

export interface VrlDiagnostic {
  readonly severity: 'bug' | 'error' | 'warning' | 'note';
  readonly code: number;
  readonly message: string;
  readonly range: VrlRange;
  readonly labels: readonly VrlLabel[];
  readonly notes: readonly string[];
  readonly documentationUrl: string | null;
}

export interface VrlLabel {
  readonly message: string;
  readonly primary: boolean;
  readonly range: VrlRange;
}

export interface VrlRange {
  readonly start: VrlPosition;
  readonly end: VrlPosition;
}

export interface VrlPosition {
  readonly line: number;
  readonly character: number;
}

interface WasmModule {
  check(source: string, sampleEventJson?: string): string;
  vrl_version(): string;
}

/**
 * The real VRL compiler, compiled to WebAssembly.
 *
 * One module is loaded per session and reused; instantiating wasm is the
 * expensive part, compiling a program with it is not.
 */
export class VrlChecker {
  private constructor(
    private readonly wasm: WasmModule,
    readonly vrlVersion: string,
  ) {}

  /**
   * Loads the module that `npm run build:wasm` produced.
   *
   * Built with `wasm-pack --target nodejs`, so this is a plain CommonJS module
   * that reads the `.wasm` next to it. That rules out vscode.dev for now, which
   * would need a `--target web` build and an async init.
   */
  static load(extensionPath: string): VrlChecker {
    const entry = path.join(extensionPath, 'wasm', 'vrl_check_wasm.js');

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const wasm = require(entry) as WasmModule;
    return new VrlChecker(wasm, wasm.vrl_version());
  }

  check(source: string, sampleEventJson?: string): Check {
    return JSON.parse(this.wasm.check(source, sampleEventJson)) as Check;
  }
}
