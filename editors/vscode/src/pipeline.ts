import { promises as fs } from 'node:fs';

import * as vscode from 'vscode';

/**
 * Which files make up a Vector pipeline.
 *
 * Vector reads its config from whatever it is given: one file, or several,
 * with `--config` and a glob over a directory of them, each of them a
 * complete config with its own `sources`, `transforms` and `sinks`. The
 * pipeline is all of them together — an input in one file names a source in
 * another — so reading the file that happens to be open reports every such
 * input as naming nothing.
 *
 * The files are what `vrl-tools.vectorConfig` says, the same patterns Vector
 * is started with. Left empty, they are every YAML or TOML file in the
 * workspace folder that declares a top-level `sources`, `transforms`, `sinks`
 * or `enrichment_tables`: a guess at which files are Vector's, which the
 * setting exists to replace when the guess is wrong. The files' contents are
 * read by the wasm module; this only decides which files to hand it.
 */
export const PIPELINE_SETTING = 'vectorConfig';

/** Every file a Vector config can be. */
export const CONFIG_GLOB = '**/*.{yaml,yml,toml}';

/** Where no Vector config lives, and where a scan would spend its time. */
export const EXCLUDE_GLOB = '**/{node_modules,.git,target}/**';

/** A cap on the files looked at when guessing, for very large workspaces. */
const MAX_CANDIDATES = 2000;

/**
 * A top-level section only a Vector config has, in either format: `sources:`
 * at the start of a YAML line, `[sources.x]` or `[sources]` in TOML.
 */
const VECTOR_SECTION =
  /^(?:(?:sources|transforms|sinks|enrichment_tables)\s*:|\[\s*(?:sources|transforms|sinks|enrichment_tables)\s*[.\]])/m;

export interface PipelineFile {
  readonly uri: vscode.Uri;
  /** Relative to the workspace folder, with forward slashes. */
  readonly name: string;
  readonly source: string;
}

export interface Pipeline {
  /** What the pipeline is called in a title: the patterns, or the folder. */
  readonly title: string;
  readonly files: readonly PipelineFile[];
}

/** The patterns configured for a workspace folder, if any. */
export function configuredPatterns(folder: vscode.WorkspaceFolder): readonly string[] {
  return vscode.workspace
    .getConfiguration('vrl-tools', folder)
    .get<string[]>(PIPELINE_SETTING, [])
    .filter((pattern) => pattern.trim() !== '');
}

/**
 * The pipeline `document` belongs to. Just `document`, alone, when it is not
 * in a workspace folder or not among the files the setting names: a config
 * opened on its own is graphed on its own.
 */
export async function pipelineOf(document: vscode.TextDocument): Promise<Pipeline> {
  const alone: Pipeline = {
    title: nameOf(document.uri),
    files: [{ uri: document.uri, name: nameOf(document.uri), source: document.getText() }],
  };

  const folder = vscode.workspace.getWorkspaceFolder(document.uri);
  if (!folder || document.uri.scheme !== 'file') {
    return alone;
  }

  const patterns = configuredPatterns(folder);
  if (patterns.length > 0) {
    const uris = await matching(folder, patterns);
    return uris.some((uri) => sameFile(uri, document.uri))
      ? { title: patterns.join(', '), files: await read(folder, uris) }
      : alone;
  }

  // Guessing, the open config is always part of it, even before its first
  // section is written.
  const uris = await guessed(folder);
  if (!uris.some((uri) => sameFile(uri, document.uri))) {
    uris.push(document.uri);
  }
  return { title: folder.name, files: await read(folder, unique(uris)) };
}

/** Whether two URIs are one file. Windows drive letters differ in case between APIs. */
export function sameFile(a: vscode.Uri, b: vscode.Uri): boolean {
  return process.platform === 'win32'
    ? a.toString().toLowerCase() === b.toString().toLowerCase()
    : a.toString() === b.toString();
}

/** The files the setting's patterns match, in a stable order. */
export async function matching(
  folder: vscode.WorkspaceFolder,
  patterns: readonly string[],
): Promise<vscode.Uri[]> {
  const found = await Promise.all(
    patterns.map((pattern) =>
      vscode.workspace.findFiles(new vscode.RelativePattern(folder, pattern), EXCLUDE_GLOB),
    ),
  );
  return unique(found.flat());
}

/** Every YAML or TOML file in the folder that declares a Vector section. */
async function guessed(folder: vscode.WorkspaceFolder): Promise<vscode.Uri[]> {
  const candidates = await vscode.workspace.findFiles(
    new vscode.RelativePattern(folder, CONFIG_GLOB),
    EXCLUDE_GLOB,
    MAX_CANDIDATES,
  );
  const texts = await Promise.all(candidates.map((uri) => textOf(uri)));
  return unique(candidates.filter((_, index) => VECTOR_SECTION.test(texts[index] ?? '')));
}

async function read(folder: vscode.WorkspaceFolder, uris: readonly vscode.Uri[]): Promise<PipelineFile[]> {
  const files: PipelineFile[] = [];
  for (const uri of uris) {
    const source = await textOf(uri);
    if (source !== undefined) {
      files.push({ uri, name: relative(folder, uri), source });
    }
  }
  return files;
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

function unique(uris: readonly vscode.Uri[]): vscode.Uri[] {
  const seen = new Map<string, vscode.Uri>();
  for (const uri of uris) {
    seen.set(uri.toString(), uri);
  }
  return [...seen.values()].sort((a, b) => a.path.localeCompare(b.path));
}

/**
 * A file's path relative to its workspace folder. VS Code's own, since on
 * Windows the drive letter's case differs between the APIs that hand out
 * URIs, and a prefix comparison quietly falls back to the bare file name.
 */
function relative(_folder: vscode.WorkspaceFolder, uri: vscode.Uri): string {
  return vscode.workspace.asRelativePath(uri, false);
}

function nameOf(uri: vscode.Uri): string {
  return uri.path.split('/').pop() ?? uri.path;
}
