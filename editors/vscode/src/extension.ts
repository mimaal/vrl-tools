import * as vscode from 'vscode';

/**
 * Phase 0/1 activation is deliberately thin: highlighting is contributed
 * declaratively through the TextMate grammar, so nothing here is required to
 * colour a document. The entry point exists so that phase 3 (compiler
 * diagnostics) and the pinned-version status bar item have somewhere to land.
 */
export function activate(context: vscode.ExtensionContext): void {
  const channel = vscode.window.createOutputChannel('VRL Tools');
  context.subscriptions.push(channel);
  channel.appendLine('VRL Tools activated.');
}

export function deactivate(): void {
  // Nothing to tear down yet.
}
