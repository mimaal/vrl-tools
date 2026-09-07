/**
 * Exercises editors/vscode/src/analysis.ts, the text reading behind hover,
 * completion and signature help.
 *
 * That module is the only hand-written reader of VRL in the project, so it is
 * also the only one that can be wrong in a way the compiler will not catch.
 * The cases below are written against half-finished lines on purpose: an
 * editor asks these questions while someone is still typing, when nothing
 * parses.
 *
 * Run with: npm run test:analysis
 */

import {
  completionContextAt,
  enclosingCall,
  identifierAt,
  isInCommentOrString,
  pathAt,
  pathsIn,
} from '../editors/vscode/src/analysis.js';

let failed = 0;

function check(what: string, actual: unknown, expected: unknown): void {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a === b) {
    console.log(`ok    ${what}`);
    return;
  }
  console.error(`FAIL  ${what}\n      expected: ${b}\n      actual:   ${a}`);
  failed++;
}

/** Marks the cursor with a `|` so the cases read like what you would type. */
function at(withCursor: string): { line: string; character: number } {
  const character = withCursor.indexOf('|');
  if (character < 0) {
    throw new Error(`case ${JSON.stringify(withCursor)} has no | cursor marker`);
  }
  return { line: withCursor.replace('|', ''), character };
}

function identifier(withCursor: string): string | undefined {
  const { line, character } = at(withCursor);
  return identifierAt(line, character)?.text;
}

function context(withCursor: string): unknown {
  const { line, character } = at(withCursor);
  const result = completionContextAt(line, character);
  return result.kind === 'function'
    ? { kind: result.kind, text: result.word.text }
    : result.kind === 'path'
      ? { kind: result.kind, text: result.path.text }
      : { kind: result.kind };
}

function call(withCursor: string): unknown {
  const character = withCursor.indexOf('|');
  return enclosingCall(withCursor.replace('|', ''), character);
}

// --- identifierAt ------------------------------------------------------
check('the word under the cursor', identifier('x = parse_j|son(.m)'), 'parse_json');
check('the word just typed', identifier('x = parse_json|'), 'parse_json');
check('a trailing ! is not part of the name', identifier('x = parse_json|!(.m)'), 'parse_json');
check('hovering the ! still names the function', identifier('x = parse_json!|(.m)'), 'parse_json');
check('a number is not an identifier', identifier('x = 4|2'), undefined);
check('whitespace has no identifier', identifier('x = |'), undefined);

// --- isInCommentOrString ----------------------------------------------
check('inside a comment', isInCommentOrString('x = 1 # note |here', 14), true);
check('outside a comment', isInCommentOrString('x = 1 # note', 3), false);
check('inside a double-quoted string', isInCommentOrString('x = "hel|lo"', 8), true);
check('after a closed string', isInCommentOrString('x = "hello" |', 12), false);
check('a # inside a string is not a comment', isInCommentOrString('x = "a # b" |', 12), false);
check('inside a raw string', isInCommentOrString("x = s'ab|c'", 8), true);
check('an escaped quote does not close the string', isInCommentOrString('x = "a\\"b|"', 9), true);

// --- pathAt ------------------------------------------------------------
check('an event path being typed', pathAt('.foo.ba', 7)?.text, '.foo.ba');
check('a bare root path', pathAt('. = ', 1)?.text, '.');
check('a metadata path', pathAt('x = %vector.ho', 14)?.text, '%vector.ho');
check('member access on a variable is not an event path', pathAt('x = fields.cli', 14), undefined);
check('a decimal point is not a path', pathAt('x = 3.1', 7), undefined);

// --- completionContextAt ----------------------------------------------
check('typing a function name', context('x = to_i|'), { kind: 'function', text: 'to_i' });
check('typing a path', context('x = .foo.|'), { kind: 'path', text: '.foo.' });
check('an empty line offers functions', context('|'), { kind: 'function', text: '' });
check('nothing is offered inside a comment', context('# just a note |'), { kind: 'none' });
check('nothing is offered inside a string', context('x = "hel|'), { kind: 'none' });

// --- enclosingCall -----------------------------------------------------
check('the call being written', call('x = parse_json(|'), { name: 'parse_json', argumentIndex: 0 });
check('the second argument', call('x = parse_timestamp(.ts, |'), {
  name: 'parse_timestamp',
  argumentIndex: 1,
});
check('a named argument names its parameter', call('x = parse_timestamp(.ts, format: |'), {
  name: 'parse_timestamp',
  argumentIndex: 1,
  keyword: 'format',
});
check('the innermost call wins', call('x = to_int(parse_int(|'), {
  name: 'parse_int',
  argumentIndex: 0,
});
check('a closed inner call does not count', call('x = format_timestamp(now(), |'), {
  name: 'format_timestamp',
  argumentIndex: 1,
});
check('a ! before the paren belongs to the call', call('x = parse_json!(|'), {
  name: 'parse_json',
  argumentIndex: 0,
});
check('a comma inside a string is not an argument break', call('x = replace(.m, "a,b", |'), {
  name: 'replace',
  argumentIndex: 2,
});
check('a comma inside an array is not an argument break', call('x = set(.o, [1, 2], |'), {
  name: 'set',
  argumentIndex: 2,
});
check('outside any call', call('x = 1 |'), undefined);
check('after the call closes', call('x = parse_json(.m) |'), undefined);
check('a call opened on an earlier line', call('x = parse_key_value(\n  .message,\n  |'), {
  name: 'parse_key_value',
  argumentIndex: 1,
});

// --- pathsIn -----------------------------------------------------------
check('paths and their parents', pathsIn('.foo.bar = 1\n.baz = 2\n'), [
  '.baz',
  '.foo',
  '.foo.bar',
]);
check('metadata paths keep their sigil', pathsIn('x = %vector.host\n'), ['%vector', '%vector.host']);
check('a path in a comment is not a path', pathsIn('# .commented = 1\n.real = 2\n'), ['.real']);
check('a path inside a string is not a path', pathsIn('x = "see .quoted"\n.real = 2\n'), ['.real']);
check('member access on a variable is not a path', pathsIn('x = fields.client\n'), []);
check('a decimal number is not a path', pathsIn('x = 3.14\n'), []);
check('the same path twice is listed once', pathsIn('.a = 1\n.a = 2\n'), ['.a']);

console.log(`\n${failed} failed`);
if (failed > 0) {
  process.exit(1);
}
