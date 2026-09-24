import * as vscode from 'vscode';

import type { Topology, VrlChecker, VrlRange } from './checker';
import { anyConfig, CONFIG_GLOB, PIPELINE_SETTING, pipelineOf, sameFile } from './pipeline';
import type { ComponentNames, PipelineChoice } from './pipeline';
import type { Pipeline, PipelineFile } from './pipeline';

/**
 * `VRL: Show pipeline graph` — where events go in a Vector config.
 *
 * Vector configs declare their own topology: every transform and sink lists
 * the `inputs` feeding it, so the graph is read rather than guessed. What
 * takes work is resolving what those inputs name, because one can be a
 * wildcard and one can name a single output of a transform rather than the
 * transform itself — `split.errors` is a different path from `split.warnings`,
 * and that distinction is the whole question of which transforms an event
 * passes through. All of that is the wasm module's; this file shows it.
 *
 * The graph opens in a panel beside the config, drawn to match the theme, and
 * follows it: edit the config and the graph redraws, click a component and the
 * editor goes to it. The same graph can be exported as Markdown with a Mermaid
 * block, which is the form worth committing next to the config, since GitHub
 * renders it too.
 *
 * `vector graph` does the drawing half of this already, but it needs the
 * binary, it leaves the editor, and it only ever runs on a config Vector has
 * accepted. A config being written is interesting precisely while it is still
 * wrong, which is why the findings are here and not there.
 */
export const GRAPH_COMMAND = 'vrl-tools.showPipelineGraph';
export const EXPORT_COMMAND = 'vrl-tools.exportPipelineGraph';
/** Go to a component in the config and narrow the graph to it. */
export const REVEAL_COMMAND = 'vrl-tools.revealComponent';

/**
 * Set on the active editor when it holds a Vector config, which is what puts
 * the graph button in the editor title. "A YAML file" would put it on every
 * Kubernetes manifest and GitHub workflow as well.
 */
const CONTEXT_KEY = 'vrl-tools.isVectorConfig';

/** URI scheme of the exported Markdown documents. */
const SCHEME = 'vrl-graph';

/** The file extensions a Vector config has. */
const CONFIG_FILE = /\.(ya?ml|toml|json)$/i;

/** How long to wait after a keystroke before redrawing. */
const DEBOUNCE_MS = 250;

export function registerTopology(
  checker: VrlChecker,
  output: vscode.OutputChannel,
  extensionUri: vscode.Uri,
  choice: PipelineChoice,
): vscode.Disposable[] {
  const exported = new GraphDocuments();
  const panel = new GraphPanel(checker, output, extensionUri, choice, (document) =>
    exportMarkdown(checker, exported, document),
  );

  const updateContext = (editor: vscode.TextEditor | undefined): void => {
    void vscode.commands.executeCommand(
      'setContext',
      CONTEXT_KEY,
      editor !== undefined && isVectorConfig(checker, editor.document),
    );
  };
  updateContext(vscode.window.activeTextEditor);

  let pendingContext: NodeJS.Timeout | undefined;

  return [
    exported,
    panel,
    { dispose: () => clearTimeout(pendingContext) },
    vscode.workspace.registerTextDocumentContentProvider(SCHEME, exported),
    vscode.window.onDidChangeActiveTextEditor(updateContext),
    // A file becomes a Vector config the moment its first `sources:` goes in.
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document !== vscode.window.activeTextEditor?.document) {
        return;
      }
      clearTimeout(pendingContext);
      pendingContext = setTimeout(
        () => updateContext(vscode.window.activeTextEditor),
        DEBOUNCE_MS,
      );
    }),
    vscode.commands.registerCommand(GRAPH_COMMAND, async (focus?: string) => {
      const document = await configToGraph(panel, checker, choice);
      if (document) {
        panel.show(document, typeof focus === 'string' ? focus : undefined);
      }
    }),
    vscode.commands.registerCommand(EXPORT_COMMAND, async () => {
      const document = await configToGraph(panel, checker, choice);
      if (document) {
        await exportMarkdown(checker, exported, document);
      }
    }),
    // What the sidebar's tree items run: go to the declaration and narrow the
    // graph to that component's paths, which is the pair of things wanted on
    // every click.
    vscode.commands.registerCommand(
      REVEAL_COMMAND,
      async (target: { uri: vscode.Uri; range: VrlRange; id: string }) => {
        const document = await configToGraph(panel, checker, choice);
        if (document) {
          panel.show(document, target.id);
        }
        await reveal(vscode.Uri.from(target.uri), target.range);
      },
    ),
  ];
}

