import * as vscode from 'vscode';

import {
  completionContextAt,
  enclosingCall,
  identifierAt,
  isInCommentOrString,
  pathsIn,
} from './analysis';
import type { StdlibFunction, VrlChecker } from './checker';

/**
 * Hover, completion and signature help.
 *
 * Everything shown here is the compiler's own description of the standard
 * library, read out of the same wasm module that produces the diagnostics.
 * There is no table of function names in this file, and there must never be
 * one: a hand-written list is wrong the day the pinned version moves.
 *
 * No link out to the VRL website either. The crate carries a summary, the long
 * usage text, the parameters and runnable examples for every function, which
 * is more than a link would give and is exactly the version being compiled
 * against.
 */
export function registerLanguageFeatures(
  checker: VrlChecker,
  output: vscode.OutputChannel,
): vscode.Disposable[] {
  const stdlib = checker.stdlib();
  const functions = new Map(stdlib.functions.map((f) => [f.name, f]));

  output.appendLine(
    `Loaded ${stdlib.functions.length} standard library functions from vrl ${stdlib.vrlVersion}.`,
  );

  const selector: vscode.DocumentSelector = { language: 'vrl' };

  return [
    vscode.languages.registerHoverProvider(selector, new VrlHoverProvider(functions)),
    vscode.languages.registerCompletionItemProvider(
      selector,
      new VrlCompletionProvider(stdlib.functions),
      '.',
      '%',
    ),
    vscode.languages.registerSignatureHelpProvider(
      selector,
      new VrlSignatureHelpProvider(functions),
      '(',
      ',',
    ),
  ];
}

class VrlHoverProvider implements vscode.HoverProvider {
  constructor(private readonly functions: ReadonlyMap<string, StdlibFunction>) {}

  provideHover(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): vscode.Hover | undefined {
    const line = document.lineAt(position.line).text;
    if (isInCommentOrString(line, position.character)) {
      return undefined;
    }

    const word = identifierAt(line, position.character);
    const fn = word && this.functions.get(word.text);
    if (!word || !fn) {
      return undefined;
    }

    return new vscode.Hover(
      describe(fn, { long: true }),
      new vscode.Range(position.line, word.start, position.line, word.end),
    );
  }
}

class VrlCompletionProvider implements vscode.CompletionItemProvider {
  /**
   * The function list never changes — it is the standard library of a pinned
   * compiler — so the items are built once rather than on every keystroke.
   * VS Code does the filtering.
   */
  private readonly functionItems: vscode.CompletionItem[];

  constructor(functions: readonly StdlibFunction[]) {
    this.functionItems = functions.map(functionCompletion);
  }

  provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): vscode.CompletionItem[] {
    const line = document.lineAt(position.line).text;
    const context = completionContextAt(line, position.character);

    if (context.kind === 'none') {
      return [];
    }

    if (context.kind === 'path') {
      const range = new vscode.Range(
        position.line,
        context.path.start,
        position.line,
        context.path.end,
      );
      return this.paths(document, context.path.text, range);
    }

    return this.functionItems;
  }

  /**
   * The paths this file already uses.
   *
   * Not a schema — nobody has told the extension what the events look like —
   * but the fields already mentioned are the ones about to be mentioned again,
   * and a suggested `.hostname` is one fewer chance to type `.host_name` and
   * silently drop the data.
   */
  private paths(
    document: vscode.TextDocument,
    typed: string,
    range: vscode.Range,
  ): vscode.CompletionItem[] {
    const sigil = typed[0];

    return pathsIn(document.getText())
      .filter((path) => path.startsWith(sigil) && path !== typed)
      .map((path) => {
        const item = new vscode.CompletionItem(path, vscode.CompletionItemKind.Field);
        item.detail = 'used in this file';
        item.range = range;
        item.insertText = path;
        // Paths sort under functions: when both could apply, the name being
        // typed is far more often a function.
        item.sortText = `z${path}`;
        return item;
      });
  }
}

class VrlSignatureHelpProvider implements vscode.SignatureHelpProvider {
  constructor(private readonly functions: ReadonlyMap<string, StdlibFunction>) {}

  provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): vscode.SignatureHelp | undefined {
    const call = enclosingCall(document.getText(), document.offsetAt(position));
    const fn = call && this.functions.get(call.name);
    if (!call || !fn) {
      return undefined;
    }

    const signature = new vscode.SignatureInformation(
      signatureLabel(fn),
      describe(fn, { long: false }),
    );
    signature.parameters = fn.parameters.map(
      (parameter) =>
        new vscode.ParameterInformation(
          parameterLabel(parameter),
          new vscode.MarkdownString(
            `\`${parameter.keyword}\`: ${parameter.kind}${parameter.required ? '' : ', optional'}`,
          ),
        ),
    );

    // A keyword argument names its parameter outright; otherwise position
    // decides, which is also how the compiler reads it.
    const byKeyword = call.keyword
      ? fn.parameters.findIndex((p) => p.keyword === call.keyword)
      : -1;

    const help = new vscode.SignatureHelp();
    help.signatures = [signature];
    help.activeSignature = 0;
    help.activeParameter =
      byKeyword >= 0 ? byKeyword : Math.min(call.argumentIndex, Math.max(fn.parameters.length - 1, 0));

    return help;
  }
}

