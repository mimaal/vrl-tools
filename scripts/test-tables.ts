/**
 * Exercises editors/vscode/src/tablefiles.ts: where in the workspace a path
 * written for Vector's machine could be, how many rows a CSV holds, and the
 * one line said about a table.
 *
 * Like the grouping, it is a place the extension guesses, and nothing the
 * compiler says keeps a guess honest. The row count is checked against the
 * CSVs of `test-corpus/pipelines/fleet/tables/`, which Vector 0.58.0 loaded
 * when the fixture was validated.
 *
 * Run with: npm run test:tables
 */

import { readFileSync } from 'node:fs';
import * as path from 'node:path';

import { candidates, countRows, describeTable } from '../editors/vscode/src/tablefiles.js';
import { ROOT } from './grammar-harness.js';

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

// ------------------------------------------------------------ where a file is

check(
  'a relative path is tried from the config directory upwards',
  candidates('tables/geo.csv', 'fleet/config/tables-geo.toml'),
  ['fleet/config/tables/geo.csv', 'fleet/tables/geo.csv', 'tables/geo.csv'],
);
check(
  'a config at the top of the folder has one place to look',
  candidates('programs/a.vrl', 'vector.toml'),
  ['programs/a.vrl'],
);
check(
  'an absolute path is tried by its tail, never by its bare file name',
  candidates('/etc/vector/tables/geo.csv', 'config/base.toml'),
  [
    'config/etc/vector/tables/geo.csv',
    'etc/vector/tables/geo.csv',
    'config/vector/tables/geo.csv',
    'vector/tables/geo.csv',
    'config/tables/geo.csv',
    'tables/geo.csv',
  ],
);
check(
  'a Windows path is an absolute one',
  candidates('C:\\vector\\tables\\geo.csv', 'vector.toml'),
  ['vector/tables/geo.csv', 'tables/geo.csv'],
);
check(
  '`..` is resolved, and a path that climbs out of the folder is dropped',
  candidates('../tables/geo.csv', 'fleet/config/a.toml'),
  ['fleet/tables/geo.csv', 'tables/geo.csv'],
);
check('`./` means nothing', candidates('./geo.csv', 'a.toml'), ['geo.csv']);
check('an empty path is nowhere', candidates('  ', 'a.toml'), []);

// ----------------------------------------------------------------- the rows

check('a header and two rows are two rows', countRows('a,b\n1,2\n3,4\n', true), 2);
check('without headers the first line is a row too', countRows('a,b\n1,2\n3,4\n', false), 3);
check('a last line with no newline still counts', countRows('a,b\n1,2', true), 1);
check('CRLF is one line ending, and blank lines are not rows', countRows('a,b\r\n1,2\r\n\r\n3,4\r\n', true), 2);
check('a line break inside quotes does not end the row', countRows('a,b\n"x\ny",2\n3,4\n', true), 2);
check('a doubled quote is a quote, not the end of the field', countRows('a\n"say ""hi""\nagain"\n', true), 1);
check('a quote in the middle of a field opens nothing', countRows('a,b\n5" disk,x\n6" disk,y\n', true), 2);
check('an empty file has no rows', countRows('', true), 0);
check('a header alone has no rows', countRows('a,b\n', true), 0);

const corpus = (name: string): string =>
  readFileSync(path.join(ROOT, 'test-corpus/pipelines/fleet/tables', name), 'utf8');
check(
  'the fixture CSVs: a quoted comma, and a quoted line break',
  [countRows(corpus('assets.csv'), true), countRows(corpus('geo.csv'), true), countRows(corpus('threat.csv'), true)],
  [3, 2, 2],
);

// ------------------------------------------------------------ the one line

const table = { type: 'file', path: 'tables/geo.csv', readers: ['a', 'b'] };
check(
  'who reads it, then the path and the rows; no type when every table is a file',
  describeTable(table, false, 2),
  'read by: a, b · tables/geo.csv (2 rows)',
);
check(
  'a table nobody here reads says so, and one row is one row',
  describeTable({ ...table, readers: [] }, false, 1),
  'unused in this pipeline · tables/geo.csv (1 row)',
);
check(
  'a CSV that is not in the workspace keeps its path and claims no count',
  describeTable(table, false),
  'read by: a, b · tables/geo.csv',
);
check(
  'the type is said when tables differ in it',
  describeTable({ type: 'memory', path: null, readers: [] }, true),
  'unused in this pipeline · memory',
);

console.log(`\n${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
