import * as vscode from 'vscode';

import { VrlChecker } from './checker';
import { DiagnosticRunner } from './diagnostics';
import { registerLanguageFeatures } from './language';
import { registerRun } from './run';
import { SampleStore } from './sample';
import { StatusBar } from './status';
import { registerTopology } from './topology';

/**
 * Diagnostics, hover, completion and signature help all come from the real VRL
 * compiler, compiled to WebAssembly. If that module fails to load there is
 * nothing to fall back to — a regex approximation of a type checker, or a
 * hand-written list of functions, would be worse than nothing at all — so the
 * extension says so plainly and leaves highlighting and snippets working.
 */
export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel('VRL Tools');
  context.subscriptions.push(output);

  let checker: VrlChecker;
  try {
    checker = VrlChecker.load(context.extensionPath);
  } catch (error) {
    output.appendLine(`Failed to load the VRL compiler module: ${String(error)}`);
    output.appendLine('Highlighting and snippets still work; diagnostics are unavailable.');
    void vscode.window.showErrorMessage(
      'VRL Tools could not load the VRL compiler. Diagnostics, hover and completion are unavailable — see the VRL Tools output channel.',
    );
    return;
  }

  output.appendLine(`VRL Tools activated, compiling against vrl ${checker.vrlVersion}.`);

  const status = new StatusBar(checker.vrlVersion, checker.vectorRelease);
  context.subscriptions.push(status);

  // The store is created before the runner and told about it afterwards: a
  // sample changing has to re-check the program next to it, and the runner is
  // what knows how to do that.
  let runner: DiagnosticRunner | undefined;
  const samples = new SampleStore((program) => runner?.recheck(program));
  context.subscriptions.push(samples);

  runner = new DiagnosticRunner(checker, samples, status, output);
  context.subscriptions.push(runner);

  context.subscriptions.push(...registerLanguageFeatures(checker, output));
  context.subscriptions.push(...registerRun(checker, samples, output));

  // Vector configs, not .vrl files: the graph command is the one thing here
  // that works on a YAML or TOML document, which is why it activates by being
  // invoked rather than by a language.
  context.subscriptions.push(...registerTopology(checker, output));
}

export function deactivate(): void {
  // The disposables registered above are enough.
}
