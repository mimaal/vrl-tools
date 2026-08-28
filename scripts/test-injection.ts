/**
 * Exercises the two injection grammars that put VRL highlighting inside Vector
 * configs.
 *
 * Both are loaded as root grammars here, which tests everything except the
 * `injectionSelector` itself: the trigger regex, where the embedded region
 * starts and — the part that actually breaks in the editor — where it stops.
 * Whether VS Code injects them into source.yaml / source.toml is decided by
 * the manifest's `injectTo`, and is verified by opening test-corpus/vector.yaml
 * in the Extension Development Host.
 *
 * Run with: npm run test:injection
 */

import { readFile } from 'node:fs/promises';
import * as path from 'node:path';
import { CORPUS, loadGrammar, scopesInDocument } from './grammar-harness.js';

const EMBEDDED = 'meta.embedded.block.vrl';

interface Case {
  readonly what: string;
  readonly scope: 'vrl.injection.yaml' | 'vrl.injection.toml';
  readonly document: string;
  /** Substring whose token is inspected. */
  readonly token: string;
  readonly expect?: string;
  readonly reject?: string;
  /** Which occurrence of `token` to inspect, 0-based. */
  readonly nth?: number;
}

const YAML_REMAP = [
  'transforms:',
  '  parse:',
  '    type: remap',
  '    source: |-',
  '      parsed, err = parse_json(.message)',
  '',
  '      .status = to_int(.status) ?? null',
  'sinks:',
  '  out:',
  '    type: console',
].join('\n');

const TOML_REMAP = [
  '[transforms.parse]',
  'type = "remap"',
  "source = '''",
  'parsed, err = parse_json(.message)',
  ".matched = match(.message, r'^\\d+$')",
  "'''",
  '',
  '[sinks.out]',
  'type = "console"',
].join('\n');

const CASES: Case[] = [
  // --- YAML -------------------------------------------------------------
  {
    what: 'YAML: the source key still looks like a YAML key',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: 'source',
    expect: 'entity.name.tag.yaml',
  },
  {
    what: 'YAML: the block scalar indicator is scoped',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: '|-',
    expect: 'keyword.control.flow.block-scalar.yaml',
  },
  {
    what: 'YAML: VRL inside the block is VRL',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: 'parse_json',
    expect: 'support.function.vrl',
  },
  {
    what: 'YAML: the block content is marked as embedded',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: 'parse_json',
    expect: EMBEDDED,
  },
  {
    what: 'YAML: a blank line does not end the block',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: 'to_int',
    expect: 'support.function.vrl',
  },
  {
    what: 'YAML: dedenting ends the block',
    scope: 'vrl.injection.yaml',
    document: YAML_REMAP,
    token: 'sinks',
    reject: EMBEDDED,
  },
  {
    what: 'YAML: a plain scalar source: value is not a VRL block',
    scope: 'vrl.injection.yaml',
    document: ['sinks:', '  out:', '    source: not-a-block-scalar'].join('\n'),
    token: 'not-a-block-scalar',
    reject: EMBEDDED,
  },
  {
    what: 'YAML: a source: at column 0 is never a Vector transform',
    scope: 'vrl.injection.yaml',
    document: ['source: |', '  parse_json(.message)'].join('\n'),
    token: 'parse_json',
    reject: EMBEDDED,
  },
  {
    what: 'YAML: a comment after the block indicator still triggers',
    scope: 'vrl.injection.yaml',
    document: ['  source: | # vrl', '    del(.message)'].join('\n'),
    token: 'del',
    expect: 'support.function.vrl',
  },
  {
    what: 'YAML: an explicit indentation indicator still triggers',
    scope: 'vrl.injection.yaml',
    document: ['  source: |2-', '    del(.message)'].join('\n'),
    token: 'del',
    expect: 'support.function.vrl',
  },
  {
    what: 'YAML: a vrl condition block is VRL too',
    scope: 'vrl.injection.yaml',
    document: [
      '  condition:',
      '    type: vrl',
      '    source: |',
      '      .status >= 500',
      '  other: 1',
    ].join('\n'),
    token: '500',
    expect: 'constant.numeric.integer.vrl',
  },

  // --- TOML -------------------------------------------------------------
  {
    what: 'TOML: VRL inside a literal string is VRL',
    scope: 'vrl.injection.toml',
    document: TOML_REMAP,
    token: 'parse_json',
    expect: 'support.function.vrl',
  },
  {
    what: 'TOML: a regex inside the block keeps its own scope',
    scope: 'vrl.injection.toml',
    document: TOML_REMAP,
    token: '\\d',
    expect: 'constant.character.escape.regexp.vrl',
  },
  {
    what: 'TOML: the closing quotes end the block',
    scope: 'vrl.injection.toml',
    document: TOML_REMAP,
    token: 'console',
    reject: EMBEDDED,
  },
  {
    what: 'TOML: a basic multi-line string works as well',
    scope: 'vrl.injection.toml',
    document: ['source = """', 'del(.message)', '"""'].join('\n'),
    token: 'del',
    expect: 'support.function.vrl',
  },
  {
    what: 'TOML: a dotted key such as condition.source triggers',
    scope: 'vrl.injection.toml',
    document: ["condition.source = '''", '.status >= 500', "'''"].join('\n'),
    token: '500',
    expect: 'constant.numeric.integer.vrl',
  },
  {
    what: 'TOML: a single-line source string is left alone',
    scope: 'vrl.injection.toml',
    document: 'source = "del(.message)"',
    token: 'del',
    reject: EMBEDDED,
  },
];

