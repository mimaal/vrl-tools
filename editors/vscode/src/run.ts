import * as vscode from 'vscode';

import type { Run, VrlChecker } from './checker';
import type { SampleStore } from './sample';

/**
 * `VRL: Run on sample event` — the command that turns the extension from a
 * checker into somewhere to work.
 *
 * The program runs against the sample event next to it and the result opens
 * beside the source: the event as Vector would emit it, with what happened
 * written in comments above. It is the vector.dev playground, except local,
 * against your own data, and on the exact compiler version your Vector runs.
 *
 * The output is a plain read-only document rather than a webview. It can be
 * diffed against the input, copied, and read by every VS Code feature there
 * is, which no amount of custom HTML would give.
 */
export const RUN_COMMAND = 'vrl-tools.runOnSampleEvent';

/** URI scheme of the result documents. */
const SCHEME = 'vrl-run';

export function registerRun(
  checker: VrlChecker,
  samples: SampleStore,
  output: vscode.OutputChannel,
): vscode.Disposable[] {
  const results = new ResultDocuments();

  return [
    results,
    vscode.workspace.registerTextDocumentContentProvider(SCHEME, results),
    vscode.commands.registerCommand(RUN_COMMAND, async () => {
      const editor = vscode.window.activeTextEditor;
      if (editor?.document.languageId !== 'vrl') {
        void vscode.window.showWarningMessage('Open a VRL file to run it on a sample event.');
        return;
      }

      const document = editor.document;
      const sample = samples.read(document.uri);
      if (!sample) {
        await offerToCreateSample(samples, document.uri);
        return;
      }

      const result = checker.run(document.getText(), sample.json);
      output.appendLine(
        `Ran ${basename(document.uri)} against ${basename(sample.uri)}: ` +
          `${result.compiled ? 'compiled' : 'did not compile'}` +
          `${result.aborted ? ', aborted' : ''}${result.error ? `, error: ${result.error}` : ''}`,
      );

      if (result.sampleError) {
        void vscode.window.showErrorMessage(`${basename(sample.uri)}: ${result.sampleError}`);
        return;
      }

      if (!result.compiled) {
        void vscode.window.showErrorMessage(
          'The program does not compile, so it was not run. The errors are in the editor.',
        );
        return;
      }

      const uri = vscode.Uri.parse(`${SCHEME}:${document.uri.path}.output.jsonc`);
      results.set(uri, render(result, document.uri, sample.uri, checker.vrlVersion));

      const shown = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(shown, {
        viewColumn: vscode.ViewColumn.Beside,
        preview: true,
        preserveFocus: true,
      });

      if (result.aborted) {
        void vscode.window.showInformationMessage(
          `The program aborted: ${result.error ?? 'no message'}. Vector would drop this event.`,
        );
      } else if (result.error) {
        void vscode.window.showErrorMessage(`The program failed at runtime: ${result.error}`);
      }
    }),
  ];
}

/**
 * Offers to create the sample file, since "there is no sample" is nearly
 * always "I have not written one yet" rather than a mistake.
 */
async function offerToCreateSample(samples: SampleStore, program: vscode.Uri): Promise<void> {
  const uri = samples.uriFor(program);
  const create = 'Create it';

  const answer = await vscode.window.showInformationMessage(
    `No sample event for this file. It goes in ${basename(uri)}, next to the program.`,
    create,
  );
  if (answer !== create) {
    return;
  }

  const starter = `{\n  "message": "replace this with one of your events"\n}\n`;
  await vscode.workspace.fs.writeFile(uri, Buffer.from(starter, 'utf8'));

  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document, { viewColumn: vscode.ViewColumn.Beside });
}

/**
 * The result as a JSON-with-comments document.
 *
 * The body is the event and nothing else, so it can be diffed against the
 * sample. Everything else — what the program returned, what it did to the
 * metadata, how it stopped — goes in comments above, where it is visible
 * without getting in the way of the comparison.
 */
function render(
  result: Run,
  program: vscode.Uri,
  sample: vscode.Uri,
  vrlVersion: string,
): string {
  const header = [
    `// ${basename(program)} on ${basename(sample)}, vrl ${vrlVersion}`,
  ];

  if (result.aborted) {
    header.push(`// ABORTED: ${result.error ?? 'no message'}`);
    header.push('// Vector would drop this event. Below is how far the program got.');
  } else if (result.error) {
    header.push(`// RUNTIME ERROR: ${result.error}`);
    header.push('// Below is the event as it stood when the program stopped.');
  }

  if (result.output !== undefined && result.output !== null) {
    // What a `filter` condition is judged on, and what a `remap` transform
    // ignores. Worth showing either way.
    header.push(`// The program returned: ${JSON.stringify(result.output)}`);
  }

  const metadata = result.metadata;
  if (metadata && Object.keys(metadata as object).length > 0) {
    header.push(`// Metadata: ${JSON.stringify(metadata)}`);
  }

  const warnings = result.diagnostics.filter((d) => d.severity === 'warning');
  for (const warning of warnings) {
    header.push(`// Warning on line ${warning.range.start.line + 1}: ${warning.message}`);
  }

  const event = result.event === undefined ? null : result.event;
  return `${header.join('\n')}\n${JSON.stringify(event, null, 2)}\n`;
}

function basename(uri: vscode.Uri): string {
  return uri.path.split('/').pop() ?? uri.path;
}

/**
 * Holds the rendered results and tells VS Code when one is replaced, so that
 * running again refreshes the open panel instead of piling up documents.
 */
class ResultDocuments implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly contents = new Map<string, string>();
  private readonly emitter = new vscode.EventEmitter<vscode.Uri>();

  readonly onDidChange = this.emitter.event;

  set(uri: vscode.Uri, content: string): void {
    this.contents.set(uri.toString(), content);
    this.emitter.fire(uri);
  }

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.contents.get(uri.toString()) ?? '// This result is no longer available.\n';
  }

  dispose(): void {
    this.emitter.dispose();
    this.contents.clear();
  }
}
