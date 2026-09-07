/**
 * Answers the questions hover, completion and signature help have to ask about
 * the text around the cursor: what word is under it, which call surrounds it,
 * which argument is being typed, and which paths this document already uses.
 *
 * This is the one place in the extension where reading VRL by hand is
 * legitimate. It is not validation — the compiler does that — it is the
 * editor's own question of "where am I", asked of a document that is usually
 * half-written and does not parse. A parser would refuse to answer.
 *
 * Nothing here imports `vscode`, so it can be tested from a plain script.
 */

/** A run of text with the offsets it occupies in its line. */
export interface Span {
  readonly text: string;
  readonly start: number;
  readonly end: number;
}

/** What is being typed at the cursor, and therefore what to offer. */
export type CompletionContext =
  | { readonly kind: 'function'; readonly word: Span }
  | { readonly kind: 'path'; readonly path: Span }
  | { readonly kind: 'none' };

/** The call the cursor sits inside, for signature help. */
export interface EnclosingCall {
  readonly name: string;
  /** Zero-based index of the argument being typed. */
  readonly argumentIndex: number;
  /** Set when the argument is written as `keyword: value`. */
  readonly keyword?: string;
}

const IDENTIFIER = /[A-Za-z_][A-Za-z0-9_]*/;

function isIdentifierChar(char: string | undefined): boolean {
  return char !== undefined && /[A-Za-z0-9_]/.test(char);
}

/**
 * The identifier the cursor is on or immediately after.
 *
 * A trailing `!` is not part of the name: `parse_json!` is a call to
 * `parse_json` with the error asserted, and hovering it should describe
 * `parse_json`.
 */
export function identifierAt(line: string, character: number): Span | undefined {
  let cursor = Math.min(character, line.length);

  // The `!` of a fallible call reads as part of the name to a person, so
  // hovering just after it should still describe the function. The `!` of
  // negation cannot be confused with it: an identifier never precedes that one.
  if (line[cursor - 1] === '!' && isIdentifierChar(line[cursor - 2])) {
    cursor--;
  }

  let start = cursor;
  while (start > 0 && isIdentifierChar(line[start - 1])) {
    start--;
  }

  let end = cursor;
  while (end < line.length && isIdentifierChar(line[end])) {
    end++;
  }

  const text = line.slice(start, end);
  if (text === '' || !IDENTIFIER.test(text) || /^[0-9]/.test(text)) {
    return undefined;
  }

  return { text, start, end };
}

/**
 * Whether the offset sits inside a comment or a string literal on this line.
 *
 * Single-line only, which is what VRL has: `#` runs to the end of the line, and
 * `"…"`, `s'…'`, `r'…'` and `t'…'` do not span lines.
 */
export function isInCommentOrString(line: string, character: number): boolean {
  let quote: string | undefined;

  for (let i = 0; i < character && i < line.length; i++) {
    const char = line[i];

    if (quote) {
      if (char === '\\' && quote === '"') {
        i++;
      } else if (char === quote) {
        quote = undefined;
      }
      continue;
    }

    if (char === '#') {
      return true;
    }
    if (char === '"' || char === "'") {
      quote = char;
    }
  }

  return quote !== undefined;
}

/**
 * The event or metadata path being typed at the cursor, e.g. `.foo.ba`.
 *
 * Returns the whole path including its leading `.` or `%`, so a completion can
 * replace it wholesale rather than trying to splice a segment in.
 */
export function pathAt(line: string, character: number): Span | undefined {
  let start = Math.min(character, line.length);

  while (start > 0) {
    const char = line[start - 1];
    if (isIdentifierChar(char) || char === '.') {
      start--;
      continue;
    }
    if (char === '%') {
      start--;
      break;
    }
    break;
  }

  const text = line.slice(start, character);
  if (!/^[.%][A-Za-z0-9_.]*$/.test(text)) {
    return undefined;
  }

  // `fields.client` is member access on a variable, not an event path. The
  // character before the path decides, and it is the same rule the grammar
  // uses.
  const before = line[start - 1];
  if (text.startsWith('.') && (isIdentifierChar(before) || before === ')' || before === ']')) {
    return undefined;
  }

  return { text, start, end: character };
}