function functionCompletion(fn: StdlibFunction): vscode.CompletionItem {
  const item = new vscode.CompletionItem(fn.name, vscode.CompletionItemKind.Function);

  item.detail = `${signatureLabel(fn)}${fn.returns ? ` → ${fn.returns}` : ''}`;
  item.documentation = describe(fn, { long: false });
  item.insertText = callSnippet(fn);

  // Fallibility is the thing people forget, so it is on the list itself rather
  // than only in the documentation panel. The `!` is deliberately not inserted
  // for you: suppressing the error is a choice, and the compiler will point at
  // the line either way.
  if (fn.fallible === true) {
    item.label = { label: fn.name, description: 'fallible' };
  }

  return item;
}

/**
 * `name(value, format)` with the required parameters as tabstops, plus the
 * closure when the function takes one.
 */
function callSnippet(fn: StdlibFunction): vscode.SnippetString {
  const snippet = new vscode.SnippetString(`${fn.name}(`);

  fn.parameters
    .filter((parameter) => parameter.required)
    .forEach((parameter, i) => {
      if (i > 0) {
        snippet.appendText(', ');
      }
      snippet.appendPlaceholder(parameter.keyword);
    });

  snippet.appendText(')');

  if (fn.closure) {
    snippet.appendText(' -> |');
    fn.closure.variables.forEach((variable, i) => {
      if (i > 0) {
        snippet.appendText(', ');
      }
      snippet.appendPlaceholder(variable);
    });
    snippet.appendText('| {\n\t');
    snippet.appendTabstop(0);
    snippet.appendText('\n}');
  }

  return snippet;
}

function signatureLabel(fn: StdlibFunction): string {
  const parameters = fn.parameters.map(parameterLabel).join(', ');
  const closure = fn.closure ? ` -> |${fn.closure.variables.join(', ')}| { … }` : '';

  return `${fn.name}(${parameters})${closure}`;
}

function parameterLabel(parameter: { keyword: string; kind: string; required: boolean }): string {
  return `${parameter.keyword}: ${parameter.kind}${parameter.required ? '' : '?'}`;
}

/**
 * The markdown shown in a hover, a completion panel or a signature popup.
 *
 * `long` adds the crate's multi-paragraph usage text and an example, which is
 * welcome on hover and far too much in a completion list.
 */
function describe(fn: StdlibFunction, { long }: { long: boolean }): vscode.MarkdownString {
  const md = new vscode.MarkdownString();
  md.appendCodeblock(signatureLabel(fn), 'vrl');

  if (fn.summary) {
    md.appendMarkdown(`\n${fn.summary}\n`);
  }

  const facts: string[] = [];
  if (fn.returns) {
    facts.push(`**Returns** \`${fn.returns}\``);
  }
  if (fn.fallible === true) {
    facts.push('**Fallible** — handle the error, coalesce it with `??`, or assert with `!`');
  } else if (fn.fallible === false) {
    facts.push('**Infallible** — no error to handle');
  }
  if (facts.length > 0) {
    md.appendMarkdown(`\n${facts.join('  \n')}\n`);
  }

  if (!long) {
    // With no prose to show, an example is the only thing that says what the
    // function is for, so the short form gets one too.
    const first = fn.examples[0];
    if (!fn.summary && first) {
      md.appendCodeblock(first.source.trim(), 'vrl');
    }
    return md;
  }

  if (fn.usage && fn.usage !== fn.summary) {
    md.appendMarkdown(`\n${fn.usage}\n`);
  }

  if (fn.parameters.length > 0) {
    md.appendMarkdown('\n**Parameters**\n');
    for (const parameter of fn.parameters) {
      md.appendMarkdown(
        `- \`${parameter.keyword}\`: ${parameter.kind}${parameter.required ? '' : ' *(optional)*'}\n`,
      );
    }
  }

  // The crate carries prose for nine functions out of nearly two hundred, so
  // on hover the examples are the documentation. Show several.
  for (const example of fn.examples.slice(0, 3)) {
    md.appendMarkdown(`\n**${example.title}**\n`);
    md.appendCodeblock(
      'Ok' in example.result
        ? `${example.source.trim()}\n# => ${example.result.Ok}`
        : example.source.trim(),
      'vrl',
    );
  }

  return md;
}