/**
 * Whether a document is a Vector config: YAML or TOML with at least one
 * source, transform or sink in it, as the topology reader sees it.
 */
export function isVectorConfig(checker: VrlChecker, document: vscode.TextDocument): boolean {
  const name = configName(document);
  if (!name) {
    return false;
  }
  try {
    const result = checker.topology(document.getText(), name);
    return !('error' in result) && result.components.length > 0;
  } catch {
    return false;
  }
}

/**
 * The name a document is read as, which is what picks the YAML or the TOML
 * parser: its file name when that ends in `.yaml`, `.yml` or `.toml`.
 *
 * The file name, not the language VS Code assigned. VS Code has YAML built in
 * but not TOML, so without a TOML extension installed a `vector.toml` opens
 * as plain text, and going by the language left it with no graph at all.
 * The language is only the fallback, for an untitled document that has no
 * file name to go by.
 */
function configName(document: vscode.TextDocument): string | undefined {
  const name = basename(document.uri);
  if (CONFIG_FILE.test(name)) {
    return name;
  }
  if (['yaml', 'toml', 'json'].includes(document.languageId)) {
    return `${name}.${document.languageId}`;
  }
  return undefined;
}

/**
 * What each config file declares, which is how the workspace's files are told
 * apart into pipelines. A file that does not parse declares nothing, which
 * keeps a config mid-edit from splitting the pipeline it belongs to.
 *
 * Answers are kept, keyed by the file's own text, because this is asked of
 * every config in the workspace every time either the graph or the sidebar
 * redraws — twice per keystroke, once each, on a debounce. Typing in one
 * config changes one file's text; the other seventeen answer from here instead
 * of crossing into wasm and parsing again.
 */
const DECLARED = new Map<string, { source: string; names: readonly string[] }>();

/** Enough for any workspace; a cache that grows without end is a leak. */
const DECLARED_LIMIT = 512;

export function componentNames(checker: VrlChecker): ComponentNames {
  return (file) => {
    const known = DECLARED.get(file.name);
    if (known && known.source === file.source) {
      return known.names;
    }

    let names: readonly string[] = [];
    try {
      const result = checker.topology(file.source, file.name);
      names = 'error' in result ? [] : result.components.map((component) => component.id);
    } catch {
      names = [];
    }

    // Emptied rather than evicted one by one: this is a cache in front of a
    // cheap answer, and the next few reads refilling it costs less than
    // keeping an eviction order.
    if (DECLARED.size >= DECLARED_LIMIT) {
      DECLARED.clear();
    }
    DECLARED.set(file.name, { source: file.source, names });
    return names;
  };
}

/**
 * The config to graph, which is deliberately not "the one in front".
 *
 * A pipeline is the same graph whichever of its files you ask from, and most
 * of the time what is in front is the `.vrl` file whose transform you are
 * writing — or a README, or nothing. So: the active editor when it is a
 * config, the one the panel is already showing, and failing both, the first
 * config in the workspace. Only a workspace with no Vector config at all has
 * nothing to answer with, and that is the one case worth a message.
 */
async function configToGraph(
  panel: GraphPanel,
  checker: VrlChecker,
  choice: PipelineChoice,
): Promise<vscode.TextDocument | undefined> {
  const active = vscode.window.activeTextEditor?.document;
  if (active && configName(active)) {
    return active;
  }
  if (panel.document) {
    return panel.document;
  }

  const found = await anyConfig(componentNames(checker), choice.key);
  if (!found) {
    void vscode.window.showWarningMessage(
      'No Vector configuration found in this workspace. Open one, or point `vrl-tools.vectorConfig` at the files you start Vector with.',
    );
    return undefined;
  }
  return vscode.workspace.openTextDocument(found);
}