/** What to offer at the cursor. */
export function completionContextAt(line: string, character: number): CompletionContext {
  if (isInCommentOrString(line, character)) {
    return { kind: 'none' };
  }

  const path = pathAt(line, character);
  if (path) {
    return { kind: 'path', path };
  }

  const word = identifierAt(line, character);
  if (word && word.start < character) {
    // A word right after a `.` or `%` is a path segment, which pathAt already
    // had its chance at; offering function names there is noise.
    const before = line[word.start - 1];
    if (before === '.' || before === '%') {
      return { kind: 'none' };
    }
    return { kind: 'function', word };
  }

  return { kind: 'function', word: word ?? { text: '', start: character, end: character } };
}

/**
 * The call surrounding `offset` in `source`, and which argument is being
 * typed.
 *
 * Walks backwards counting brackets, so a nested call reports the innermost
 * one — which is the one the cursor is actually inside.
 */
export function enclosingCall(source: string, offset: number): EnclosingCall | undefined {
  let depth = 0;
  let commas = 0;
  let i = Math.min(offset, source.length) - 1;

  while (i >= 0) {
    const char = source[i];

    if (char === '\n') {
      // Only step over a line if it cannot close the expression: a comment
      // ends at the newline, and everything before it is invisible.
      const lineStart = source.lastIndexOf('\n', i - 1) + 1;
      const hash = source.slice(lineStart, i).indexOf('#');
      if (hash >= 0) {
        i = lineStart + hash - 1;
        continue;
      }
      i--;
      continue;
    }

    if (char === '"' || char === "'") {
      // Skip the literal wholesale, backwards.
      const quote = char;
      i--;
      while (i >= 0 && source[i] !== quote) {
        i--;
      }
      i--;
      continue;
    }

    if (char === ')' || char === ']' || char === '}') {
      depth++;
      i--;
      continue;
    }

    if (char === '(') {
      if (depth > 0) {
        depth--;
        i--;
        continue;
      }

      const name = identifierBefore(source, i);
      if (!name) {
        return undefined;
      }

      const argument = source.slice(i + 1, offset);
      const keyword = /(?:^|,)\s*([A-Za-z_][A-Za-z0-9_]*)\s*:/.exec(argument.split(',').pop() ?? '');

      return {
        name,
        argumentIndex: commas,
        ...(keyword ? { keyword: keyword[1] } : {}),
      };
    }

    if (char === '[' || char === '{') {
      if (depth > 0) {
        depth--;
      }
      i--;
      continue;
    }

    if (char === ',' && depth === 0) {
      commas++;
      i--;
      continue;
    }

    i--;
  }

  return undefined;
}

/** The identifier ending at `end` (exclusive), skipping a `!` before it. */
function identifierBefore(source: string, end: number): string | undefined {
  let i = end;
  if (source[i - 1] === '!') {
    i--;
  }

  const stop = i;
  while (i > 0 && isIdentifierChar(source[i - 1])) {
    i--;
  }

  const text = source.slice(i, stop);
  return text !== '' && !/^[0-9]/.test(text) ? text : undefined;
}

/**
 * Every event and metadata path the document already mentions, plus each of
 * their parents.
 *
 * Offering the fields this file has already touched is the cheapest protection
 * there is against a silent typo: `.hostname` in one place and `.host_name` in
 * another compiles perfectly and quietly drops the data.
 */
export function pathsIn(source: string): string[] {
  const paths = new Set<string>();

  for (const line of source.split(/\r?\n/)) {
    const code = stripCommentsAndStrings(line);
    const matches = code.matchAll(/(?<![A-Za-z0-9_)\]}])([.%])([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)/g);

    for (const match of matches) {
      const [, sigil, body] = match;
      const segments = body.split('.');
      for (let i = 1; i <= segments.length; i++) {
        paths.add(sigil + segments.slice(0, i).join('.'));
      }
    }
  }

  return [...paths].sort();
}

/** Blanks out comments and string bodies so path scanning cannot see them. */
function stripCommentsAndStrings(line: string): string {
  let out = '';
  let quote: string | undefined;

  for (let i = 0; i < line.length; i++) {
    const char = line[i];

    if (quote) {
      if (char === '\\' && quote === '"') {
        out += '  ';
        i++;
      } else if (char === quote) {
        quote = undefined;
        out += ' ';
      } else {
        out += ' ';
      }
      continue;
    }

    if (char === '#') {
      break;
    }
    if (char === '"' || char === "'") {
      quote = char;
      out += ' ';
      continue;
    }

    out += char;
  }

  return out;
}
