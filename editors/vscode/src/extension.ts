import * as vscode from 'vscode';

import { VrlChecker } from './checker';
import { DiagnosticRunner } from './diagnostics';

/**
 * Diagnostics come from the real VRL compiler, compiled to WebAssembly. If that
 * module fails to load there is nothing to fall back to — a regex approximation
 * of a type checker would be worse than no diagnostics at all — so the
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
      'VRL Tools could not load the VRL compiler. Diagnostics are unavailable — see the VRL Tools output channel.',
    );
    return;
  }

  output.appendLine(`VRL Tools activated, compiling against vrl ${checker.vrlVersion}.`);

  context.subscriptions.push(new DiagnosticRunner(checker, output));
  context.subscriptions.push(createStatusBarItem(checker.vrlVersion));
}

/**
 * The pinned VRL version is on screen at all times on purpose: when this
 * disagrees with the Vector running in production, the editor and the
 * deployment disagree too, and that is the first thing to check.
 */
function createStatusBarItem(vrlVersion: string): vscode.StatusBarItem {
  const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  item.text = `VRL ${vrlVersion}`;
  item.tooltip = `Diagnostics come from the vrl crate ${vrlVersion}, the version Vector 0.52.0 ships. A different Vector in production may disagree.`;

  const update = (editor: vscode.TextEditor | undefined): void => {
    if (editor?.document.languageId === 'vrl') {
      item.show();
    } else {
      item.hide();
    }
  };

  vscode.window.onDidChangeActiveTextEditor(update);
  update(vscode.window.activeTextEditor);

  return item;
}

export function deactivate(): void {
  // The disposables registered above are enough.
}
