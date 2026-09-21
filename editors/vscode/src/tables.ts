import { promises as fs } from 'node:fs';

import * as vscode from 'vscode';

import type { VrlChecker } from './checker';
import { CONFIG_GLOB, configuredPatterns, EXCLUDE_GLOB, matching, PIPELINE_SETTING } from './pipeline';

/**
 * How many candidate files the first scan reads. A workspace with more YAML
 * than this is not a Vector repository with a few configs in it, and reading
 * every Kubernetes manifest in a monorepo to find two table names is not a
 * trade worth making on activation.
 */
const MAX_CONFIGS = 2000;

/** The key a config declares its tables under. */
const MARKER = 'enrichment_tables';

/**
 * The enrichment tables the workspace's Vector configs declare.
 *
 * `find_enrichment_table_records` and `get_enrichment_table_record` check the
 * table they name at compile time, against the tables in the config, exactly
 * as `vector validate` does. A `.vrl` file does not say which config it runs
 * in — the `remap` transform points at the file, not the other way round, and
 * usually by a path on the machine Vector runs on — so the tables are those of
 * the workspace's Vector configs: the files `vrl-tools.vectorConfig` names
 * when it is set, the same ones the graph draws, and every config in the
 * workspace when it is not.
 *
 * The reading is the wasm module's, the same YAML and TOML parsers the graph
 * uses. The only thing done here without it is skipping files that never
 * mention `enrichment_tables`, which cannot declare any.
 */
export class EnrichmentTables implements vscode.Disposable {
  /** Tables per config file, keyed by URI. */
  private readonly byConfig = new Map<string, readonly string[]>();
  private readonly disposables: vscode.Disposable[] = [];
  private current: readonly string[] = [];
  /**
   * For a workspace folder with `vrl-tools.vectorConfig` set, the files it
   * names; a file outside them declares nothing Vector will load. Absent for
   * a folder without the setting, where every config counts.
   */
  private readonly allowed = new Map<string, Set<string>>();

  /**
   * @param onChanged Called when the set of declared tables changes, so every
   * open program can be checked against the new set.
   */
  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
    private readonly onChanged: () => void,
  ) {
    const watcher = vscode.workspace.createFileSystemWatcher(CONFIG_GLOB);
    this.disposables.push(
      watcher,
      // A new or deleted file can enter or leave the set a pattern names.
      watcher.onDidCreate(() => void this.scan()),
      watcher.onDidChange((uri) => void this.readFile(uri)),
      watcher.onDidDelete((uri) => this.forget(uri)),
      // The editor's copy wins over the disk's while a config is being edited,
      // so a table added and not yet saved is already usable.
      vscode.workspace.onDidChangeTextDocument((event) => this.readDocument(event.document)),
      vscode.workspace.onDidChangeConfiguration((event) => {
        if (event.affectsConfiguration(`vrl-tools.${PIPELINE_SETTING}`)) {
          this.byConfig.clear();
          void this.scan();
        }
      }),
    );

    void this.scan();
  }

  /** Every table name declared by some config, sorted. */
  names(): readonly string[] {
    return this.current;
  }

  private async scan(): Promise<void> {
    const files: vscode.Uri[] = [];
    this.allowed.clear();
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      const patterns = configuredPatterns(folder);
      if (patterns.length > 0) {
        const named = await matching(folder, patterns);
        this.allowed.set(folder.uri.toString(), new Set(named.map(key)));
        files.push(...named);
      } else {
        files.push(
          ...(await vscode.workspace.findFiles(
            new vscode.RelativePattern(folder, CONFIG_GLOB),
            EXCLUDE_GLOB,
            MAX_CONFIGS,
          )),
        );
      }
    }
    for (const stale of [...this.byConfig.keys()]) {
      if (!files.some((uri) => key(uri) === stale)) {
        this.byConfig.delete(stale);
      }
    }
    await Promise.all(files.map((uri) => this.readFile(uri, false)));
    this.publish();
  }

  /** Whether a file is one the tables can come from. */
  private counts(uri: vscode.Uri): boolean {
    const folder = vscode.workspace.getWorkspaceFolder(uri);
    const named = folder ? this.allowed.get(folder.uri.toString()) : undefined;
    return !named || named.has(key(uri));
  }

  private async readFile(uri: vscode.Uri, publish = true): Promise<void> {
    if (!this.counts(uri)) {
      return;
    }
    const open = vscode.workspace.textDocuments.find((d) => key(d.uri) === key(uri));
    if (open?.isDirty) {
      return;
    }

    let text: string;
    try {
      text = await fs.readFile(uri.fsPath, 'utf8');
    } catch {
      this.forget(uri);
      return;
    }

    this.read(uri, text, publish);
  }

  private readDocument(document: vscode.TextDocument): void {
    if (
      document.uri.scheme !== 'file' ||
      !/\.(ya?ml|toml)$/i.test(document.uri.path) ||
      !this.counts(document.uri)
    ) {
      return;
    }
    this.read(document.uri, document.getText(), true);
  }

  private read(uri: vscode.Uri, text: string, publish: boolean): void {
    const file = key(uri);

    if (!text.includes(MARKER)) {
      this.byConfig.delete(file);
    } else {
      let tables: string[] | undefined;
      try {
        tables = this.checker.enrichmentTables(text, uri.path.split('/').pop() ?? uri.path);
      } catch (error) {
        this.output.appendLine(`Reading ${uri.fsPath} for enrichment tables failed: ${String(error)}`);
        return;
      }

      // A config that does not parse right now is a config being typed.
      // Keeping what it declared last time beats flagging every lookup in
      // every program until the closing bracket goes in.
      if (tables === undefined) {
        return;
      }

      if (tables.length === 0) {
        this.byConfig.delete(file);
      } else {
        this.byConfig.set(file, tables);
      }
    }

    if (publish) {
      this.publish();
    }
  }

  private forget(uri: vscode.Uri): void {
    if (this.byConfig.delete(key(uri))) {
      this.publish();
    }
  }

  /** Recomputes the union and, when it moved, says so and re-checks. */
  private publish(): void {
    const names = [...new Set([...this.byConfig.values()].flat())].sort();
    if (names.join('\n') === this.current.join('\n')) {
      return;
    }

    this.current = names;
    const configs = [...this.byConfig.keys()].map((key) => vscode.Uri.parse(key).fsPath);
    this.output.appendLine(
      names.length === 0
        ? 'No Vector config in the workspace declares enrichment tables.'
        : `Enrichment tables declared in the workspace: ${names.join(', ')} (from ${configs.join(', ')}).`,
    );
    this.onChanged();
  }

  dispose(): void {
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
  }
}

/**
 * One key per file. Windows drive letters differ in case between the APIs
 * that hand out URIs, and the same file must not count twice.
 */
function key(uri: vscode.Uri): string {
  return process.platform === 'win32' ? uri.toString().toLowerCase() : uri.toString();
}
