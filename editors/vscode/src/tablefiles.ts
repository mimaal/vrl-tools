/**
 * Finding, in the workspace, a file a Vector config names by path — the CSV
 * behind a `file` enrichment table, the program behind a `remap`'s `file` —
 * and counting the rows of the first.
 *
 * A config writes these paths for the machine Vector runs on: "if a relative
 * path is provided, its root is the current working directory"
 * (`src/transforms/remap.rs`), and an absolute one is usually a path inside a
 * container. Neither says where the file is in a repository. So this is a
 * guess, and it is kept honest two ways: it only ever answers with a file
 * that exists, and whoever shows the answer shows which file it was.
 *
 * Nothing here knows about VS Code or the wasm module, so `npm run
 * test:tables` can exercise it — like `grouping.ts`, it is a place the
 * extension guesses, and no amount of compiling VRL keeps a guess honest.
 */

/**
 * Where `written` could be, as paths relative to the workspace folder, most
 * likely first.
 *
 * `config` is the config file that names it, relative to the same folder with
 * `/` separators. A relative path is tried against the config's directory and
 * every directory above it: Vector is started from one of them. An absolute
 * path is tried the same way with its leading directories dropped one at a
 * time — `/etc/vector/tables/geo.csv` as `etc/vector/tables/geo.csv`,
 * `vector/tables/geo.csv`, `tables/geo.csv` — and never down to the bare file
 * name, which would match any `geo.csv` in the repository.
 */
export function candidates(written: string, config: string): string[] {
  const path = written.trim().replaceAll('\\', '/');
  if (path === '') {
    return [];
  }

  const absolute = path.startsWith('/') || /^[A-Za-z]:\//.test(path);
  // A relative path keeps its `..` until it is joined to a directory, which
  // is what it climbs out of.
  const raw = path.replace(/^[A-Za-z]:/, '').split('/');
  const segments = absolute ? clean(raw) : raw.filter((segment) => segment !== '');
  if (segments === undefined || segments.length === 0) {
    return [];
  }

  // Every suffix of an absolute path that still has a directory in it; the
  // relative path itself otherwise.
  const tails: string[][] = [];
  if (absolute) {
    for (let from = 0; from < segments.length - 1; from++) {
      tails.push(segments.slice(from));
    }
    if (segments.length === 1) {
      tails.push(segments);
    }
  } else {
    tails.push(segments);
  }

  const directory = config.split('/').slice(0, -1);
  const found: string[] = [];
  for (const tail of tails) {
    for (let depth = directory.length; depth >= 0; depth--) {
      const joined = clean([...directory.slice(0, depth), ...tail]);
      if (joined !== undefined && joined.length > 0) {
        const candidate = joined.join('/');
        if (!found.includes(candidate)) {
          found.push(candidate);
        }
      }
    }
  }
  return found;
}

/** Resolves `.` and `..`; `undefined` when the path climbs out of the folder. */
function clean(segments: readonly string[]): string[] | undefined {
  const out: string[] = [];
  for (const segment of segments) {
    if (segment === '' || segment === '.') {
      continue;
    }
    if (segment === '..') {
      if (out.length === 0) {
        return undefined;
      }
      out.pop();
    } else {
      out.push(segment);
    }
  }
  return out;
}

/**
 * How many rows a CSV holds, the way Vector's `file` table counts them.
 *
 * Records, not lines: a quoted field may hold a line break, and a blank line
 * is not a record (`csv::Reader`, which `src/enrichment_tables/file.rs` reads
 * with). With `headers` — `include_headers`, on by default — the first record
 * names the columns and is not a row.
 *
 * `delimiter` only matters for telling a quote that opens a field from one in
 * the middle of it.
 */
export function countRows(text: string, headers: boolean, delimiter = ','): number {
  let records = 0;
  let quoted = false;
  /** Whether the record being read has anything in it. */
  let content = false;
  /** Whether the next character starts a field. */
  let fieldStart = true;

  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (quoted) {
      if (char === '"') {
        if (text[index + 1] === '"') {
          index++;
        } else {
          quoted = false;
        }
      }
      continue;
    }

    if (char === '\n' || char === '\r') {
      if (content) {
        records++;
      }
      content = false;
      fieldStart = true;
    } else if (char === '"' && fieldStart) {
      quoted = true;
      content = true;
      fieldStart = false;
    } else {
      content = true;
      fieldStart = char === delimiter;
    }
  }
  if (content) {
    records++;
  }

  return Math.max(0, records - (headers ? 1 : 0));
}

/** What is said about a table in one line of the sidebar. */
export interface TableLine {
  readonly type: string;
  readonly path: string | null;
  readonly readers: readonly string[];
}

/**
 * The one-line description of a table: who reads it, then where its data is
 * and how much of it there is.
 *
 * `showType` is false when every table of the pipeline has the same type —
 * 372 rows each saying `file` say nothing. `rows` is undefined when the data
 * file is not in the workspace, which is the usual case for a path written
 * for another machine.
 */
export function describeTable(table: TableLine, showType: boolean, rows?: number): string {
  const parts = [
    table.readers.length > 0 ? `read by: ${table.readers.join(', ')}` : 'unused in this pipeline',
  ];
  if (showType) {
    parts.push(table.type || 'no type');
  }
  if (table.path) {
    parts.push(rows === undefined ? table.path : `${table.path} (${rows} ${rows === 1 ? 'row' : 'rows'})`);
  }
  return parts.join(' · ');
}