/** Opens a config at a range, in the column it is already open in if it is. */
async function reveal(uri: vscode.Uri, range: VrlRange): Promise<void> {
  const target = new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  );
  // Back in the column a config is already open in, rather than a new tab on
  // top of the graph.
  const visible = vscode.window.visibleTextEditors.find((e) => sameFile(e.document.uri, uri));
  const editor = await vscode.window.showTextDocument(uri, {
    viewColumn: visible?.viewColumn ?? vscode.ViewColumn.One,
    selection: target,
  });
  editor.revealRange(target, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
}

/**
 * Reads the pipeline `document` belongs to, every file of it. See
 * `pipelineOf` for which files those are.
 */
async function analyse(
  checker: VrlChecker,
  document: vscode.TextDocument,
  focus?: string,
): Promise<{ pipeline: Pipeline; analysis: Topology }> {
  const pipeline = await pipelineOf(document, componentNames(checker));
  const files = pipeline.files.map((file) => ({
    // The name picks the parser, so a file without a telling extension (an
    // untitled one) is named for its language instead.
    name: sameFile(file.uri, document.uri) ? relativeConfigName(file.name, document) : file.name,
    source: file.source,
  }));
  return { pipeline, analysis: checker.topologyFiles(files, pipeline.title, focus) };
}

/** A file's name as given, with its format appended when the name has none. */
function relativeConfigName(name: string, document: vscode.TextDocument): string {
  return CONFIG_FILE.test(name) ? name : (configName(document) ?? name);
}

async function exportMarkdown(
  checker: VrlChecker,
  exported: GraphDocuments,
  document: vscode.TextDocument,
): Promise<void> {
  const { analysis } = await analyse(checker, document);
  if (analysis.unreadable.length > 0) {
    const first = analysis.unreadable[0];
    void vscode.window.showErrorMessage(
      `${analysis.files[first.file]} could not be read: ${first.message}`,
    );
    return;
  }

  // Built rather than parsed: a config whose name contains `#` or `?` would
  // otherwise have its path truncated at that character.
  const uri = vscode.Uri.from({ scheme: SCHEME, path: `${document.uri.path}.graph.md` });
  exported.set(uri, analysis.document);
  await vscode.commands.executeCommand('markdown.showPreviewToSide', uri);
}

/** What the webview sends back. */
type FromWebview =
  | { readonly type: 'ready' }
  | { readonly type: 'export' }
  | { readonly type: 'reveal'; readonly range: VrlRange; readonly file: number }
  | { readonly type: 'focus'; readonly id: string | null };

/**
 * The one graph panel.
 *
 * One rather than one per config: the panel follows whichever Vector config
 * is in front, the way the Markdown preview follows the active file. What it
 * draws is the whole pipeline that config belongs to, so moving between the
 * files of one pipeline keeps the same graph, and editing any of them redraws
 * it.
 */
