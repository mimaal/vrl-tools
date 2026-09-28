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
 * with name ...") — except where Vector merges them first, within one
 * `--config-dir` ([`clash`] says exactly when). So the whole folder is tried
 * first and split only where the names actually clash — by the next directory
 * down, then file by file. A
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
 * What one file declares, as far as telling pipelines apart needs to know.
 *
 * Built from the wasm module's reading of the file on its own
 * ([`shapeOf`]); a file that does not parse is [`EMPTY`], so a config mid-edit
 * does not split the pipeline it belongs to.
 */
export interface Shape {
  /** The components the file says the `type` of. */
  readonly declared: readonly string[];
  /**
   * The components it only adds to: `[[transforms.split.routes]]` in a file
   * of its own, which `--config-dir` merges into the router another declares.
   */
  readonly pieces: readonly string[];
  readonly sources: number;
  readonly sinks: number;
}

export const EMPTY: Shape = { declared: [], pieces: [], sources: 0, sinks: 0 };

/** A file's [`Shape`], from the components the wasm module read in it. */
export function shapeOf(
  components: readonly { readonly id: string; readonly type: string; readonly role: string }[],
): Shape {
  const typed = components.filter((component) => component.type !== '');
  return {
    declared: typed.map((component) => component.id),
    pieces: components.filter((component) => component.type === '').map((component) => component.id),
    sources: typed.filter((component) => component.role === 'source').length,
    sinks: typed.filter((component) => component.role === 'sink').length,
  };
}

/**
 * Whether a file could run as a config by itself: it declares where events
 * come from and where they go.
 */
function whole(shape: Shape): boolean {
  return shape.sources > 0 && shape.sinks > 0;
}

/**
 * Splits `files` into the pipelines they form.
 *
 * `shapeOf` gives what one file declares. `fallback` names the group when the
 * files share no directory — the workspace folder's name reads better there
 * than an empty string.
 */
