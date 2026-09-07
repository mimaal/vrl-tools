import * as vscode from 'vscode';

import { RUN_COMMAND } from './run';

/** What the last check made of the sample event for a document. */
export interface SampleState {
  readonly typedWithSample: boolean;
  readonly sampleError: string | null;
  readonly sampleName?: string;
}

/**
 * The status bar item.
 *
 * It carries two facts, both of which change what the diagnostics mean:
 *
 * - the `vrl` version being compiled against, because when that disagrees with
 *   the Vector running in production, the editor and the deployment disagree
 *   too, and that is the first thing to check;
 * - whether a sample event was used, because a program checked against a known
 *   event and the same program checked against an unknown one get different
 *   answers, and a person should never have to guess which they are reading.
 */
export class StatusBar implements vscode.Disposable {
  private readonly item = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    100,
  );
  private readonly states = new Map<string, SampleState>();
  private readonly disposables: vscode.Disposable[] = [];

  constructor(private readonly vrlVersion: string) {
    this.item.command = RUN_COMMAND;
    this.disposables.push(
      vscode.window.onDidChangeActiveTextEditor(() => this.refresh()),
      this.item,
    );
    this.refresh();
  }

  /** Records what a check found, and refreshes if it was the visible file. */
  report(uri: vscode.Uri, state: SampleState): void {
    this.states.set(uri.toString(), state);
    if (vscode.window.activeTextEditor?.document.uri.toString() === uri.toString()) {
      this.refresh();
    }
  }

  forget(uri: vscode.Uri): void {
    this.states.delete(uri.toString());
  }

  private refresh(): void {
    const document = vscode.window.activeTextEditor?.document;
    if (document?.languageId !== 'vrl') {
      this.item.hide();
      return;
    }

    const state = this.states.get(document.uri.toString());
    const tooltip = [
      `Diagnostics come from the vrl crate ${this.vrlVersion}, the version Vector 0.52.0 ships.`,
      'A different Vector in production may disagree.',
      '',
    ];

    if (state?.sampleError) {
      this.item.text = `$(warning) VRL ${this.vrlVersion} · sample ignored`;
      tooltip.push(`The sample event could not be used: ${state.sampleError}`);
      tooltip.push('This file is being checked against an event of unknown shape.');
    } else if (state?.typedWithSample) {
      this.item.text = `VRL ${this.vrlVersion} · sample`;
      tooltip.push(
        `Typed against ${state.sampleName ?? 'the sample event'}: fields have the types the sample gives them.`,
      );
      tooltip.push('Click to run this program on it.');
    } else {
      this.item.text = `VRL ${this.vrlVersion}`;
      tooltip.push('No sample event, so `.` is of unknown shape and every field could be anything.');
      tooltip.push('Click to create one and run this program on it.');
    }

    this.item.tooltip = tooltip.join('\n');
    this.item.show();
  }

  dispose(): void {
    for (const disposable of this.disposables) {
      disposable.dispose();
    }
    this.states.clear();
  }
}
