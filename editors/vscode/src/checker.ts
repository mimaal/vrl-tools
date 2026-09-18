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
  /** `true` when a sample event was supplied and used to type the program. */
  readonly typedWithSample: boolean;
  /** Why the sample was ignored, when one was supplied and unusable. */
  readonly sampleError: string | null;
}

/** What happened when a program ran against a sample event. */
export interface Run {
  readonly compiled: boolean;
  readonly vrlVersion: string;
  readonly diagnostics: readonly VrlDiagnostic[];
  /** The event after the program, which is what Vector would emit. */
  readonly event: unknown;
  /** The event's metadata after the program. */
  readonly metadata: unknown;
  /** The value the program itself returned. */
  readonly output: unknown;
  /** The runtime error, if the program stopped. */
  readonly error: string | null;
  /** `true` when the program called `abort`: Vector would drop the event. */
  readonly aborted: boolean;
  readonly sampleError: string | null;
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

/**
 * The standard library, as `vrl-check-core` describes it. Every field here
 * comes from the compiler: the parameters from `vrl::stdlib::all()`, the
 * return type and fallibility from compiling a probe call.
 */
export interface Stdlib {
  readonly vrlVersion: string;
  readonly functions: readonly StdlibFunction[];
}

export interface StdlibFunction {
  readonly name: string;
  readonly summary: string;
  readonly usage: string;
  readonly parameters: readonly StdlibParameter[];
  /** Present when the function takes a closure, e.g. `for_each`. */
  readonly closure: { readonly variables: readonly string[] } | null;
  /** What a call returns, or null when the probe did not compile. */
  readonly returns: string | null;
  /** Whether a call can fail, or null when the probe did not compile. */
  readonly fallible: boolean | null;
  readonly examples: readonly StdlibExample[];
}

export interface StdlibParameter {
  readonly keyword: string;
  readonly kind: string;
  readonly required: boolean;
}

export interface StdlibExample {
  readonly title: string;
  readonly source: string;
  /** `Ok` for an example that returns a value, `Err` for one that fails. */
  readonly result: { readonly Ok: string } | { readonly Err: string };
}

interface WasmModule {
  check(source: string, sampleEventJson?: string): string;
  run(source: string, eventJson: string): string;
  stdlib(): string;
  vrl_version(): string;
  vector_release(): string;
}

/**
 * The real VRL compiler, compiled to WebAssembly.
 *
 * One module is loaded per session and reused; instantiating wasm is the
 * expensive part, compiling a program with it is not.
 */
export class VrlChecker {
  private cachedStdlib?: Stdlib;

  private constructor(
    private wasm: WasmModule,
    private readonly entry: string,
    readonly vrlVersion: string,
    /** The Vector release that ships exactly `vrlVersion`. */
    readonly vectorRelease: string,
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
    const wasm = instantiate(entry);

    // Read once and kept: they are the only two answers that must survive the
    // module being replaced under `call`.
    return new VrlChecker(wasm, entry, wasm.vrl_version(), wasm.vector_release());
  }

  /**
   * Calls into the module, replacing it if the call traps.
   *
   * A wasm trap is not an exception the module recovers from: the instance is
   * finished, and every later call into it — `check`, `stdlib`, even
   * `vrl_version` — throws the same `memory access out of bounds`. One module
   * is loaded per session, so without this a single trap would end
   * diagnostics, hover and completion for as long as the window stays open,
   * and the only sign of it would be a line in an output channel nobody has
   * open.
   *
   * It takes very little to get there. Around 800 nested brackets overflows
   * the stack inside the parser, which is not something a person types but is
   * well within what a generated file or a paste can hold.
   *
   * Instantiating is cheap next to losing the feature: `require` is dropped
   * from the cache so the wrapper re-reads the bytes and builds a fresh
   * `WebAssembly.Instance` with its own memory. The retry is single: if the
   * new instance trips on the same input, the caller gets the error and
   * decides, which for the diagnostics runner means leaving the file
   * unchecked rather than looping.
   */
  private call(into: (wasm: WasmModule) => string): string {
    try {
      return into(this.wasm);
    } catch {
      this.wasm = instantiate(this.entry);
      this.cachedStdlib = undefined;
      return into(this.wasm);
    }
  }

  check(source: string, sampleEventJson?: string): Check {
    return JSON.parse(this.call((wasm) => wasm.check(source, sampleEventJson))) as Check;
  }

  /**
   * Compiles `source` against `eventJson` and runs it.
   *
   * This executes the user's program. It runs in the same wasm sandbox as the
   * checker, with no filesystem and no network to reach.
   */
  run(source: string, eventJson: string): Run {
    return JSON.parse(this.call((wasm) => wasm.run(source, eventJson))) as Run;
  }

  /**
   * The standard library this module compiles against.
   *
   * Building it walks every function and compiles a probe call for each, so it
   * is asked for once and kept.
   */
  stdlib(): Stdlib {
    this.cachedStdlib ??= JSON.parse(this.call((wasm) => wasm.stdlib())) as Stdlib;
    return this.cachedStdlib;
  }
}

/**
 * Loads the module, bypassing the `require` cache.
 *
 * `wasm-pack --target nodejs` emits a CommonJS wrapper that reads the `.wasm`
 * and builds its `WebAssembly.Instance` at module scope, so dropping the cache
 * entry and requiring again is what produces a new instance with new memory.
 * Requiring without dropping it would hand back the same dead one.
 */
function instantiate(entry: string): WasmModule {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  delete require.cache[require.resolve(entry)];
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require(entry) as WasmModule;
}
