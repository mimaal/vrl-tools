import * as vscode from 'vscode';

import type { Topology, TopologyFinding, VrlChecker } from './checker';
import { markerEdit, takesComments } from './markers';
import { affectsPipeline, pipelineOf, sameFile, TERMINAL_SETTING } from './pipeline';
import type { Pipeline } from './pipeline';
import { readPipeline } from './reading';
import { componentNames } from './topology';

/**
 * The graph's findings, in the Problems panel and under the text they are
 * about.
 *
 * They were only in the graph panel and in the sidebar, which is where you
 * look when you are looking at the pipeline. A config being edited is looked
 * at in the editor, and an input that names nothing belongs under the input:
 * these are the same findings, placed where the config is written.
 *
 * It is also what a Quick Fix needs. The one offered here is for the only
 * finding that can be answered with "yes, on purpose" — an output nothing
 * reads — and it writes the comment, or the setting, that says so.
 *
 * The pipeline checked is the one the config in front belongs to, every file
 * of it, so an input naming a source in another file is not an error here
 * either.
 */

/** What the diagnostics are attributed to in the Problems panel. */
const SOURCE = 'vrl-tools';

/** The code of the one finding with a fix. */
const UNREAD = 'unread-output';

/** Adds an output to `vrl-tools.terminalOutputs`. */
export const ADD_TERMINAL_COMMAND = 'vrl-tools.addTerminalOutput';

/** How long to wait after a keystroke before re-reading the pipeline. */
const DEBOUNCE_MS = 400;

const CONFIG_FILE = /\.(ya?ml|toml|json)$/i;

/** What a Quick Fix needs to know about one unread output. */
interface Fixable {
  readonly output: string;
  readonly finding: TopologyFinding;
  /** The file and line the marker goes on, and the name to write after it. */
  readonly target: vscode.Uri | undefined;
  readonly line: number;
  readonly name: string | null;
}

export function registerFindings(checker: VrlChecker, output: vscode.OutputChannel): vscode.Disposable[] {
  const findings = new PipelineFindings(checker, output);
  const check = (document: vscode.TextDocument | undefined): void => {
    if (document && CONFIG_FILE.test(document.uri.path)) {
      findings.schedule(document);
    }
  };
  check(vscode.window.activeTextEditor?.document);

  return [
    findings,
    vscode.window.onDidChangeActiveTextEditor((editor) => check(editor?.document)),
    vscode.workspace.onDidChangeTextDocument((event) => check(event.document)),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (affectsPipeline(event)) {
        check(vscode.window.activeTextEditor?.document);
      }
    }),
    vscode.languages.registerCodeActionsProvider(
      { scheme: 'file', pattern: '**/*.{yaml,yml,toml,json}' },
      findings,
      { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] },
    ),
    vscode.commands.registerCommand(
      ADD_TERMINAL_COMMAND,
      async (written: string, resource: vscode.Uri) => {
        const settings = vscode.workspace.getConfiguration('vrl-tools', resource);
        const current = settings.get<string[]>(TERMINAL_SETTING, []);
        if (!current.includes(written)) {
          // In the folder's settings: which outputs end on purpose is a fact
          // about this pipeline, not about the person editing it.
          await settings.update(
            TERMINAL_SETTING,
            [...current, written],
            vscode.workspace.getWorkspaceFolder(resource)
              ? vscode.ConfigurationTarget.WorkspaceFolder
              : vscode.ConfigurationTarget.Global,
          );
        }
      },
    ),
  ];
}

