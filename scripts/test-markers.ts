/**
 * Exercises editors/vscode/src/markers.ts, which writes the
 * `# vrl-tools: terminal` comment a Quick Fix inserts.
 *
 * It edits somebody's config, so what it refuses to do matters as much as
 * what it does. Whether the comment it writes is then *read* as a marker is
 * checked where the reading is: `test:pipelines` applies these edits to the
 * fleet fixture and reads it again.
 *
 * Run with: npm run test:markers
 */

import { markerEdit, takesComments } from '../editors/vscode/src/markers.js';

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

/** The line after the edit, or `undefined` when none is offered. */
function apply(line: string, name: string | null): string | undefined {
  const edit = markerEdit(line, name);
  return edit && line.slice(0, edit.at) + edit.text + line.slice(edit.at);
}

check(
  'a component with one output: the bare marker, after the header',
  apply('[transforms.dropped-handler]', null),
  '[transforms.dropped-handler] # vrl-tools: terminal',
);
check(
  'an output with no line of its own is named after the marker',
  apply('[transforms.route_by_product]', '_unmatched'),
  '[transforms.route_by_product] # vrl-tools: terminal _unmatched',
);
check(
  'a route on its own line, in TOML and in YAML',
  [apply('name = "imposible"', null), apply('      - name: imposible', null)],
  ['name = "imposible" # vrl-tools: terminal', '      - name: imposible # vrl-tools: terminal'],
);
check(
  'trailing space and a Windows line ending stay after the comment',
  apply('  split:  \r', '_unmatched'),
  '  split: # vrl-tools: terminal _unmatched  \r',
);
check(
  'a second output joins the names already there',
  apply('[transforms.r] # vrl-tools: terminal _unmatched', 'dropped'),
  '[transforms.r] # vrl-tools: terminal _unmatched, dropped',
);
check(
  'a comment already on the line is kept, and the marker goes after it',
  apply('[transforms.r] # the product router', '_unmatched'),
  '[transforms.r] # the product router # vrl-tools: terminal _unmatched',
);
check(
  'nothing to add: the line is already marked whole, or already names it',
  [
    apply('[transforms.r] # vrl-tools: terminal', '_unmatched'),
    apply('[transforms.r] # vrl-tools: terminal', null),
    apply('[transforms.r] # vrl-tools: terminal _unmatched', '_unmatched'),
  ],
  [undefined, undefined, undefined],
);
check(
  'a bare mark is not offered on a line that marks only some outputs',
  apply('[transforms.r] # vrl-tools: terminal _unmatched', null),
  undefined,
);
check(
  'a line that opens or closes a multi-line string is left alone',
  [
    apply("transforms.r.route.errors = '''", null),
    apply("'''", 'errors'),
    apply('source = """', null),
  ],
  [undefined, undefined, undefined],
);
check(
  'another vrl-tools comment is not mistaken for the marker',
  apply('[transforms.r] # vrl-tools: terminalOutputs', '_unmatched'),
  '[transforms.r] # vrl-tools: terminalOutputs # vrl-tools: terminal _unmatched',
);
check(
  'JSON has no comments, so it is never offered one',
  ['a.toml', 'a.yaml', 'a.YML', 'a.json'].map(takesComments),
  [true, true, true, false],
);

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
