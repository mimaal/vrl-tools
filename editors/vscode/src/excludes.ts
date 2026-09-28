/**
 * What the guess at which files are Vector's should not look at.
 *
 * `vscode.workspace.findFiles` with an exclude pattern of its own applies that
 * pattern and nothing else: not `files.exclude`, not `search.exclude`, and
 * never `.gitignore`. So a `tmp.*` scratch directory, ignored by git and
 * hidden from search, turned up in the list of pipelines with every config
 * someone had copied into it. The stable API has no way to ask for the
 * ignore files to be honoured (`findFiles2` is still a proposal), so the rules
 * are read here and the candidates filtered.
 *
 * Only the guess is filtered. Files named by `vrl-tools.vectorConfig` or
 * `vrl-tools.vectorConfigDir` are what Vector is started with, ignored or not.
 *
 * Nothing here knows about VS Code, which is what lets `npm run test:grouping`
 * exercise it.
 */

/** One pattern, and whether it can only match a directory (`tmp/`). */
export interface Rule {
  readonly glob: string;
  readonly directoryOnly: boolean;
}

/**
 * The patterns a `files.exclude` or `search.exclude` object switches on.
 *
 * A value can also be `{ when: "$(basename).ts" }`, a condition on a sibling
 * file; those are left out rather than half-implemented, which errs towards
 * reading a file, never towards hiding a pipeline.
 */
export function settingRules(setting: unknown): Rule[] {
  if (typeof setting !== 'object' || setting === null) {
    return [];
  }
  return Object.entries(setting)
    .filter(([, enabled]) => enabled === true)
    .map(([glob]) => ({ glob, directoryOnly: false }));
}

/**
 * The patterns of a `.gitignore`, as globs relative to the directory it is in.
 *
 * Git's rules, as far as a glob can say them: a pattern with no slash but a
 * trailing one matches at any depth, one with a slash is anchored, a trailing
 * slash means a directory. Negations (`!keep.toml`) are skipped — honouring
 * them needs git's ordering rules, and skipping them can only hide less.
 */
export function gitignoreRules(text: string): Rule[] {
  const rules: Rule[] = [];
  for (const raw of text.split(/\r?\n/)) {
    let line = raw.replace(/(?<!\\)\s+$/, '');
    if (line === '' || line.startsWith('#') || line.startsWith('!')) {
      continue;
    }
    line = line.replace(/^\\([#!])/, '$1');
    const directoryOnly = line.endsWith('/');
    if (directoryOnly) {
      line = line.replace(/\/+$/, '');
    }
    const anchored = line.includes('/');
    line = line.replace(/^\//, '');
    rules.push({ glob: anchored ? line : `**/${line}`, directoryOnly });
  }
  return rules;
}

/** Answers "is this path excluded?" for a set of [`Rule`]s. */
export class Exclusions {
  private readonly compiled: { readonly pattern: RegExp; readonly directoryOnly: boolean }[];

  constructor(rules: readonly Rule[]) {
    this.compiled = rules.map((rule) => ({
      pattern: globToRegExp(rule.glob),
      directoryOnly: rule.directoryOnly,
    }));
  }

  /**
   * Whether `path` — relative to the workspace folder, with `/` — is
   * excluded, itself or through a directory it is in: `**` + `/tmp.*` hides
   * `tmp.old/config/vector.toml` because it matches `tmp.old`.
   */
  excludes(path: string): boolean {
    const segments = path.split('/');
    for (let depth = 1; depth <= segments.length; depth += 1) {
      const prefix = segments.slice(0, depth).join('/');
      const isFile = depth === segments.length;
      if (
        this.compiled.some(
          (rule) => !(isFile && rule.directoryOnly) && rule.pattern.test(prefix),
        )
      ) {
        return true;
      }
    }
    return false;
  }
}

/**
 * A glob as VS Code writes them — `**`, `*`, `?`, `{a,b}`, `[abc]` — as a
 * regular expression over a whole relative path.
 */
export function globToRegExp(glob: string): RegExp {
  let out = '';
  let braces = 0;
  for (let i = 0; i < glob.length; i += 1) {
    const char = glob[i];
    if (char === '*' && glob[i + 1] === '*') {
      i += 1;
      if (glob[i + 1] === '/') {
        i += 1;
        out += '(?:.*/)?';
      } else {
        out += '.*';
      }
    } else if (char === '*') {
      out += '[^/]*';
    } else if (char === '?') {
      out += '[^/]';
    } else if (char === '{') {
      braces += 1;
      out += '(?:';
    } else if (char === '}' && braces > 0) {
      braces -= 1;
      out += ')';
    } else if (char === ',' && braces > 0) {
      out += '|';
    } else if (char === '[') {
      const end = glob.indexOf(']', i + 1);
      if (end < 0) {
        out += '\\[';
      } else {
        const body = glob.slice(i + 1, end).replace(/^!/, '^').replace(/\\/g, '\\\\');
        out += `[${body}]`;
        i = end;
      }
    } else {
      out += char.replace(/[.+^$()|\\\]]/g, '\\$&');
    }
  }
  return new RegExp(`^${out}$`);
}
