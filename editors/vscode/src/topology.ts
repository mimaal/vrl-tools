import * as vscode from 'vscode';

import type { Topology, VrlChecker, VrlRange } from './checker';

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

/**
 * Set on the active editor when it holds a Vector config, which is what puts
 * the graph button in the editor title. "A YAML file" would put it on every
 * Kubernetes manifest and GitHub workflow as well.
 */
const CONTEXT_KEY = 'vrl-tools.isVectorConfig';

/** URI scheme of the exported Markdown documents. */
const SCHEME = 'vrl-graph';

/** The file extensions a Vector config has. */
const CONFIG_FILE = /\.(ya?ml|toml)$/i;

/** How long to wait after a keystroke before redrawing. */
const DEBOUNCE_MS = 250;

export function registerTopology(
  checker: VrlChecker,
  output: vscode.OutputChannel,
  extensionUri: vscode.Uri,
): vscode.Disposable[] {
  const exported = new GraphDocuments();
  const panel = new GraphPanel(checker, output, extensionUri, (document) =>
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
    vscode.commands.registerCommand(GRAPH_COMMAND, () => {
      const document = activeConfig();
      if (document) {
        panel.show(document);
      }
    }),
    vscode.commands.registerCommand(EXPORT_COMMAND, async () => {
      const document = activeConfig() ?? panel.document;
      if (document) {
        await exportMarkdown(checker, exported, document);
      }
    }),
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
  if (document.languageId === 'yaml' || document.languageId === 'toml') {
    return `${name}.${document.languageId}`;
  }
  return undefined;
}

function activeConfig(): vscode.TextDocument | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor || !configName(editor.document)) {
    void vscode.window.showWarningMessage(
      'Open a Vector configuration — a .yaml or .toml file — to graph it.',
    );
    return undefined;
  }
  return editor.document;
}

async function exportMarkdown(
  checker: VrlChecker,
  exported: GraphDocuments,
  document: vscode.TextDocument,
): Promise<void> {
  const name = basename(document.uri);
  const result = checker.topology(document.getText(), configName(document) ?? name);
  if ('error' in result) {
    void vscode.window.showErrorMessage(`${name} could not be read: ${result.error.message}`);
    return;
  }

  // Built rather than parsed: a config whose name contains `#` or `?` would
  // otherwise have its path truncated at that character.
  const uri = vscode.Uri.from({ scheme: SCHEME, path: `${document.uri.path}.graph.md` });
  exported.set(uri, result.document);
  await vscode.commands.executeCommand('markdown.showPreviewToSide', uri);
}

/** What the webview sends back. */
type FromWebview =
  | { readonly type: 'ready' }
  | { readonly type: 'export' }
  | { readonly type: 'reveal'; readonly range: VrlRange };

/**
 * The one graph panel.
 *
 * One rather than one per config: the panel follows whichever Vector config
 * is in front, the way the Markdown preview follows the active file, so
 * switching between `sources.yaml` and `sinks.yaml` does not pile up tabs.
 */
class GraphPanel implements vscode.Disposable {
  private panel: vscode.WebviewPanel | undefined;
  private shown: vscode.TextDocument | undefined;
  private pending: NodeJS.Timeout | undefined;
  private readonly disposables: vscode.Disposable[] = [];

  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
    private readonly extensionUri: vscode.Uri,
    private readonly onExport: (document: vscode.TextDocument) => Promise<void>,
  ) {
    this.disposables.push(
      vscode.workspace.onDidChangeTextDocument((event) => {
        if (event.document === this.shown) {
          this.schedule(false);
        }
      }),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        if (
          this.panel &&
          editor &&
          editor.document !== this.shown &&
          isVectorConfig(this.checker, editor.document)
        ) {
          this.shown = editor.document;
          this.post(true);
        }
      }),
    );
  }

  /** The config the panel is showing, if it is open. */
  get document(): vscode.TextDocument | undefined {
    return this.panel ? this.shown : undefined;
  }

  show(document: vscode.TextDocument): void {
    const changed = document !== this.shown;
    this.shown = document;

    if (this.panel) {
      this.panel.reveal(vscode.ViewColumn.Beside, true);
      if (changed) {
        this.post(true);
      }
      return;
    }

    const media = vscode.Uri.joinPath(this.extensionUri, 'media');
    this.panel = vscode.window.createWebviewPanel(
      'vrl-tools.pipelineGraph',
      'Pipeline graph',
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      { enableScripts: true, localResourceRoots: [media] },
    );
    this.panel.iconPath = vscode.Uri.joinPath(media, 'icon.png');
    this.panel.webview.html = html(this.panel.webview, media);

    this.panel.webview.onDidReceiveMessage(
      (message: FromWebview) => this.receive(message),
      undefined,
      this.disposables,
    );
    this.panel.onDidDispose(() => {
      this.panel = undefined;
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
        this.post(true);
        break;
      case 'export':
        void this.onExport(document);
        break;
      case 'reveal':
        void this.reveal(document, message.range);
        break;
    }
  }

  private async reveal(document: vscode.TextDocument, range: VrlRange): Promise<void> {
    const target = new vscode.Range(
      range.start.line,
      range.start.character,
      range.end.line,
      range.end.character,
    );
    // Back in the column the config is already open in, rather than a new
    // tab on top of the graph.
    const visible = vscode.window.visibleTextEditors.find((e) => e.document === document);
    const editor = await vscode.window.showTextDocument(document, {
      viewColumn: visible?.viewColumn ?? vscode.ViewColumn.One,
      selection: target,
    });
    editor.revealRange(target, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  }

  private schedule(refit: boolean): void {
    clearTimeout(this.pending);
    this.pending = setTimeout(() => this.post(refit), DEBOUNCE_MS);
  }

  private post(refit: boolean): void {
    const document = this.shown;
    if (!this.panel || !document) {
      return;
    }

    const name = basename(document.uri);
    this.panel.title = `Graph: ${name}`;

    let result: Topology | { error: { message: string } };
    try {
      result = this.checker.topology(document.getText(), configName(document) ?? name);
    } catch (error) {
      this.output.appendLine(`Graphing ${document.uri.fsPath} failed: ${String(error)}`);
      return;
    }

    if ('error' in result) {
      void this.panel.webview.postMessage({ type: 'unreadable', message: result.error.message });
      return;
    }

    void this.panel.webview.postMessage({ type: 'graph', title: name, analysis: result, refit });
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
    <span class="legend"><span class="source">source</span><span class="transform">transform</span><span class="sink">sink</span></span>
    <span class="spacer"></span>
    <button id="fit" title="Fit the whole pipeline in view">Fit</button>
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
    <div id="empty">No sources, transforms or sinks yet.<br>Components appear here as the config declares them.</div>
  </main>
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
