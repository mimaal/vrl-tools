/**
 * Telling a workspace's config files apart into pipelines.
 *
 * A workspace holds more than one pipeline often enough to matter: `config/prod`
 * beside `config/staging`, a folder of examples, a test corpus. Reading all of
 * them as one config is what the guess used to do, and what it produces is
 * nonsense — forty-four "two components are called `app_logs`" errors and a
 * picture of a topology nobody runs.
 *
 * The rule for splitting them is **Vector's, not a guess about folder names**:
 * files whose component names collide cannot be one config, because
 * `check_shape` refuses to start on exactly that ("More than one component
 * with name ..."). So the whole folder is tried first and split only where the
 * names actually clash — by the next directory down, then file by file. A
 * pipeline genuinely spread across subdirectories, which is what
 * `--config 'config/**' + '/*.toml'` is for, stays whole; eighteen unrelated
 * configs come apart into eighteen.
 *
 * Nothing here knows about VS Code or about the wasm module: it takes names
 * and gives back groups, which is what makes it testable without either.
 */

/** The least a file has to be for grouping: a path, with `/` separators. */
export interface Named {
  readonly name: string;
}

export interface Group<T extends Named> {
  /** The directory they share, or the file's own name when it is alone. */
  readonly title: string;
  /** Stable enough to remember a choice by, across a reload. */
  readonly key: string;
  readonly files: readonly T[];
}

/**
 * Splits `files` into the pipelines they form.
 *
 * `namesOf` gives one file's component IDs; a file that does not parse should
 * give none, so a config mid-edit does not split the pipeline it belongs to.
 * `fallback` names the group when the files share no directory — the workspace
 * folder's name reads better there than an empty string.
 */
export function group<T extends Named>(
  files: readonly T[],
  fallback: string,
  namesOf: (file: T) => readonly string[],
): Group<T>[] {
  if (files.length === 0) {
    return [];
  }

  const base = commonDirectory(files);
  if (files.length === 1 || !clash(files, namesOf)) {
    return [named(base, files, fallback)];
  }

  // Split by the next path segment below the directory they share.
  const buckets = new Map<string, T[]>();
  for (const file of files) {
    const rest = base ? file.name.slice(base.length + 1) : file.name;
    const head = rest.includes('/') ? rest.slice(0, rest.indexOf('/')) : rest;
    const key = base ? `${base}/${head}` : head;
    buckets.set(key, [...(buckets.get(key) ?? []), file]);
  }

  // Everything in one bucket and still clashing: they are all in the same
  // directory, so there is nothing left to split by but the files themselves.
  if (buckets.size <= 1) {
    return files.map((file) => named(base, [file], fallback));
  }

  return [...buckets.values()].flatMap((bucket) => group(bucket, fallback, namesOf));
}

/**
 * Names a group after the directory its files share — except a group of one,
 * which is named after the file.
 *
 * A directory that comes apart yields several groups, and naming all of them
 * after that same directory would leave the list of pipelines with three rows
 * saying `corpus`. The file name is both unique and the more useful thing to
 * read when a pipeline is one file.
 */
function named<T extends Named>(base: string, files: readonly T[], fallback: string): Group<T> {
  if (files.length === 1) {
    return { title: files[0].name, key: files[0].name, files };
  }
  return { title: base || fallback, key: base || '.', files };
}

/** Whether a component name is declared in more than one of these files. */
function clash<T extends Named>(
  files: readonly T[],
  namesOf: (file: T) => readonly string[],
): boolean {
  const seen = new Set<string>();
  for (const file of files) {
    // Within one file a repeated name is a mistake in that file, not a sign
    // that it belongs to a different pipeline, so each file counts its own
    // names once.
    for (const name of new Set(namesOf(file))) {
      if (seen.has(name)) {
        return true;
      }
      seen.add(name);
    }
  }
  return false;
}

/** The deepest directory every one of these files is under, '' for the root. */
export function commonDirectory<T extends Named>(files: readonly T[]): string {
  const paths = files.map((file) => file.name.split('/').slice(0, -1));
  if (paths.length === 0) {
    return '';
  }

  const common: string[] = [];
  for (let depth = 0; depth < paths[0].length; depth += 1) {
    const segment = paths[0][depth];
    if (!paths.every((path) => path[depth] === segment)) {
      break;
    }
    common.push(segment);
  }
  return common.join('/');
}