class GraphPanel implements vscode.Disposable {
  private panel: vscode.WebviewPanel | undefined;
  /** The config the graph was opened from; its pipeline is what is drawn. */
  private shown: vscode.TextDocument | undefined;
  /** The files of the pipeline last drawn, in the order the analysis numbers them. */
  private files: readonly PipelineFile[] = [];
  /** Whether a complete graph has been drawn since the panel opened. */
  private drawn = false;
  /** The component the graph is narrowed to, if the person asked for that. */
  private focus: string | undefined;
  private pending: NodeJS.Timeout | undefined;
  /** The draw in flight, so a burst of edits does not interleave reads. */
  private generation = 0;
  private readonly disposables: vscode.Disposable[] = [];

  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
    private readonly extensionUri: vscode.Uri,
    private readonly choice: PipelineChoice,
    private readonly onExport: (document: vscode.TextDocument) => Promise<void>,
  ) {
    const watcher = vscode.workspace.createFileSystemWatcher(CONFIG_GLOB);
    const onDisk = (): void => {
      if (this.panel) this.schedule(false);
    };
    this.disposables.push(
      watcher,
      // A file of the pipeline created, deleted or changed outside the editor.
      watcher.onDidCreate(onDisk),
      watcher.onDidDelete(onDisk),
      watcher.onDidChange(onDisk),
      vscode.workspace.onDidChangeTextDocument((event) => {
        if (this.panel && this.belongs(event.document)) {
          this.schedule(false);
        }
      }),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        if (
          this.panel &&
          editor &&
          editor.document !== this.shown &&
          !this.files.some((file) => sameFile(file.uri, editor.document.uri)) &&
          isVectorConfig(this.checker, editor.document)
        ) {
          // A config from another pipeline, or one opened on its own.
          this.shown = editor.document;
          this.drawn = false;
          this.focus = undefined;
          void this.post(true);
        }
      }),
      vscode.workspace.onDidChangeConfiguration((event) => {
        if (this.panel && event.affectsConfiguration(`vrl-tools.${PIPELINE_SETTING}`)) {
          this.drawn = false;
          void this.post(true);
        }
      }),
      // Another pipeline was picked in the sidebar: the panel follows, so the
      // two never show different things.
      this.choice.onDidChange(() => {
        if (this.panel) {
          this.shown = undefined;
          this.files = [];
          this.drawn = false;
          this.focus = undefined;
          void vscode.commands.executeCommand(GRAPH_COMMAND);
        }
      }),
    );
  }

  /** The config the panel is showing, if it is open. */
  get document(): vscode.TextDocument | undefined {
    return this.panel ? this.shown : undefined;
  }

  /** Whether an edit to `document` can change the graph on screen. */
  private belongs(document: vscode.TextDocument): boolean {
    return (
      document === this.shown ||
      this.files.some((file) => sameFile(file.uri, document.uri)) ||
      // A config that becomes one by being typed into joins a guessed pipeline.
      CONFIG_FILE.test(document.uri.path)
    );
  }

  /**
   * Opens the panel on `document`'s pipeline, narrowed to `focus` when one is
   * given. Asking for a component the graph is already showing just reveals
   * the panel, so clicking twice in the sidebar does not redraw twice.
   */
  show(document: vscode.TextDocument, focus?: string): void {
    const changed =
      document !== this.shown && !this.files.some((file) => sameFile(file.uri, document.uri));
    this.shown = document;

    if (this.panel) {
      this.panel.reveal(vscode.ViewColumn.Beside, true);
      if (changed || focus !== this.focus) {
        this.drawn = this.drawn && !changed;
        this.focus = focus;
        void this.post(true);
      }
      return;
    }

    this.focus = focus;

    const media = vscode.Uri.joinPath(this.extensionUri, 'media');
    this.panel = vscode.window.createWebviewPanel(
      'vrl-tools.pipelineGraph',
      'Pipeline graph',
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      { enableScripts: true, localResourceRoots: [media] },
    );
    this.panel.iconPath = vscode.Uri.joinPath(media, 'icon.png');
    this.panel.webview.html = html(this.panel.webview, media);
    this.drawn = false;

    this.panel.webview.onDidReceiveMessage(
      (message: FromWebview) => this.receive(message),
      undefined,
      this.disposables,
    );
    this.panel.onDidDispose(() => {
      this.panel = undefined;
      this.files = [];
      this.focus = undefined;
      clearTimeout(this.pending);
    });
  }

  private receive(message: FromWebview): void {
    const document = this.shown;
    if (!document) {
      return;
    }
    switch (message.type) {
      case 'ready':
        void this.post(true);
        break;
      case 'export':
        void this.onExport(document);
        break;
      case 'reveal': {
        const file = this.files[message.file];
        void reveal(file ? file.uri : document.uri, message.range);
        break;
      }
      case 'focus':
        this.focus = message.id ?? undefined;
        void this.post(true);
        break;
    }
  }

  private schedule(refit: boolean): void {
    clearTimeout(this.pending);
    this.pending = setTimeout(() => void this.post(refit), DEBOUNCE_MS);
  }

  private async post(refit: boolean): Promise<void> {
    const document = this.shown;
    if (!this.panel || !document) {
      return;
    }
    const generation = ++this.generation;

    let result: { pipeline: Pipeline; analysis: Topology };
    try {
      result = await analyse(this.checker, document, this.focus);
    } catch (error) {
      this.output.appendLine(`Graphing ${document.uri.fsPath} failed: ${String(error)}`);
      return;
    }
    // A later edit started another draw while this one was reading files.
    if (generation !== this.generation || !this.panel) {
      return;
    }

    const { pipeline, analysis } = result;
    this.panel.title = `Graph: ${pipeline.title}`;
    // The focused component was renamed or deleted: back to everything.
    if (this.focus && !analysis.focus) {
      this.focus = undefined;
    }

    // A file that does not parse right now is a file being typed. Its
    // components are missing from this analysis, so drawing it would show
    // every input that names them as broken. The last complete graph stays on
    // screen, with the reason above it. Only when there is none yet is the
    // partial one drawn, since something beats nothing.
    const unreadable = analysis.unreadable
      .map((entry) => `${analysis.files[entry.file]}: ${entry.message}`)
      .join('; ');
    if (unreadable && this.drawn) {
      void this.panel.webview.postMessage({ type: 'unreadable', message: unreadable });
      return;
    }

    this.files = pipeline.files;
    this.drawn = !unreadable;
    void this.panel.webview.postMessage({
      type: 'graph',
      title: pipeline.title,
      analysis,
      refit,
      warning: unreadable || undefined,
    });
  }

  dispose(): void {
    clearTimeout(this.pending);
    this.panel?.dispose();
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
  }
}

