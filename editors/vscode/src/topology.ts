import * as vscode from 'vscode';

import type { VrlChecker } from './checker';

/**
 * `VRL: Show pipeline graph` — where events go in a Vector config.
 *
 * Vector configs declare their own topology: every transform and sink lists
 * the `inputs` feeding it, so the graph is read rather than guessed. What
 * takes work is resolving what those inputs name, because one can be a
 * wildcard and one can name a single output of a transform rather than the
 * transform itself — `split.errors` is a different path from `split.warnings`,
 * and that distinction is the whole question of which transforms an event
 * passes through.
 *
 * The result is a Markdown document with a Mermaid diagram in it, opened in
 * VS Code's own preview. A document rather than a webview, for the same reason
 * `run` returns one: it can be saved next to the config, committed, and read
 * on GitHub, which renders Mermaid too.
 *
 * `vector graph` does the drawing half of this already, but it needs the
 * binary, it leaves the editor, and it only ever runs on a config Vector has
 * accepted. A config being written is interesting precisely while it is still
 * wrong, which is why the findings are here and not there.
 */
export const GRAPH_COMMAND = 'vrl-tools.showPipelineGraph';

/** URI scheme of the graph documents. */
const SCHEME = 'vrl-graph';

/** The languages a Vector config is written in. */
const CONFIG_LANGUAGES = ['yaml', 'toml'];

export function registerTopology(
  checker: VrlChecker,
  output: vscode.OutputChannel,
): vscode.Disposable[] {
  const graphs = new GraphDocuments();

  return [
    graphs,
    vscode.workspace.registerTextDocumentContentProvider(SCHEME, graphs),
    vscode.commands.registerCommand(GRAPH_COMMAND, async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || !CONFIG_LANGUAGES.includes(editor.document.languageId)) {
        void vscode.window.showWarningMessage(
          'Open a Vector configuration — a .yaml or .toml file — to graph it.',
        );
        return;
      }

      const document = editor.document;
      const name = basename(document.uri);
      const result = checker.topology(document.getText(), name);

      if ('error' in result) {
        // The file does not parse at all, which the YAML or TOML extension is
        // already saying in the editor. Repeating it as a modal would be
        // noise; naming the file and the reason is enough to explain why no
        // graph appeared.
        void vscode.window.showErrorMessage(`${name} could not be read: ${result.error.message}`);
        return;
      }

      output.appendLine(
        `Graphed ${document.uri.fsPath}: ${result.components.length} components, ` +
          `${result.edges.length} connections, ${result.findings.length} findings.`,
      );

      // Built rather than parsed: a config whose name contains `#` or `?`
      // would otherwise have its path truncated at that character.
      const uri = vscode.Uri.from({ scheme: SCHEME, path: `${document.uri.path}.graph.md` });
      graphs.set(uri, result.document);

      // The preview is the point — the diagram is the answer, and the Markdown
      // behind it is the thing you can save and commit. Beside, so the config
      // stays on screen next to the picture of itself.
      await vscode.commands.executeCommand('markdown.showPreviewToSide', uri);
    }),
  ];
}

/** Whether a document is one the graph command can be run on. */
export function isVectorConfig(document: vscode.TextDocument): boolean {
  return CONFIG_LANGUAGES.includes(document.languageId);
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
