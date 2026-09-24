import { promises as fs } from 'node:fs';

import * as vscode from 'vscode';

import { group } from './grouping';

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

/**
 * Every file a Vector config can be: the three formats Vector reads, which are
 * also the three extensions `--config-dir` keeps.
 */
export const CONFIG_GLOB = '**/*.{yaml,yml,toml,json}';

/**
 * What is scanned when guessing which files are Vector's.
 *
 * JSON is deliberately left out of the *guess*, though not out of a pipeline:
 * a repository has hundreds of JSON files that are nothing to do with Vector —
 * lockfiles, tsconfigs, fixtures, source maps — and every one of them would be
 * opened and read to find out. A JSON config is picked up when it is open, or
 * when `vrl-tools.vectorConfig` names it, which is how anyone running Vector
 * on one starts the process anyway.
 */
const GUESS_GLOB = '**/*.{yaml,yml,toml}';

/** Where no Vector config lives, and where a scan would spend its time. */
export const EXCLUDE_GLOB = '**/{node_modules,.git,target}/**';

/** A cap on the files looked at when guessing, for very large workspaces. */
const MAX_CANDIDATES = 2000;

/**
 * A top-level section only a Vector config has, in any of the formats:
 * `sources:` at the start of a YAML line, `"sources":` in JSON, `[sources.x]`
 * or `[sources]` in TOML.
 */
const VECTOR_SECTION =
  /^(?:"?(?:sources|transforms|sinks|enrichment_tables)"?\s*:|\[\s*(?:sources|transforms|sinks|enrichment_tables)\s*[.\]])/m;

export interface PipelineFile {
  readonly uri: vscode.Uri;
  /** Relative to the workspace folder, with forward slashes. */
  readonly name: string;
  readonly source: string;
}

export interface Pipeline {
  /** What the pipeline is called in a title: the patterns, or the folder. */
  readonly title: string;
  /** Stable enough to remember a choice by, across a reload. */
  readonly key: string;
  readonly files: readonly PipelineFile[];
}

/**
 * The component names one file declares, which is how the files found in a
 * workspace are told apart into pipelines.
 *
 * Passed in rather than read here, so this file still knows nothing about the
 * wasm module — it decides which files to hand over, and nothing else.
 */
export type ComponentNames = (file: PipelineFile) => readonly string[];

/** The patterns configured for a workspace folder, if any. */
export function configuredPatterns(folder: vscode.WorkspaceFolder): readonly string[] {
  return vscode.workspace
    .getConfiguration('vrl-tools', folder)
    .get<string[]>(PIPELINE_SETTING, [])
    .filter((pattern) => pattern.trim() !== '');
}

/**
 * The pipeline `document` belongs to: the one of [`discover`]'s that holds it.
 *
 * Just `document`, alone, when it is in none of them — which is every config
 * whose first `sources:` is still being typed, and every file opened outside a
 * workspace folder.
 */
export async function pipelineOf(
  document: vscode.TextDocument,
  namesOf: ComponentNames,
): Promise<Pipeline> {
  const name = nameOf(document.uri);
  const alone: Pipeline = {
    title: name,
    key: name,
    files: [{ uri: document.uri, name, source: document.getText() }],
  };

  const folder = vscode.workspace.getWorkspaceFolder(document.uri);
  if (!folder || document.uri.scheme !== 'file') {
    return alone;
  }

  const pipelines = await discover(folder, namesOf);
  return (
    pipelines.find((pipeline) =>
      pipeline.files.some((file) => sameFile(file.uri, document.uri)),
    ) ?? alone
  );
}

/** Every pipeline in the workspace, across all its folders. */
export async function allPipelines(namesOf: ComponentNames): Promise<Pipeline[]> {
  const all: Pipeline[] = [];
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    all.push(...(await discover(folder, namesOf)));
  }
  return all;
}

/**
 * The pipeline to work from when nothing in the editor points at one: the
 * chosen one, or the first there is.
 *
 * This is what lets the graph be opened from the sidebar with a `.vrl` file —
 * or a README, or nothing at all — in front.
 */
export async function anyPipeline(
  namesOf: ComponentNames,
  preferred?: string,
): Promise<Pipeline | undefined> {
  const all = await allPipelines(namesOf);
  return all.find((pipeline) => pipeline.key === preferred) ?? all[0];
}

/** One file of [`anyPipeline`], for a caller that needs a document to open. */
export async function anyConfig(
  namesOf: ComponentNames,
  preferred?: string,
): Promise<vscode.Uri | undefined> {
  return (await anyPipeline(namesOf, preferred))?.files[0]?.uri;
}

/**
 * Every pipeline in a workspace folder, not just the first.
 *
 * A workspace holds more than one often enough to matter: `config/prod` beside
 * `config/staging`, a folder of examples, the test corpus of this very
 * repository. Treating all of them as one pipeline is what the guess used to
 * do, and what it produces is nonsense — forty-four "two components are called
 * `app_logs`" errors and a picture of a topology nobody runs.
 *
 * How they are told apart is `./grouping.ts`; this decides which files to
 * look at and reads them.
 */
export async function discover(
  folder: vscode.WorkspaceFolder,
  namesOf: ComponentNames,
): Promise<Pipeline[]> {
  // Configured patterns are not a guess: they are the files Vector is started
  // with, so they are one pipeline whatever the names do.
  const patterns = configuredPatterns(folder);
  if (patterns.length > 0) {
    const files = await read(folder, await matching(folder, patterns));
    return files.length > 0 ? [{ title: patterns.join(', '), key: patterns.join(','), files }] : [];
  }

  const files = await read(folder, await guessed(folder));
  const names = new Map<string, readonly string[]>();
  const namesCached: ComponentNames = (file) => {
    const known = names.get(file.name);
    if (known) {
      return known;
    }
    const read = namesOf(file);
    names.set(file.name, read);
    return read;
  };

  return group(files, folder.name, namesCached).map((found) => ({
    title: found.title,
    key: found.key,
    files: found.files,
  }));
}

/**
 * Which pipeline the sidebar and the graph are both looking at.
 *
 * One object shared by both, because a workspace with two pipelines in it and
 * a sidebar and a graph disagreeing about which one is being shown would be
 * worse than not letting you choose at all. Remembered per workspace, so the
 * choice survives a reload.
 */
export class PipelineChoice implements vscode.Disposable {
  private static readonly KEY = 'vrl-tools.pipeline';
  private readonly emitter = new vscode.EventEmitter<void>();

  readonly onDidChange = this.emitter.event;

  constructor(private readonly state: vscode.Memento) {}

  /** The chosen pipeline's key, or undefined for "whichever comes first". */
  get key(): string | undefined {
    return this.state.get<string>(PipelineChoice.KEY);
  }

  set(key: string | undefined): void {
    void this.state.update(PipelineChoice.KEY, key);
    this.emitter.fire();
  }

  dispose(): void {
    this.emitter.dispose();
  }
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
export async function guessed(folder: vscode.WorkspaceFolder): Promise<vscode.Uri[]> {
  const candidates = await vscode.workspace.findFiles(
    new vscode.RelativePattern(folder, GUESS_GLOB),
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
