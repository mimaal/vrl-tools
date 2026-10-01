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

/** Where an output goes, in a few words: `→ time-diff` or `(unread)`. */
export function destination(readers: readonly string[] | undefined): string {
  return readers && readers.length > 0 ? `→ ${readers.join(', ')}` : '(unread)';
}
