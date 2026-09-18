import * as fs from 'node:fs';

import * as vscode from 'vscode';

/**
 * Finds the sample event that belongs to a `.vrl` file.
 *
 * The convention is one JSON file sitting next to the program with
 * `.sample.json` appended: `parse.vrl` is described by
 * `parse.vrl.sample.json`. No setting, no picker, no state to remember —
 * the file is either there or it is not, and it lives in the repository next
 * to the program it describes, which is where a colleague will look for it.
 *
 * A sample changes what the compiler knows: with one, `.message` is a string
 * and `.mesage` does not exist. So it is read on every check, and reading it
 * has to be cheap and never stale — hence a cache keyed on modification time,
 * and a preference for the editor's own copy when the file is open and
 * unsaved.
 */
export const SAMPLE_SUFFIX = '.sample.json';

/**
 * The largest sample that gets read.
 *
 * A sample is one event, which is kilobytes. This bound is not about what a
 * sample should be, it is about what this code does with it: the file is read
 * whole, synchronously, on the same path as the diagnostics that run while
 * somebody types, and then handed across the wasm boundary as a string. A
 * production dump saved next to a program by accident would otherwise stall
 * the editor on every keystroke, with nothing on screen to say why.
 */
const MAX_SAMPLE_BYTES = 4 * 1024 * 1024;

export interface Sample {
  readonly uri: vscode.Uri;
  /** The event, absent when the file is there but cannot be used. */
  readonly json?: string;
  /** Why the file cannot be used, when it cannot. */
  readonly unusable?: string;
}

interface Cached {
  readonly mtimeMs: number;
  readonly json: string;
}

export class SampleStore implements vscode.Disposable {
  private readonly cache = new Map<string, Cached>();
  private readonly watcher: vscode.FileSystemWatcher;

  /**
   * @param onChanged Called with the `.vrl` document a sample belongs to,
   * whenever that sample appears, changes or is deleted, so its diagnostics
   * can be recomputed.
   */
  constructor(onChanged: (program: vscode.Uri) => void) {
    this.watcher = vscode.workspace.createFileSystemWatcher(`**/*${SAMPLE_SUFFIX}`);

    const changed = (sample: vscode.Uri): void => {
      this.cache.delete(sample.fsPath);
      onChanged(programFor(sample));
    };

    this.watcher.onDidCreate(changed);
    this.watcher.onDidChange(changed);
    this.watcher.onDidDelete(changed);
  }

  /** Where the sample for `program` would be, whether or not it exists. */
  uriFor(program: vscode.Uri): vscode.Uri {
    return program.with({ path: `${program.path}${SAMPLE_SUFFIX}` });
  }

  /**
   * The sample for `program`, or `undefined` when there is none.
   *
   * Synchronous on purpose: this is called on the same path as the diagnostics
   * that run while someone types, and an await there would mean checking the
   * document against last keystroke's sample.
   */
  read(program: vscode.Uri): Sample | undefined {
    const uri = this.uriFor(program);

    // An open editor is ahead of the disk. Reading it means diagnostics follow
    // the sample as it is edited, before it is saved.
    const open = vscode.workspace.textDocuments.find(
      (document) => document.uri.fsPath === uri.fsPath && !document.isClosed,
    );
    if (open) {
      return within(uri, open.getText());
    }

    let stat: fs.Stats;
    try {
      stat = fs.statSync(uri.fsPath);
    } catch {
      return undefined;
    }

    // Asked of the stat rather than of the contents: the point is not to pull
    // the file into memory at all.
    if (stat.size > MAX_SAMPLE_BYTES) {
      return { uri, unusable: tooLarge(stat.size) };
    }

    const cached = this.cache.get(uri.fsPath);
    if (cached && cached.mtimeMs === stat.mtimeMs) {
      return { uri, json: cached.json };
    }

    try {
      const json = fs.readFileSync(uri.fsPath, 'utf8');
      this.cache.set(uri.fsPath, { mtimeMs: stat.mtimeMs, json });
      return { uri, json };
    } catch {
      return undefined;
    }
  }

  dispose(): void {
    this.watcher.dispose();
    this.cache.clear();
  }
}

/**
 * The same bound applied to a sample the editor already holds.
 *
 * An open document is in memory whatever this says, but the cost being
 * avoided is not the read: it is crossing the wasm boundary with it, on
 * every keystroke.
 */
function within(uri: vscode.Uri, json: string): Sample {
  const bytes = Buffer.byteLength(json, 'utf8');
  return bytes > MAX_SAMPLE_BYTES ? { uri, unusable: tooLarge(bytes) } : { uri, json };
}

function tooLarge(bytes: number): string {
  const mb = (bytes / 1024 / 1024).toFixed(1);
  const limit = MAX_SAMPLE_BYTES / 1024 / 1024;
  return `it is ${mb} MB and the limit is ${limit} MB — a sample is one event, not a capture`;
}

/** Whether `uri` is a sample file rather than a program. */
export function isSample(uri: vscode.Uri): boolean {
  return uri.path.endsWith(SAMPLE_SUFFIX);
}

/** The program a sample describes: the inverse of `uriFor`. */
export function programFor(sample: vscode.Uri): vscode.Uri {
  return sample.with({ path: sample.path.slice(0, -SAMPLE_SUFFIX.length) });
}
