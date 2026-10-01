/**
 * Naming a component's outputs, and saying where each one goes.
 *
 * An output is written the way an input names it — `id` for the default one,
 * `id.port` for a named one — because that is the string in the config, the
 * one Vector's warnings use, and the only form that is not ambiguous: three
 * routers each have a `_unmatched`, and a bare "dropped" on an arrow says
 * nothing about whose.
 *
 * Nothing here knows about VS Code or the wasm module, so `npm run
 * test:pipelines` can check it against a real reading.
 */

/** The least of a component this needs. */
export interface HasOutputs {
  readonly id: string;
  readonly defaultOutput: boolean;
  readonly namedOutputs: readonly string[];
}

/** The least of an edge this needs. */
export interface Flows {
  readonly from: string;
  readonly output: string | null;
  readonly to: string;
}

/** The output an `exclusive_route` sends what no route took to. Not a route. */
const UNMATCHED = '_unmatched';

/**
 * The position each route of an `exclusive_route` is tried at, from 1.
 *
 * Its named outputs are its routes in the order Vector tries them — for a
 * router written across files, the order the files merge in — followed by
 * `_unmatched`, which is not tried but fallen through to. Empty for every
 * other type: a plain `route` sends an event down every route that matches,
 * so its routes have no order to speak of.
 */
export function evaluationOrder(component: {
  readonly type: string;
  readonly namedOutputs: readonly string[];
}): Map<string, number> {
  if (component.type !== 'exclusive_route') {
    return new Map();
  }
  return new Map(
    component.namedOutputs
      .filter((output) => output !== UNMATCHED)
      .map((output, index) => [output, index + 1]),
  );
}

/**
 * A route with its turn and the file that contributes it:
 * `1. firewall-demo — 00-module-demo-firewall.toml`. The file is what
 * explains the turn, when the routes come from several.
 */
export function routeLine(order: number, output: string, file: string): string {
  return `${order}. ${output} — ${file.split('/').pop() ?? file}`;
}

/** An output as an input names it. `null` is the default output. */
export function written(id: string, output: string | null): string {
  return output === null ? id : `${id}.${output}`;
}

/** Every output of a component, the default one first, as `null`. */
export function outputsOf(component: HasOutputs): (string | null)[] {
  return [...(component.defaultOutput ? [null] : []), ...component.namedOutputs];
}

/** Who reads each output, by its written name, in the order of the edges. */
export function consumers(edges: readonly Flows[]): Map<string, string[]> {
  const found = new Map<string, string[]>();
  for (const edge of edges) {
    const key = written(edge.from, edge.output);
    const readers = found.get(key) ?? [];
    if (!readers.includes(edge.to)) {
      readers.push(edge.to);
    }
    found.set(key, readers);
  }
  return found;
}

/**
 * Where an output goes, in a few words: `→ time-diff`, `(unread)`, or
 * `⊣ terminal` for one nothing reads that is marked as ending on purpose.
 */
export function destination(readers: readonly string[] | undefined, terminal = false): string {
  if (readers && readers.length > 0) {
    return `→ ${readers.join(', ')}`;
  }
  return terminal ? '⊣ terminal' : '(unread)';
}
