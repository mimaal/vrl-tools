/**
 * Writing the `# vrl-tools: terminal` comment that marks an output as ending
 * on purpose. What the comment means, and which line it belongs on, is the
 * wasm module's (`crates/vector-topology/src/terminal.rs`); this only works
 * out the text to insert once the line is known.
 *
 * It edits somebody's config, so it is careful rather than clever: it adds to
 * the end of one line, and when it cannot be sure the end of that line is a
 * place a comment can go, it offers nothing.
 *
 * Nothing here knows about VS Code, so `npm run test:markers` can exercise it.
 */

/** The comment, as it is written. */
export const MARKER = '# vrl-tools: terminal';

/** A marker already on a line, and the names after it. */
const EXISTING = /#\s*vrl-tools:\s*terminal(?![\w-])([^#]*)$/;

export interface MarkerEdit {
  /** The column to insert at: the end of the line's text. */
  readonly at: number;
  readonly text: string;
}

/**
 * What to insert at the end of `line` to mark an output there.
 *
 * `name` is the output to name after the marker, or `null` when the line
 * itself says which output is meant (a component with one output, a route on
 * its own line).
 *
 * `undefined` when there is nothing to add — the line already marks it — and
 * when the line opens or closes a multi-line string, where text appended
 * after the quotes could be inside the string or right after it, and telling
 * which means parsing the file.
 */
export function markerEdit(line: string, name: string | null): MarkerEdit | undefined {
  const text = line.replace(/\s+$/, '');
  if (text.includes("'''") || text.includes('"""')) {
    return undefined;
  }

  const existing = EXISTING.exec(text);
  if (existing) {
    const names = existing[1].split(/[\s,]+/).filter((entry) => entry !== '');
    // Bare, it already marks everything on the line; named, it gains a name.
    if (names.length === 0 || name === null || names.includes(name)) {
      return undefined;
    }
    return { at: text.length, text: `, ${name}` };
  }

  return { at: text.length, text: `${text === '' ? '' : ' '}${MARKER}${name === null ? '' : ` ${name}`}` };
}

/** Whether a config format has comments at all. JSON does not. */
export function takesComments(fileName: string): boolean {
  return /\.(ya?ml|toml)$/i.test(fileName);
}
