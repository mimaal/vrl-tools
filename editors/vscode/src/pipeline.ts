import { promises as fs } from 'node:fs';

import * as vscode from 'vscode';

import { Exclusions, gitignoreRules, settingRules } from './excludes';
import { declaresVectorSection, group, mainPipeline, pipelinesOnly } from './grouping';
import type { Shape } from './grouping';

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
 * The files are what `vrl-tools.vectorConfig` and `vrl-tools.vectorConfigDir`
 * say, the same arguments Vector is started with: `--config` patterns, whose
 * files are each loaded on their own, and `--config-dir` directories, whose
 * top-level files Vector merges into one — so a file there can add a route to
 * a router another declares. Left empty, they are every YAML or TOML file in
 * the workspace folder that declares a top-level `sources`, `transforms`,
 * `sinks` or `enrichment_tables`: a guess at which files are Vector's, which
 * the settings exist to replace when the guess is wrong. A guessed file is
 * read as a `--config-dir` one, the only reading in which a file of pieces
 * works at all. The files' contents are read by the wasm module; this only
 * decides which files to hand it, and how Vector would load each.
 */
export const PIPELINE_SETTING = 'vectorConfig';

/** The `--config-dir` directories. See [`PIPELINE_SETTING`]. */
export const PIPELINE_DIR_SETTING = 'vectorConfigDir';

/** Outputs that end on purpose. See `vector_topology::terminal`. */
export const TERMINAL_SETTING = 'terminalOutputs';

/**
 * Whether a settings change changes what is said about the pipeline: which
 * files it is, or how they are read.
 */
export function affectsPipeline(event: vscode.ConfigurationChangeEvent): boolean {
  return [PIPELINE_SETTING, PIPELINE_DIR_SETTING, TERMINAL_SETTING].some((setting) =>
    event.affectsConfiguration(`vrl-tools.${setting}`),
  );
}

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

export interface PipelineFile {
  readonly uri: vscode.Uri;
  /** Relative to the workspace folder, with forward slashes. */
  readonly name: string;
  readonly source: string;
  /**
   * Given to Vector with `--config`, which loads it on its own: nothing in it
   * merges with another file. `vector_topology::ConfigFile::standalone`.
   */
  readonly standalone: boolean;
}

export interface Pipeline {
  /** What the pipeline is called in a title: the patterns, or the folder. */
  readonly title: string;
  /** Stable enough to remember a choice by, across a reload. */
  readonly key: string;
  readonly files: readonly PipelineFile[];
}

/**
 * What one file declares, which is how the files found in a workspace are told
 * apart into pipelines. See `Shape` in `./grouping.ts`.
 *
 * Passed in rather than read here, so this file still knows nothing about the
 * wasm module — it decides which files to hand over, and nothing else.
 */
export type ComponentNames = (file: PipelineFile) => Shape;

/** What a workspace folder says Vector is started with, if anything. */
export interface Configured {
  /** `--config` patterns. */
  readonly files: readonly string[];
  /** `--config-dir` directories, which may be patterns too, as in Vector. */
  readonly dirs: readonly string[];
}

export function configured(folder: vscode.WorkspaceFolder): Configured | undefined {
  const settings = vscode.workspace.getConfiguration('vrl-tools', folder);
  const read = (setting: string) =>
    settings.get<string[]>(setting, []).filter((pattern) => pattern.trim() !== '');
  const found = { files: read(PIPELINE_SETTING), dirs: read(PIPELINE_DIR_SETTING) };
  return found.files.length + found.dirs.length > 0 ? found : undefined;
}

/**
 * The files a configured folder names, and whether each is loaded on its own.
 *
 * A directory contributes the files directly in it with an extension Vector
 * reads (`load_dir`, without recursion). The component subfolders a
 * `--config-dir` may also have — `sources/`, `transforms/`, one component per
 * file — are not read yet. A file named both ways is read as the directory's.
 */
