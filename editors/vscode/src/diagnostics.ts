import * as vscode from 'vscode';

import type { Check, VrlDiagnostic, VrlRange } from './checker';
import { VrlChecker } from './checker';
import { isSample, programFor, SampleStore } from './sample';
import type { StatusBar } from './status';

/** How long to wait after a keystroke before recompiling. */
const DEBOUNCE_MS = 300;

/** The setting that turns sample-typed diagnostics off. */
const USE_SAMPLE = 'useSampleEventForDiagnostics';

/**
 * Keeps the diagnostic collection in step with every open VRL document.
 *
 * VRL compiles in well under a millisecond, so the debounce is here to avoid
 * pointless work while someone is mid-word, not because compilation is slow.
 *
 * When a sample event sits next to the file, the program is typed against it,
 * which is what turns "`.message` might not be a string" into "`.mesage` does
 * not exist". Editing the sample re-checks the program, so the two are always
 * looked at together.
 */
export class DiagnosticRunner implements vscode.Disposable {
  private readonly collection = vscode.languages.createDiagnosticCollection('vrl');
  private readonly pending = new Map<string, NodeJS.Timeout>();
  private readonly disposables: vscode.Disposable[] = [];
  /** Last sample problem reported per document, so it is logged once. */
  private readonly reportedSampleErrors = new Map<string, string>();

  constructor(
    private readonly checker: VrlChecker,
    private readonly samples: SampleStore,
    private readonly status: StatusBar,
    private readonly output: vscode.OutputChannel,
  ) {
    this.disposables.push(
      vscode.workspace.onDidOpenTextDocument((document) => this.schedule(document, 0)),
      vscode.workspace.onDidChangeTextDocument((event) => this.schedule(event.document, DEBOUNCE_MS)),
      vscode.workspace.onDidCloseTextDocument((document) => this.forget(document)),
      // Turning the setting on or off changes what every open file is checked
      // against, so every open file has to be checked again.
      vscode.workspace.onDidChangeConfiguration((event) => {
        if (event.affectsConfiguration(`vrl-tools.${USE_SAMPLE}`)) {
          this.recheckAll();
        }
      }),
    );

    this.recheckAll();
  }

  /** Re-checks the program a sample belongs to, when that sample changes. */
  recheck(program: vscode.Uri): void {
    const document = vscode.workspace.textDocuments.find(
      (open) => open.uri.toString() === program.toString(),
    );
    if (document) {
      this.schedule(document, 0);
    }
  }

  private recheckAll(): void {
    for (const document of vscode.workspace.textDocuments) {
      this.schedule(document, 0);
    }
  }

  private schedule(document: vscode.TextDocument, delay: number): void {
    // A sample being edited changes the answer for the program beside it.
    if (isSample(document.uri)) {
      this.recheck(programFor(document.uri));
      return;
    }

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

    const sample = this.sampleEnabled() ? this.samples.read(document.uri) : undefined;

    let result: Check;
    try {
      result = this.checker.check(document.getText(), sample?.json);
    } catch (error) {
      // A panic inside the wasm module would otherwise leave stale squiggles on
      // screen with no explanation.
      this.output.appendLine(`Checking ${document.uri.fsPath} failed: ${String(error)}`);
      this.collection.delete(document.uri);
      return;
    }

    // A sample that cannot be read is not a small problem: the file is being
    // checked against an unknown event while its author believes otherwise.
    // It goes to the status bar, and to the output channel once per change.
    if (result.sampleError && result.sampleError !== this.reportedSampleErrors.get(document.uri.toString())) {
      this.reportedSampleErrors.set(document.uri.toString(), result.sampleError);
      this.output.appendLine(
        `${document.uri.fsPath}: the sample event was ignored — ${result.sampleError}`,
      );
    } else if (!result.sampleError) {
      this.reportedSampleErrors.delete(document.uri.toString());
    }

    this.status.report(document.uri, {
      typedWithSample: result.typedWithSample,
      sampleError: result.sampleError,
      sampleName: sample ? sample.uri.path.split('/').pop() : undefined,
    });

    this.collection.set(
      document.uri,
      result.diagnostics.map((diagnostic) => toVsCode(diagnostic, document.uri)),
    );
  }

  /**
   * Sample-typed diagnostics can be turned off. They are strictly more
   * informed, but they are also a claim about production data, and someone
   * whose sample has drifted from reality would rather have the pessimistic
   * answer than a confident wrong one.
   */
  private sampleEnabled(): boolean {
    return vscode.workspace.getConfiguration('vrl-tools').get<boolean>(USE_SAMPLE, true);
  }

  private forget(document: vscode.TextDocument): void {
    const key = document.uri.toString();
    const queued = this.pending.get(key);
    if (queued) {
      clearTimeout(queued);
      this.pending.delete(key);
    }
    this.reportedSampleErrors.delete(key);
    this.status.forget(document.uri);
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