export function group<T extends Named>(
  files: readonly T[],
  fallback: string,
  shapeOf: (file: T) => Shape,
): Group<T>[] {
  if (files.length === 0) {
    return [];
  }

  const base = commonDirectory(files);
  if (files.length === 1 || !clash(files, shapeOf)) {
    return [named(base, files, fallback)];
  }

  // Split by the next directory below the one they share. The files directly
  // in it are one bucket, not one each: they are as much one directory as any
  // subdirectory is, and `--config 'config/*.toml'` is how most pipelines are
  // started. Giving each its own bucket read a 16-file pipeline beside one
  // subdirectory of examples as sixteen pipelines of one file.
  const buckets = new Map<string, T[]>();
  for (const file of files) {
    const rest = base ? file.name.slice(base.length + 1) : file.name;
    const key = rest.includes('/')
      ? (base ? `${base}/` : '') + rest.slice(0, rest.indexOf('/'))
      : base;
    buckets.set(key, [...(buckets.get(key) ?? []), file]);
  }

  // Everything in one bucket and still clashing: they are all in the same
  // directory, and [`clash`] only says that of complete configs, so there is
  // nothing left to split by but the files themselves.
  if (buckets.size <= 1) {
    return files.map((file) => named(base, [file], fallback));
  }

  return [...buckets.values()].flatMap((bucket) => group(bucket, fallback, shapeOf));
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

/**
 * The names a file declares: those of the components it says the `type` of.
 *
 * A component without one is a piece of a component declared elsewhere, and
 * a directory where every file adds one is the opposite of a clash.
 */
export function declaredNames(
  components: readonly { readonly id: string; readonly type: string }[],
): string[] {
  return components.filter((component) => component.type !== '').map((component) => component.id);
}

/**
 * Whether these files cannot be one pipeline: one name, declared twice, in a
 * way no config Vector starts on has.
 *
 * - **In two directories**, always. Vector merges only the files of one
 *   `--config-dir`; two directories are appended, and a name they share is
 *   "duplicate id". That is `config/prod` beside `config/staging`.
 * - **In one directory**, only between two complete configs — each with its
 *   own sources and sinks. Vector would merge them, since the files of one
 *   directory are one value before they are components; but a directory of
 *   examples that each declare `app` is not one pipeline that happens to
 *   declare `app` twice. A file that repeats a declaration without being a
 *   config of its own — a product's file restating the router it adds a route
 *   to — is exactly what the merge is for, and the wasm module reads it so.
 */
function clash<T extends Named>(files: readonly T[], shapeOf: (file: T) => Shape): boolean {
  const declaredBy = new Map<string, T[]>();
  for (const file of files) {
    // Within one file a repeated name is a mistake in that file, not a sign
    // that it belongs to a different pipeline, so each file counts its own
    // names once.
    for (const name of new Set(shapeOf(file).declared)) {
      declaredBy.set(name, [...(declaredBy.get(name) ?? []), file]);
    }
  }

  for (const holders of declaredBy.values()) {
    for (let i = 0; i < holders.length; i += 1) {
      for (let j = i + 1; j < holders.length; j += 1) {
        const [a, b] = [holders[i], holders[j]];
        if (directoryOf(a.name) !== directoryOf(b.name)) {
          return true;
        }
        if (whole(shapeOf(a)) && whole(shapeOf(b))) {
          return true;
        }
      }
    }
  }
  return false;
}

function directoryOf(name: string): string {
  return name.includes('/') ? name.slice(0, name.lastIndexOf('/')) : '';
}

/** What a whole group declares: the union of its files' [`Shape`]s. */
function shapeOfGroup<T extends Named>(
  files: readonly T[],
  shapeOf: (file: T) => Shape,
): { names: Set<string>; sources: number; sinks: number } {
  const names = new Set<string>();
  let sources = 0;
  let sinks = 0;
  for (const file of files) {
    const shape = shapeOf(file);
    shape.declared.forEach((name) => names.add(name));
    shape.pieces.forEach((name) => names.add(name));
    sources += shape.sources;
    sinks += shape.sinks;
  }
  return { names, sources, sinks };
}

/**
 * The groups that are pipelines, without the ones that are only parts of
 * another.
 *
 * A directory of product files kept ready to be copied in — `devices-available/`
 * beside the `config/` that holds the enabled ones — splits off from the
 * pipeline because its names clash with it, and on its own it is 258 files
 * adding routes to a router it does not have, with 777 findings to prove it.
 * It is not a pipeline: it has no sources and no sinks, so Vector would not
 * start on it ("no sources", `check_shape`), and every name in it that is not
 * its own belongs to a pipeline that is listed.
 *
 * Dropped rather than listed as "fragments of `config`": a row nobody should
 * pick is a row somebody will, and what it would show — a graph of pieces with
 * nowhere to attach — is the thing being fixed. Its files are still configs:
 * opening one still graphs it, alone, as any file outside a pipeline is.
 *
 * A group is dropped only when it has neither sources nor sinks **and** shares
 * a name with a group that has them. A workspace of nothing but transforms has
 * no pipeline to prefer, and keeps what it has.
 */
export function pipelinesOnly<G extends { readonly files: readonly T[] }, T extends Named>(
  groups: readonly G[],
  shapeOf: (file: T) => Shape,
): G[] {
  const shapes = groups.map((found) => shapeOfGroup(found.files, shapeOf));
  return groups.filter((_, index) => {
    const own = shapes[index];
    if (own.sources > 0 || own.sinks > 0) {
      return true;
    }
    return !shapes.some(
      (other, at) =>
        at !== index &&
        (other.sources > 0 || other.sinks > 0) &&
        [...own.names].some((name) => other.names.has(name)),
    );
  });
}

/**
 * The pipeline to show when none has been chosen: one with both sources and
 * sinks, and of those the one with the most components.
 *
 * Not simply the first: the first alphabetically is as likely to be
 * `config-monitoring` — three components watching the real pipeline — as the
 * real pipeline. The biggest runnable one is what a workspace is usually
 * about. Ties keep the earlier, so the choice is stable. `-1` for no groups.
 */
export function mainPipeline<G extends { readonly files: readonly T[] }, T extends Named>(
  groups: readonly G[],
  shapeOf: (file: T) => Shape,
): number {
  let best = -1;
  let bestScore: [number, number] = [-1, -1];
  groups.forEach((found, index) => {
    const shape = shapeOfGroup(found.files, shapeOf);
    const score: [number, number] = [
      shape.sources > 0 && shape.sinks > 0 ? 1 : 0,
      shape.names.size,
    ];
    if (score[0] > bestScore[0] || (score[0] === bestScore[0] && score[1] > bestScore[1])) {
      best = index;
      bestScore = score;
    }
  });
  return best;
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

/**
 * Whether a file's text declares a top-level section only a Vector config has:
 * `sources`, `transforms`, `sinks` or `enrichment_tables`.
 *
 * - YAML: `sources:` at the start of a line. Only there — an indented one is a
 *   key of something else, a Helm chart's `customConfig` for one.
 * - JSON: `"sources":`, at any indentation, since every JSON key is indented.
 * - TOML: `[sources.x]`, `[sources]` or `[[transforms.x.routes]]`, at any
 *   indentation, because TOML allows whitespace before a header and configs
 *   that nest their tables visually use it. The double bracket matters on its
 *   own: a file that only adds routes to a router declared elsewhere has no
 *   other header.
 *
 * Here rather than beside the file search so `npm run test:grouping` can
 * exercise it: which files are Vector's is the input to everything else.
 */
export function declaresVectorSection(text: string): boolean {
  return VECTOR_SECTION.test(text);
}

const VECTOR_SECTION =
  /^(?:(?:sources|transforms|sinks|enrichment_tables)\s*:|[ \t]*"(?:sources|transforms|sinks|enrichment_tables)"\s*:|[ \t]*\[\[?[ \t]*(?:sources|transforms|sinks|enrichment_tables)[ \t]*[.\]])/m;