export async function configuredFiles(
  folder: vscode.WorkspaceFolder,
  config: Configured,
): Promise<{ readonly uri: vscode.Uri; readonly standalone: boolean }[]> {
  const inDirs = await Promise.all(
    config.dirs.map((dir) => {
      const base = trimDir(dir);
      const pattern = base === '' || base === '.' ? '*.{yaml,yml,toml,json}' : `${base}/*.{yaml,yml,toml,json}`;
      return vscode.workspace.findFiles(new vscode.RelativePattern(folder, pattern), EXCLUDE_GLOB);
    }),
  );
  const merged = unique(inDirs.flat());
  const alone = (await matching(folder, config.files)).filter(
    (uri) => !merged.some((other) => sameFile(other, uri)),
  );
  return [
    ...alone.map((uri) => ({ uri, standalone: true })),
    ...merged.map((uri) => ({ uri, standalone: false })),
  ].sort((a, b) => a.uri.path.localeCompare(b.uri.path));
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
    files: [{ uri: document.uri, name, source: document.getText(), standalone: false }],
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
  return all.find((pipeline) => pipeline.key === preferred) ?? all[mainPipeline(all, namesOf)];
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
  // Configured files are not a guess: they are what Vector is started with,
  // so they are one pipeline whatever the names do.
  const config = configured(folder);
  if (config) {
    const files = await read(folder, await configuredFiles(folder, config));
    const named = [...config.files, ...config.dirs.map((dir) => `${trimDir(dir)}/`)];
    return files.length > 0 ? [{ title: named.join(', '), key: named.join(','), files }] : [];
  }

  const files = await read(
    folder,
    (await guessed(folder)).map((uri) => ({ uri, standalone: false })),
  );
  const names = new Map<string, Shape>();
  const namesCached: ComponentNames = (file) => {
    const known = names.get(file.name);
    if (known) {
      return known;
    }
    const read = namesOf(file);
    names.set(file.name, read);
    return read;
  };

  return pipelinesOnly(group(files, folder.name, namesCached), namesCached).map((found) => ({
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

/**
 * Every YAML or TOML file in the folder that declares a Vector section, and
 * that the workspace does not exclude. See `./excludes.ts`.
 */
export async function guessed(folder: vscode.WorkspaceFolder): Promise<vscode.Uri[]> {
  const excluded = await exclusions(folder);
  const candidates = (
    await vscode.workspace.findFiles(
      new vscode.RelativePattern(folder, GUESS_GLOB),
      EXCLUDE_GLOB,
      MAX_CANDIDATES,
    )
  ).filter((uri) => !excluded.excludes(relative(folder, uri)));
  const texts = await Promise.all(candidates.map((uri) => textOf(uri)));
  return unique(candidates.filter((_, index) => declaresVectorSection(texts[index] ?? '')));
}

/**
 * What the guess leaves out: `files.exclude`, `search.exclude` and the
 * folder's own `.gitignore`. Only the one at the folder's root — a nested
 * `.gitignore` is rarer, and reading every one means walking the tree the
 * search was meant to spare.
 */
async function exclusions(folder: vscode.WorkspaceFolder): Promise<Exclusions> {
  const setting = (section: string) =>
    settingRules(vscode.workspace.getConfiguration(section, folder).get('exclude'));
  let ignored: string | undefined;
  try {
    ignored = await fs.readFile(vscode.Uri.joinPath(folder.uri, '.gitignore').fsPath, 'utf8');
  } catch {
    ignored = undefined;
  }
  return new Exclusions([
    ...setting('files'),
    ...setting('search'),
    ...(ignored ? gitignoreRules(ignored) : []),
  ]);
}

async function read(
  folder: vscode.WorkspaceFolder,
  found: readonly { readonly uri: vscode.Uri; readonly standalone: boolean }[],
): Promise<PipelineFile[]> {
  const files: PipelineFile[] = [];
  for (const { uri, standalone } of found) {
    const source = await textOf(uri);
    if (source !== undefined) {
      files.push({ uri, name: relative(folder, uri), source, standalone });
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

/** A directory as written in a setting, without its trailing separators. */
function trimDir(dir: string): string {
  return dir.trim().replace(/[\\/]+$/, '');
}

function nameOf(uri: vscode.Uri): string {
  return uri.path.split('/').pop() ?? uri.path;
}
