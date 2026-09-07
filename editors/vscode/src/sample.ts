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

export interface Sample {
  readonly uri: vscode.Uri;
  readonly json: string;
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
      return { uri, json: open.getText() };
    }

    let stat: fs.Stats;
    try {
      stat = fs.statSync(uri.fsPath);
    } catch {
      return undefined;
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

/** Whether `uri` is a sample file rather than a program. */
export function isSample(uri: vscode.Uri): boolean {
  return uri.path.endsWith(SAMPLE_SUFFIX);
}

/** The program a sample describes: the inverse of `uriFor`. */
export function programFor(sample: vscode.Uri): vscode.Uri {
  return sample.with({ path: sample.path.slice(0, -SAMPLE_SUFFIX.length) });
}