/**
 * The panel's page. Scripts only from the extension's `media` folder, with a
 * nonce, and nothing from the network: a config's component names end up in
 * this page, and the content security policy is what makes that harmless.
 */
function html(webview: vscode.Webview, media: vscode.Uri): string {
  const nonce = [...Array(32)].map(() => Math.floor(Math.random() * 36).toString(36)).join('');
  const script = webview.asWebviewUri(vscode.Uri.joinPath(media, 'graph.js'));
  const style = webview.asWebviewUri(vscode.Uri.joinPath(media, 'graph.css'));

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <link rel="stylesheet" href="${style.toString()}">
  <title>Pipeline graph</title>
</head>
<body>
  <header>
    <h1 id="title"></h1>
    <span id="summary"></span>
    <span id="focus-pill" role="status">Paths through <strong id="focus-name"></strong><button id="unfocus" title="Show the whole pipeline (Esc)">Show all</button></span>
    <span class="spacer"></span>
    <span class="search"><input id="search" type="search" placeholder="Find a component (Ctrl+F)" aria-label="Find a component by name, type or file" spellcheck="false"><span id="match-count" aria-live="polite"></span></span>
    <button id="fit" title="Fit the whole pipeline in view (0)">Fit</button>
    <button id="export" title="Open as Markdown with a Mermaid diagram, to save next to the config">Export Markdown</button>
  </header>
  <div id="banner" role="status"></div>
  <main>
    <svg id="canvas" role="img" aria-label="Pipeline graph">
      <defs>
        <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z"/></marker>
        <marker id="arrow-active" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z"/></marker>
        <marker id="arrow-error" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z"/></marker>
      </defs>
      <g id="viewport"><g id="edges"></g><g id="labels"></g><g id="nodes"></g></g>
    </svg>
    <svg id="minimap" aria-hidden="true"></svg>
    <div id="hint"><span class="legend"><span class="source">source</span><span class="transform">transform</span><span class="sink">sink</span><span class="table">table</span></span><span>Scroll to move · Ctrl+scroll to zoom · Click a component to follow its paths</span></div>
    <div id="empty">No sources, transforms or sinks yet.<br>Components appear here as the config declares them.</div>
  </main>
  <div id="details" role="region" aria-label="Selected component"></div>
  <section id="problems"><h2>Problems</h2><ul id="problem-list"></ul></section>
  <script nonce="${nonce}" src="${script.toString()}"></script>
</body>
</html>`;
}

class GraphDocuments implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly contents = new Map<string, string>();
  private readonly emitter = new vscode.EventEmitter<vscode.Uri>();

  readonly onDidChange = this.emitter.event;

  set(uri: vscode.Uri, content: string): void {
    this.contents.set(uri.toString(), content);
    this.emitter.fire(uri);
  }

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.contents.get(uri.toString()) ?? '# This graph is no longer available.\n';
  }

  dispose(): void {
    this.emitter.dispose();
    this.contents.clear();
  }
}

function basename(uri: vscode.Uri): string {
  return uri.path.split('/').pop() ?? uri.path;
}
