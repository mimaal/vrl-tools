import * as vscode from 'vscode';

import type { Check, VrlDiagnostic, VrlRange } from './checker';
import { VrlChecker } from './checker';

/** How long to wait after a keystroke before recompiling. */
const DEBOUNCE_MS = 300;

/**
 * Keeps the diagnostic collection in step with every open VRL document.
 *
 * VRL compiles in well under a millisecond, so the debounce is here to avoid
 * pointless work while someone is mid-word, not because compilation is slow.
 */
export class DiagnosticRunner implements vscode.Disposable {
  private readonly collection = vscode.languages.createDiagnosticCollection('vrl');
  private readonly pending = new Map<string, NodeJS.Timeout>();
  private readonly disposables: vscode.Disposable[] = [];

  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
  ) {
    this.disposables.push(
      vscode.workspace.onDidOpenTextDocument((document) => this.schedule(document, 0)),
      vscode.workspace.onDidChangeTextDocument((event) => this.schedule(event.document, DEBOUNCE_MS)),
      vscode.workspace.onDidCloseTextDocument((document) => this.forget(document)),
    );

    for (const document of vscode.workspace.textDocuments) {
      this.schedule(document, 0);
    }
  }

  private schedule(document: vscode.TextDocument, delay: number): void {
    if (document.languageId !== 'vrl') {
      return;
    }

    const key = document.uri.toString();
    const queued = this.pending.get(key);
    if (queued) {
      clearTimeout(queued);
    }

    this.pending.set(
      key,
      setTimeout(() => {
        this.pending.delete(key);
        this.run(document);
      }, delay),
    );
  }

  private run(document: vscode.TextDocument): void {
    if (document.isClosed) {
      return;
    }

    let result: Check;
    try {
      result = this.checker.check(document.getText());
    } catch (error) {
      // A panic inside the wasm module would otherwise leave stale squiggles on
      // screen with no explanation.
      this.output.appendLine(`Checking ${document.uri.fsPath} failed: ${String(error)}`);
      this.collection.delete(document.uri);
      return;
    }

    this.collection.set(
      document.uri,
      result.diagnostics.map((diagnostic) => toVsCode(diagnostic, document.uri)),
    );
  }

  private forget(document: vscode.TextDocument): void {
    const key = document.uri.toString();
    const queued = this.pending.get(key);
    if (queued) {
      clearTimeout(queued);
      this.pending.delete(key);
    }
    this.collection.delete(document.uri);
  }

  dispose(): void {
    for (const timeout of this.pending.values()) {
      clearTimeout(timeout);
    }
    this.pending.clear();
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
    this.collection.dispose();
  }
}

function toVsCode(diagnostic: VrlDiagnostic, uri: vscode.Uri): vscode.Diagnostic {
  const range = toRange(diagnostic.range);
  const result = new vscode.Diagnostic(range, diagnostic.message, toSeverity(diagnostic.severity));

  result.source = 'vrl';
  result.code = diagnostic.documentationUrl
    ? { value: `E${diagnostic.code}`, target: vscode.Uri.parse(diagnostic.documentationUrl) }
    : `E${diagnostic.code}`;

  // The compiler's context labels and notes carry most of the explanation.
  // Keeping them as related information leaves the message itself short enough
  // to read in the hover, and gives the labels a clickable position.
  const related: vscode.DiagnosticRelatedInformation[] = [];

  for (const label of diagnostic.labels) {
    if (label.primary && label.message === diagnostic.message) {
      continue;
    }
    related.push(
      new vscode.DiagnosticRelatedInformation(
        new vscode.Location(uri, toRange(label.range)),
        label.message,
      ),
    );
  }

  for (const note of diagnostic.notes) {
    related.push(new vscode.DiagnosticRelatedInformation(new vscode.Location(uri, range), note));
  }

  result.relatedInformation = related;
  return result;
}

function toRange(range: VrlRange): vscode.Range {
  return new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  );
}

function toSeverity(severity: VrlDiagnostic['severity']): vscode.DiagnosticSeverity {
  switch (severity) {
    case 'bug':
    case 'error':
      return vscode.DiagnosticSeverity.Error;
    case 'warning':
      return vscode.DiagnosticSeverity.Warning;
    case 'note':
      return vscode.DiagnosticSeverity.Information;
  }
}
