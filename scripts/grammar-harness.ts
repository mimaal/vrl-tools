/**
 * Shared plumbing for the grammar tests: a vscode-textmate registry backed by
 * the same Oniguruma engine VS Code uses, plus the assertions both the plain
 * grammar test and the injection test need.
 *
 * Keeping this in one place means the tests exercise exactly one loading path,
 * so a grammar that fails to parse fails the same way everywhere.
 */

import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as oniguruma from 'vscode-oniguruma';
import * as textmate from 'vscode-textmate';

const require = createRequire(import.meta.url);
const HERE = path.dirname(fileURLToPath(import.meta.url));

export const ROOT = path.resolve(HERE, '..');
export const CORPUS = path.join(ROOT, 'test-corpus');

const SYNTAXES = path.join(ROOT, 'editors/vscode/syntaxes');

/** Every scope this repository ships, and the file that defines it. */
const GRAMMARS: Readonly<Record<string, string>> = {
  'source.vrl': path.join(SYNTAXES, 'vrl.tmLanguage.json'),
  'vrl.injection.yaml': path.join(SYNTAXES, 'vrl-injection-yaml.json'),
  'vrl.injection.toml': path.join(SYNTAXES, 'vrl-injection-toml.json'),
};

let wasmLoaded: Promise<void> | undefined;

function loadOniguruma(): Promise<void> {
  wasmLoaded ??= (async () => {
    const wasm = await readFile(require.resolve('vscode-oniguruma/release/onig.wasm'));
    await oniguruma.loadWASM(wasm.buffer as ArrayBuffer);
  })();
  return wasmLoaded;
}

export async function loadGrammar(scopeName: string): Promise<textmate.IGrammar> {
  await loadOniguruma();

  const registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
      createOnigString: (str) => new oniguruma.OnigString(str),
    }),
    loadGrammar: async (scope) => {
      const file = GRAMMARS[scope];
      if (!file) {
        return null;
      }
      return textmate.parseRawGrammar(await readFile(file, 'utf8'), file);
    },
  });

  const grammar = await registry.loadGrammar(scopeName);
  if (!grammar) {
    throw new Error(`${scopeName} grammar failed to load`);
  }
  return grammar;
}

/** Scopes covering the character range of the nth occurrence of `token`. */
export function scopesAt(
  grammar: textmate.IGrammar,
  line: string,
  token: string,
  nth = 0,
): string[] {
  let index = -1;
  for (let i = 0; i <= nth; i++) {
    index = line.indexOf(token, index + 1);
    if (index < 0) {
      throw new Error(`token ${JSON.stringify(token)} #${nth} not present in ${JSON.stringify(line)}`);
    }
  }

  const result = grammar.tokenizeLine(line, textmate.INITIAL);
  const scopes = new Set<string>();
  for (const t of result.tokens) {
    const overlaps = t.startIndex < index + token.length && t.endIndex > index;
    if (overlaps) {
      for (const s of t.scopes) {
        scopes.add(s);
      }
    }
  }
  return [...scopes];
}

/**
 * Scopes on the nth occurrence of `token` when the whole document is
 * tokenised line by line, which is the only way to see a construct that
 * depends on grammar state pushed by an earlier line — a YAML block scalar,
 * say.
 */
export function scopesInDocument(
  grammar: textmate.IGrammar,
  source: string,
  token: string,
  nth = 0,
): string[] {
  let stack = textmate.INITIAL;
  let seen = 0;

  for (const line of source.split(/\r?\n/)) {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;

    let index = line.indexOf(token);
    while (index >= 0) {
      if (seen === nth) {
        const scopes = new Set<string>();
        for (const t of result.tokens) {
          if (t.startIndex < index + token.length && t.endIndex > index) {
            for (const s of t.scopes) {
              scopes.add(s);
            }
          }
        }
        return [...scopes];
      }
      seen++;
      index = line.indexOf(token, index + 1);
    }
  }

  throw new Error(`token ${JSON.stringify(token)} #${nth} not present in the document`);
}

/**
 * Tokenises a whole file and reports lines that finish with grammar state still
 * pushed, which is what "the highlighting goes dark from here on" looks like.
 */
export function findLeakingState(grammar: textmate.IGrammar, source: string): number[] {
  let stack = textmate.INITIAL;
  const leaks: number[] = [];

  source.split(/\r?\n/).forEach((line, i) => {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;
    // Every construct in VRL closes on its own line; nothing spans lines.
    if (stack !== textmate.INITIAL && stack.depth > 1) {
      leaks.push(i + 1);
    }
  });

  return leaks;
}

/**
 * Non-whitespace text that came out carrying only the root scope. That is
 * exactly what "the highlighting goes dark here" looks like in an editor:
 * the text renders in the default foreground because no theme rule matches.
 */
export function findUnscopedText(grammar: textmate.IGrammar, source: string): string[] {
  let stack = textmate.INITIAL;
  const dark: string[] = [];

  source.split(/\r?\n/).forEach((line, i) => {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;
    for (const t of result.tokens) {
      const text = line.slice(t.startIndex, t.endIndex);
      if (text.trim() !== '' && t.scopes.length === 1) {
        dark.push(`line ${i + 1}: ${JSON.stringify(text)}`);
      }
    }
  });

  return dark;
}