class PipelineFindings implements vscode.CodeActionProvider, vscode.Disposable {
  private readonly collection = vscode.languages.createDiagnosticCollection('vector-pipeline');
  private pending: NodeJS.Timeout | undefined;
  /** The read in flight, so a burst of edits does not land out of order. */
  private generation = 0;
  /** The unread outputs of the pipeline last read, by the file their finding is in. */
  private fixable = new Map<string, Fixable[]>();

  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
  ) {}

  schedule(document: vscode.TextDocument): void {
    clearTimeout(this.pending);
    this.pending = setTimeout(() => void this.run(document), DEBOUNCE_MS);
  }

  private async run(document: vscode.TextDocument): Promise<void> {
    const generation = ++this.generation;
    let pipeline: Pipeline;
    let analysis: Topology;
    try {
      pipeline = await pipelineOf(document, componentNames(this.checker));
      analysis = await readPipeline(this.checker, pipeline);
    } catch (error) {
      this.output.appendLine(`Checking the pipeline of ${document.uri.fsPath} failed: ${String(error)}`);
      return;
    }
    if (generation !== this.generation) {
      return;
    }
    // A file that does not parse right now is a file being typed: its
    // components are missing, and every input naming one would be reported.
    // What was said before it broke stays.
    if (analysis.unreadable.length > 0) {
      return;
    }

    const byFile = new Map<number, vscode.Diagnostic[]>();
    const fixable = new Map<string, Fixable[]>();
    const unread = new Map(analysis.unread.map((entry) => [entry.finding, entry]));

    analysis.findings.forEach((finding, index) => {
      const diagnostic = new vscode.Diagnostic(
        toRange(finding.range),
        finding.message.replaceAll('`', ''),
        SEVERITY[finding.severity],
      );
      diagnostic.source = SOURCE;

      const entry = unread.get(index);
      const file = pipeline.files[finding.file];
      if (entry && file) {
        diagnostic.code = UNREAD;
        const key = keyOf(file.uri);
        fixable.set(key, [
          ...(fixable.get(key) ?? []),
          {
            output: entry.output,
            finding,
            target: pipeline.files[entry.mark.file]?.uri,
            line: entry.mark.line,
            name: entry.mark.name,
          },
        ]);
      }
      byFile.set(finding.file, [...(byFile.get(finding.file) ?? []), diagnostic]);
    });

    // A file with no components is not a Vector config. Opening a Kubernetes
    // manifest says nothing about it, and takes nothing back about the
    // pipeline that was being looked at before.
    if (analysis.components.length === 0) {
      return;
    }
    this.collection.clear();
    this.fixable = fixable;
    pipeline.files.forEach((file, index) => {
      this.collection.set(file.uri, byFile.get(index) ?? []);
    });
  }

  async provideCodeActions(
    document: vscode.TextDocument,
    _range: vscode.Range,
    context: vscode.CodeActionContext,
  ): Promise<vscode.CodeAction[]> {
    const known = this.fixable.get(keyOf(document.uri)) ?? [];
    const actions: vscode.CodeAction[] = [];

    for (const diagnostic of context.diagnostics) {
      if (diagnostic.source !== SOURCE || diagnostic.code !== UNREAD) {
        continue;
      }
      const entry = known.find(
        (candidate) =>
          toRange(candidate.finding.range).isEqual(diagnostic.range) &&
          candidate.finding.message.replaceAll('`', '') === diagnostic.message,
      );
      if (!entry) {
        continue;
      }

      const comment = await this.comment(document, entry);
      if (comment) {
        comment.diagnostics = [diagnostic];
        comment.isPreferred = true;
        actions.push(comment);
      }

      const setting = new vscode.CodeAction(
        `Mark ${entry.output} as terminal in the settings`,
        vscode.CodeActionKind.QuickFix,
      );
      setting.diagnostics = [diagnostic];
      setting.command = {
        command: ADD_TERMINAL_COMMAND,
        title: 'Add to vrl-tools.terminalOutputs',
        arguments: [entry.output, document.uri],
      };
      actions.push(setting);
    }
    return actions;
  }

  /** The fix that writes the comment, when the line can take one. */
  private async comment(
    document: vscode.TextDocument,
    entry: Fixable,
  ): Promise<vscode.CodeAction | undefined> {
    if (!entry.target || !takesComments(entry.target.path)) {
      return undefined;
    }

    let target: vscode.TextDocument;
    try {
      target = sameFile(entry.target, document.uri)
        ? document
        : await vscode.workspace.openTextDocument(entry.target);
    } catch {
      return undefined;
    }
    if (entry.line >= target.lineCount) {
      return undefined;
    }
    const edit = markerEdit(target.lineAt(entry.line).text, entry.name);
    if (!edit) {
      return undefined;
    }

    const action = new vscode.CodeAction(
      `Mark ${entry.output} as terminal with a comment`,
      vscode.CodeActionKind.QuickFix,
    );
    action.edit = new vscode.WorkspaceEdit();
    action.edit.insert(target.uri, new vscode.Position(entry.line, edit.at), edit.text);
    return action;
  }

  dispose(): void {
    clearTimeout(this.pending);
    this.collection.dispose();
  }
}

/** One key per file: Windows drive letters differ in case between APIs. */
function keyOf(uri: vscode.Uri): string {
  return process.platform === 'win32' ? uri.toString().toLowerCase() : uri.toString();
}

const SEVERITY = {
  error: vscode.DiagnosticSeverity.Error,
  warning: vscode.DiagnosticSeverity.Warning,
  info: vscode.DiagnosticSeverity.Information,
} as const;

function toRange(range: TopologyFinding['range']): vscode.Range {
  return new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  );
}