/** Assertions against the synthetic Vector configs in test-corpus/. */
interface CorpusCase {
  readonly file: string;
  readonly scope: 'vrl.injection.yaml' | 'vrl.injection.toml';
  readonly token: string;
  readonly nth?: number;
  readonly expect?: string;
  readonly reject?: string;
  readonly what: string;
}

const CORPUS_CASES: CorpusCase[] = [
  {
    what: 'corpus vector.yaml: the remap block is VRL',
    file: 'vector.yaml',
    scope: 'vrl.injection.yaml',
    token: 'parse_json',
    expect: 'support.function.vrl',
  },
  {
    what: 'corpus vector.yaml: the vrl condition block is VRL',
    file: 'vector.yaml',
    scope: 'vrl.injection.yaml',
    token: '>= 500',
    expect: EMBEDDED,
  },
  {
    what: 'corpus vector.yaml: the plain source key is left alone',
    file: 'vector.yaml',
    scope: 'vrl.injection.yaml',
    token: 'not-a-block-scalar',
    reject: EMBEDDED,
  },
  {
    what: 'corpus vector.toml: the remap block is VRL',
    file: 'vector.toml',
    scope: 'vrl.injection.toml',
    token: 'parse_json',
    expect: 'support.function.vrl',
  },
  {
    what: 'corpus vector.toml: the config after the block is not VRL',
    file: 'vector.toml',
    scope: 'vrl.injection.toml',
    token: '[sinks.out]',
    reject: EMBEDDED,
  },
];

async function main(): Promise<void> {
  const grammars = {
    'vrl.injection.yaml': await loadGrammar('vrl.injection.yaml'),
    'vrl.injection.toml': await loadGrammar('vrl.injection.toml'),
  } as const;

  let failed = 0;

  const check = (
    what: string,
    scopes: string[],
    expect: string | undefined,
    reject: string | undefined,
  ): void => {
    if (expect && !scopes.includes(expect)) {
      console.error(`FAIL  ${what}\n      expected: ${expect}\n      actual:   ${scopes.join(', ')}`);
      failed++;
      return;
    }
    if (reject && scopes.includes(reject)) {
      console.error(`FAIL  ${what}\n      must not have: ${reject}\n      actual:        ${scopes.join(', ')}`);
      failed++;
      return;
    }
    console.log(`ok    ${what}`);
  };

  for (const c of CASES) {
    try {
      check(c.what, scopesInDocument(grammars[c.scope], c.document, c.token, c.nth ?? 0), c.expect, c.reject);
    } catch (error) {
      console.error(`FAIL  ${c.what}\n      ${String(error)}`);
      failed++;
    }
  }

  for (const c of CORPUS_CASES) {
    try {
      const source = await readFile(path.join(CORPUS, c.file), 'utf8');
      check(c.what, scopesInDocument(grammars[c.scope], source, c.token, c.nth ?? 0), c.expect, c.reject);
    } catch (error) {
      console.error(`FAIL  ${c.what}\n      ${String(error)}`);
      failed++;
    }
  }

  console.log(`\n${CASES.length + CORPUS_CASES.length} checks, ${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((error: unknown) => {
  console.error(String(error));
  process.exit(1);
});
