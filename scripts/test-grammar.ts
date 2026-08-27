/**
 * Tokenises VRL through the same engine VS Code uses (vscode-textmate over
 * Oniguruma) and asserts the scopes the phase 1 table calls for, plus the four
 * documented failure modes.
 *
 * Run with: npm run test:grammar
 */

import { createRequire } from 'node:module';
import { readdir, readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as oniguruma from 'vscode-oniguruma';
import * as textmate from 'vscode-textmate';

const require = createRequire(import.meta.url);
const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const GRAMMAR = path.join(ROOT, 'editors/vscode/syntaxes/vrl.tmLanguage.json');
const CORPUS = path.join(ROOT, 'test-corpus');

interface Case {
  /** What the assertion is about, printed on failure. */
  readonly what: string;
  readonly source: string;
  /** Substring of the line whose token is inspected. */
  readonly token: string;
  /** Scope that must be present on that token. */
  readonly expect?: string;
  /** Scope that must NOT be present on that token. */
  readonly reject?: string;
  /** Which occurrence of `token` to inspect, 0-based. */
  readonly nth?: number;
}

const CASES: Case[] = [
  // --- the construct table ---------------------------------------------
  { what: 'event path', source: '.foo.bar = 1', token: 'foo', expect: 'variable.other.property.vrl' },
  { what: 'root path alone', source: '. = {}', token: '.', expect: 'variable.language.vrl' },
  { what: 'metadata path', source: 'x = %vector.ingest_timestamp', token: 'vector', expect: 'variable.other.metadata.vrl' },
  { what: 'path coalescence operator', source: 'x = .foo.(a | b)', token: '|', expect: 'keyword.operator.coalesce.path.vrl' },
  { what: 'path coalescence member', source: 'x = .foo.(a | b)', token: 'a', expect: 'variable.other.property.vrl' },
  { what: 'indexed path', source: 'x = .list[0]', token: '0', expect: 'constant.numeric.integer.vrl' },
  { what: 'fallible call name', source: 'x = parse_json!(.message)', token: 'parse_json', expect: 'support.function.vrl' },
  { what: 'fallible call bang', source: 'x = parse_json!(.message)', token: '!', expect: 'keyword.operator.fallible.vrl' },
  { what: 'error destructuring binding', source: 'x, err = parse_json(.message)', token: 'err', expect: 'variable.other.error.vrl' },
  { what: 'destructuring comma', source: 'x, err = parse_json(.message)', token: ',', expect: 'punctuation.separator.destructuring.vrl' },
  { what: 'null-coalescing operator', source: 'x = .a ?? .b ?? "d"', token: '??', expect: 'keyword.operator.coalesce.vrl' },
  { what: 'double-quoted string', source: 'x = "texto \\n"', token: 'texto', expect: 'string.quoted.double.vrl' },
  { what: 'string escape', source: 'x = "texto \\n"', token: '\\n', expect: 'constant.character.escape.vrl' },
  { what: 'raw string', source: "x = s'sin escapes'", token: 'sin', expect: 'string.quoted.single.raw.vrl' },
  { what: 'regex literal', source: "x = r'^\\d+$'", token: '+', expect: 'string.regexp.vrl' },
  { what: 'timestamp literal', source: "x = t'2021-01-01T00:00:00Z'", token: '2021', expect: 'constant.other.timestamp.vrl' },
  { what: 'template interpolation', source: 'x = "id {{ .field }}"', token: '{{', expect: 'meta.embedded.line.vrl' },
  { what: 'path inside interpolation', source: 'x = "id {{ .field }}"', token: 'field', expect: 'variable.other.property.vrl' },
  { what: 'comment', source: '# just a note', token: 'just', expect: 'comment.line.number-sign.vrl' },
  { what: 'keyword if', source: 'if .a { }', token: 'if', expect: 'keyword.control.vrl' },
  { what: 'keyword abort', source: 'abort', token: 'abort', expect: 'keyword.control.vrl' },
  { what: 'boolean literal', source: 'x = true', token: 'true', expect: 'constant.language.vrl' },
  { what: 'null literal', source: 'x = null', token: 'null', expect: 'constant.language.vrl' },
  { what: 'closure parameter', source: 'for_each(.obj) -> |k, v| { }', token: 'k', expect: 'variable.parameter.vrl' },
  { what: 'closure second parameter', source: 'for_each(.obj) -> |k, v| { }', token: 'v', expect: 'variable.parameter.vrl' },
  { what: 'integer', source: 'x = 42', token: '42', expect: 'constant.numeric.integer.vrl' },
  { what: 'float', source: 'x = 3.14', token: '3.14', expect: 'constant.numeric.float.vrl' },
  { what: 'negative number', source: 'x = (-1)', token: '-1', expect: 'constant.numeric.integer.vrl' },
  { what: 'stdlib function', source: 'x = to_int(.n)', token: 'to_int', expect: 'support.function.vrl' },
  { what: 'stdlib del', source: 'del(.foo)', token: 'del', expect: 'support.function.vrl' },
  { what: 'stdlib exists', source: 'x = exists(.foo)', token: 'exists', expect: 'support.function.vrl' },
  { what: 'member access on a variable', source: 'x = fields.client', token: 'client', expect: 'variable.other.property.vrl' },
  { what: 'member access dot is an accessor', source: 'x = fields.client', token: '.', expect: 'punctuation.accessor.vrl' },

  // --- the four documented failure modes --------------------------------
  {
    what: 'PITFALL 1: the dot of 3.14 must not be read as a path',
    source: 'x = 3.14',
    token: '3.14',
    reject: 'variable.other.property.vrl',
  },
  {
    what: 'PITFALL 1b: the dot of 3.14 is not a root path either',
    source: 'x = 3.14',
    token: '3.14',
    reject: 'variable.language.vrl',
  },
  {
    what: 'PITFALL 2: negation ! is not the fallibility !',
    source: 'if !exists(.x) { }',
    token: '!',
    expect: 'keyword.operator.logical.vrl',
  },
  {
    what: 'PITFALL 2b: negation ! must not be scoped as fallible',
    source: 'if !exists(.x) { }',
    token: '!',
    reject: 'keyword.operator.fallible.vrl',
  },
  {
    what: 'PITFALL 3: regex is not a plain string',
    source: "x = match(.m, r'^\\d+$')",
    token: '^',
    reject: 'string.quoted.double.vrl',
  },
  {
    what: 'PITFALL 3b: regex escapes are scoped as escapes',
    source: "x = match(.m, r'^\\d+$')",
    token: '\\d',
    expect: 'constant.character.escape.regexp.vrl',
  },
  {
    what: 'PITFALL 3c: code after a regex is no longer inside the regex',
    source: "x = match(.m, r'^\\d+$') && true",
    token: 'true',
    reject: 'string.regexp.vrl',
  },
  {
    what: 'PITFALL 4: quoted path segment (ECS style)',
    source: 'x = ."@timestamp"',
    token: '@timestamp',
    expect: 'variable.other.property.quoted.vrl',
  },
  {
    what: 'PITFALL 4b: bracketed quoted path segment',
    source: 'x = .["a-b"]',
    token: 'a-b',
    expect: 'variable.other.property.quoted.vrl',
  },
  {
    what: 'PITFALL 4c: a quoted path segment is not a free-standing string',
    source: 'x = ."@timestamp"',
    token: '@timestamp',
    reject: 'string.quoted.double.vrl',
  },
];

async function loadGrammar(): Promise<textmate.IGrammar> {
  const wasm = await readFile(require.resolve('vscode-oniguruma/release/onig.wasm'));
  await oniguruma.loadWASM(wasm.buffer as ArrayBuffer);

  const registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
      createOnigString: (str) => new oniguruma.OnigString(str),
    }),
    loadGrammar: async (scopeName) => {
      if (scopeName !== 'source.vrl') {
        return null;
      }
      const raw = await readFile(GRAMMAR, 'utf8');
      return textmate.parseRawGrammar(raw, GRAMMAR);
    },
  });

  const grammar = await registry.loadGrammar('source.vrl');
  if (!grammar) {
    throw new Error('source.vrl grammar failed to load');
  }
  return grammar;
}

