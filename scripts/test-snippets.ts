/**
 * Checks the snippets in editors/vscode/snippets/vrl.json.
 *
 * Two things are verified. First, that each body is a well-formed VS Code
 * snippet: tabstops parse, mirrors point at a tabstop that exists, and no stray
 * `$` survives expansion. Second, that the expanded body tokenises cleanly as
 * VRL, which is what catches an unbalanced quote or a raw string that never
 * closes — the failure mode that makes a snippet paint the rest of the file
 * one colour.
 *
 * And third, since phase 3, that the expansion actually compiles: the same
 * bodies go through vrl-check-core by way of the wasm module the extension
 * ships. A snippet that tokenises beautifully and then makes the compiler
 * complain the moment it lands in the buffer is worse than no snippet.
 *
 * Run with: npm run test:snippets
 */

import { readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { errorsIn, vrlVersion } from './checker-harness.js';
import { findLeakingState, findUnscopedText, loadGrammar, ROOT } from './grammar-harness.js';

const SNIPPETS = path.join(ROOT, 'editors/vscode/snippets/vrl.json');

interface Snippet {
  readonly prefix: string;
  readonly description?: string;
  readonly body: string[] | string;
}

/**
 * What a tabstop with no default of its own expands to.
 *
 * Empty is what the editor shows before anyone types, so it is the honest
 * choice for the tokenising pass. For the compile pass it has to be *some*
 * expression, and `null` is the one that neither invents a field nor changes
 * what the surrounding code does.
 */
const AS_TYPED = '';
const AS_COMPILED = 'null';

/**
 * Every piece of snippet syntax we use, in one alternation so that a single
 * left-to-right pass sees them in source order. That ordering is what lets a
 * mirror resolve to the default of the tabstop it mirrors.
 */
const SYNTAX = /\\\$|\$\{(\d+)\|([^}]*)\|\}|\$\{(\d+):([^}]*)\}|\$\{(\d+)\}|\$(\d+)/g;

/**
 * Expands a snippet body the way VS Code would with every default accepted:
 * `${1:foo}` becomes `foo`, `${2|a,b|}` becomes `a`, and a mirror `$1` becomes
 * whatever tabstop 1 was defined as.
 */
function expand(body: string, fill: string): { text: string; problems: string[] } {
  const problems: string[] = [];
  const defaults = new Map<string, string>();

  const text = body.replace(
    SYNTAX,
    (
      match: string,
      choiceIndex: string | undefined,
      choices: string | undefined,
      defaultIndex: string | undefined,
      value: string | undefined,
      emptyIndex: string | undefined,
      mirrorIndex: string | undefined,
    ): string => {
      if (match === '\\$') {
        return '$';
      }
      if (choiceIndex !== undefined) {
        const first = (choices ?? '').split(',')[0] || fill;
        defaults.set(choiceIndex, first);
        return first;
      }
      if (defaultIndex !== undefined) {
        defaults.set(defaultIndex, value ?? '');
        return value ?? '';
      }
      if (emptyIndex !== undefined) {
        const known = defaults.get(emptyIndex) ?? fill;
        defaults.set(emptyIndex, known);
        return known;
      }

      // A bare $n is a tabstop with no default the first time it appears, and
      // a mirror of that tabstop afterwards. Either way it expands to the fill
      // unless a default was declared earlier.
      const index = mirrorIndex as string;
      const known = defaults.get(index) ?? fill;
      defaults.set(index, known);
      return known;
    },
  );

  if (/(?<!\\)\$/.test(text)) {
    problems.push('an unescaped $ survives expansion; write \\$ for a literal dollar');
  }

  return { text, problems };
}

async function main(): Promise<void> {
  // Loads the wasm module up front, so a missing build fails here with one
  // clear message instead of on the first snippet.
  console.log(`compiling snippets against vrl ${vrlVersion()}\n`);

  const grammar = await loadGrammar('source.vrl');
  const raw = JSON.parse(await readFile(SNIPPETS, 'utf8')) as Record<string, Snippet>;
  const entries = Object.entries(raw);

  let failed = 0;
  const prefixes = new Set<string>();

  if (entries.length === 0) {
    console.error('FAIL  snippets/vrl.json defines no snippets');
    process.exit(1);
  }

  for (const [name, snippet] of entries) {
    const fail = (message: string): void => {
      console.error(`FAIL  ${name}: ${message}`);
      failed++;
    };

    if (!snippet.prefix || !snippet.description) {
      fail('needs both a prefix and a description');
      continue;
    }
    if (prefixes.has(snippet.prefix)) {
      fail(`prefix ${JSON.stringify(snippet.prefix)} is already taken`);
      continue;
    }
    prefixes.add(snippet.prefix);

    const body = Array.isArray(snippet.body) ? snippet.body.join('\n') : snippet.body;
    const { text, problems } = expand(body, AS_TYPED);
    if (problems.length > 0) {
      fail(problems.join('; '));
      continue;
    }

    const leaks = findLeakingState(grammar, text);
    if (leaks.length > 0) {
      fail(`grammar state leaks past lines ${leaks.join(', ')} of the expansion`);
      continue;
    }

    const dark = findUnscopedText(grammar, text);
    if (dark.length > 0) {
      fail(`expansion has text carrying no scope:\n${dark.map((d) => `      ${d}`).join('\n')}`);
      continue;
    }

    const errors = errorsIn(expand(body, AS_COMPILED).text);
    if (errors.length > 0) {
      fail(
        `the expansion does not compile:\n${errors
          .map((e) => `      line ${e.range.start.line + 1}: E${e.code} ${e.message}`)
          .join('\n')}`,
      );
      continue;
    }

    console.log(`ok    ${name} (${snippet.prefix})`);
  }

  console.log(`\n${entries.length} snippets, ${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((error: unknown) => {
  console.error(String(error));
  process.exit(1);
});
