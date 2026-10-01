import { promises as fs } from 'node:fs';

import * as vscode from 'vscode';

import type { PipelineOptions, Topology, TopologyTable, VrlChecker } from './checker';
import { sameFile } from './pipeline';
import type { Pipeline } from './pipeline';
import { candidates, countRows } from './tablefiles';

/**
 * Reading a pipeline: its files, handed to the wasm module, together with
 * what the module cannot get for itself.
 *
 * The module opens no files. A `remap` whose program is `file = "…"` names a
 * path, and what that program looks up is part of the answer to "who reads
 * this table" — so the files are found here, read, and handed over. Which
 * ones is only known once the config has been read, hence the two passes: the
 * first says which programs it wanted, the second has them. The paths are
 * remembered per pipeline, so every read after the first is one pass.
 *
 * The graph panel and the sidebar both read through this, which is what keeps
 * them saying the same thing about the same pipeline.
 */

/** The program paths each pipeline asked for last time, by pipeline key. */
const WANTED = new Map<string, readonly { path: string; file: number }[]>();

export async function readPipeline(
  checker: VrlChecker,
  pipeline: Pipeline,
  options: Pick<PipelineOptions, 'focus' | 'showTables'> = {},
): Promise<Topology> {
  const files = pipeline.files.map((file) => ({
    name: file.name,
    source: file.source,
    standalone: file.standalone,
  }));
  const read = async (wanted: readonly { path: string; file: number }[]) =>
    checker.pipeline(files, pipeline.title, {
      ...options,
      programs: await programs(pipeline, wanted),
    });

  let analysis = await read(WANTED.get(pipeline.key) ?? []);
  const wanted = analysis.programs.map(({ path, file }) => ({ path, file }));
  if (analysis.programs.some((program) => !program.read)) {
    analysis = await read(wanted);
  }
  WANTED.set(pipeline.key, wanted);
  return analysis;
}

/** The programs that can be found, as the module wants them. */
async function programs(
  pipeline: Pipeline,
  wanted: readonly { path: string; file: number }[],
): Promise<{ path: string; source: string }[]> {
  const found: { path: string; source: string }[] = [];
  for (const { path, file } of wanted) {
    if (found.some((program) => program.path === path)) {
      continue;
    }
    const uri = await locate(pipeline, path, file);
    const source = uri && (await textOf(uri));
    if (source !== undefined) {
      found.push({ path, source });
    }
  }
  return found;
}

/**
 * The workspace file a path written in a config means, if there is one. See
 * `candidates` for the guess; this is the part that looks.
 */
export async function locate(
  pipeline: Pipeline,
  written: string,
  file: number,
): Promise<vscode.Uri | undefined> {
  const config = pipeline.files[file];
  const folder = config && vscode.workspace.getWorkspaceFolder(config.uri);
  if (!config || !folder) {
    return undefined;
  }
  for (const candidate of candidates(written, config.name)) {
    const uri = vscode.Uri.joinPath(folder.uri, candidate);
    if (await isFile(uri)) {
      return uri;
    }
  }
  return undefined;
}

async function isFile(uri: vscode.Uri): Promise<boolean> {
  try {
    return (await fs.stat(uri.fsPath)).isFile();
  } catch {
    return false;
  }
}

/** The editor's copy when the file is open, unsaved edits included; the disk's otherwise. */
async function textOf(uri: vscode.Uri): Promise<string | undefined> {
  const open = vscode.workspace.textDocuments.find((d) => sameFile(d.uri, uri));
  if (open) {
    return open.getText();
  }
  try {
    return await fs.readFile(uri.fsPath, 'utf8');
  } catch {
    return undefined;
  }
}

/** How many rows a table's CSV holds, and which file was counted. */
export interface Counted {
  readonly rows: number;
  /** Relative to the workspace folder: the file the count is of. */
  readonly counted: string;
}

/** Row counts by file, kept until the file changes. */
const ROWS = new Map<string, { stamp: string; rows: number }>();

/**
 * The rows of a `file` table, when its CSV is in the workspace.
 *
 * `undefined` otherwise, which is most of the time for a config written for
 * another machine; the path is still shown, as written. Kept per file and
 * re-counted only when its size or its modification time moves, since a
 * pipeline's tables are opened a file's worth at a time and many share a CSV.
 */
export async function rowsOf(pipeline: Pipeline, table: TopologyTable): Promise<Counted | undefined> {
  if (table.type !== 'file' || !table.path) {
    return undefined;
  }
  const uri = await locate(pipeline, table.path, table.file);
  if (!uri) {
    return undefined;
  }

  try {
    const stat = await fs.stat(uri.fsPath);
    const stamp = `${stat.size}:${stat.mtimeMs}`;
    const known = ROWS.get(uri.fsPath);
    const counted = vscode.workspace.asRelativePath(uri, false);
    if (known && known.stamp === stamp) {
      return { rows: known.rows, counted };
    }
    const rows = countRows(await fs.readFile(uri.fsPath, 'utf8'), table.csvHeaders);
    ROWS.set(uri.fsPath, { stamp, rows });
    return { rows, counted };
  } catch {
    return undefined;
  }
}