/** Scopes covering the character range of the nth occurrence of `token`. */
function scopesAt(grammar: textmate.IGrammar, line: string, token: string, nth: number): string[] {
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
 * Tokenises a whole file and reports lines that finish with grammar state still
 * pushed, which is what "the highlighting goes dark from here on" looks like.
 */
function findLeakingState(grammar: textmate.IGrammar, source: string): number[] {
  let stack = textmate.INITIAL;
  const leaks: number[] = [];
  const lines = source.split(/\r?\n/);

  lines.forEach((line, i) => {
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
function findUnscopedText(grammar: textmate.IGrammar, source: string): string[] {
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

async function main(): Promise<void> {
  const grammar = await loadGrammar();
  let failed = 0;

  for (const c of CASES) {
    const nth = c.nth ?? 0;
    let scopes: string[];
    try {
      scopes = scopesAt(grammar, c.source, c.token, nth);
    } catch (error) {
      console.error(`FAIL  ${c.what}\n      ${String(error)}`);
      failed++;
      continue;
    }

    if (c.expect && !scopes.includes(c.expect)) {
      console.error(
        `FAIL  ${c.what}\n      source:   ${c.source}\n      token:    ${c.token}\n` +
          `      expected: ${c.expect}\n      actual:   ${scopes.join(', ')}`,
      );
      failed++;
      continue;
    }

    if (c.reject && scopes.includes(c.reject)) {
      console.error(
        `FAIL  ${c.what}\n      source:   ${c.source}\n      token:    ${c.token}\n` +
          `      must not have: ${c.reject}\n      actual:        ${scopes.join(', ')}`,
      );
      failed++;
      continue;
    }

    console.log(`ok    ${c.what}`);
  }

  // Whole-file pass over the corpus: nothing may "go dark".
  let corpusFiles: string[] = [];
  try {
    corpusFiles = (await readdir(CORPUS)).filter((f) => f.endsWith('.vrl'));
  } catch {
    corpusFiles = [];
  }

  if (corpusFiles.length === 0) {
    console.error('\nFAIL  test-corpus/ has no .vrl files to tokenise');
    failed++;
  }

  for (const file of corpusFiles) {
    const source = await readFile(path.join(CORPUS, file), 'utf8');
    const leaks = findLeakingState(grammar, source);
    if (leaks.length > 0) {
      console.error(`FAIL  ${file}: grammar state leaks past lines ${leaks.join(', ')}`);
      failed++;
    } else {
      console.log(`ok    corpus ${file} tokenises with no leaking state`);
    }

    const dark = findUnscopedText(grammar, source);
    if (dark.length > 0) {
      console.error(
        `FAIL  ${file}: ${dark.length} pieces of text carry no scope at all\n` +
          dark.map((d) => `      ${d}`).join('\n'),
      );
      failed++;
    } else {
      console.log(`ok    corpus ${file} has no unscoped text`);
    }
  }

  console.log(`\n${CASES.length + corpusFiles.length * 2} checks, ${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((error: unknown) => {
  console.error(String(error));
  process.exit(1);
});
