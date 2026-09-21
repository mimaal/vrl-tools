// @ts-check
/*
 * Draws a Vector pipeline inside the graph panel.
 *
 * Everything that is a decision about the graph — which component feeds
 * which, through which output, what is wrong, which column and row each one
 * belongs in, which part of a large pipeline to narrow to — arrives already
 * made, from the wasm module's topology reader (`crates/vector-topology`).
 * This file turns that into pixels and handles the pointer and the keyboard.
 * It owns no knowledge of Vector.
 *
 * A pipeline of a few components is read at a glance. One of forty is not,
 * and most of what is here is for that case: a search that jumps to a
 * component, a selection that keeps one component's paths lit and says where
 * it is declared, a "show only its paths" that redraws just those, an
 * overview mode that trades detail for legible names when zoomed out, and a
 * minimap for knowing where the view is.
 *
 * Plain JavaScript, loaded as-is by the webview: the extension has no bundler,
 * and a drawing script of this size does not justify adding one.
 */
(function () {
  'use strict';

  // @ts-ignore — provided by the webview host.
  const vscode = acquireVsCodeApi();

  const NS = 'http://www.w3.org/2000/svg';
  const NODE_W = 210;
  const NODE_H = 62;
  const COL_GAP = 120;
  const ROW_GAP = 26;
  /** The height a lane takes in a column: an arrow passing between boxes. */
  const LANE_H = 4;
  /** The gap between two lanes, which only need to stay apart as lines. */
  const LANE_GAP = 6;
  const PAD = 48;
  const MIN_ZOOM = 0.1;
  const MAX_ZOOM = 3;
  /** Below this zoom the boxes' small print is unreadable, so it is swapped for big names. */
  const OVERVIEW_BELOW = 0.6;
  /** The zoom a search result is brought to, if the view is further out than that. */
  const READABLE = 0.9;

  const svg = /** @type {SVGSVGElement} */ (document.querySelector('#canvas'));
  const viewport = /** @type {SVGGElement} */ (document.querySelector('#viewport'));
  const edgeLayer = /** @type {SVGGElement} */ (document.querySelector('#edges'));
  const labelLayer = /** @type {SVGGElement} */ (document.querySelector('#labels'));
  const nodeLayer = /** @type {SVGGElement} */ (document.querySelector('#nodes'));
  const minimap = /** @type {SVGSVGElement} */ (document.querySelector('#minimap'));
  const title = /** @type {HTMLElement} */ (document.getElementById('title'));
  const summary = /** @type {HTMLElement} */ (document.getElementById('summary'));
  const banner = /** @type {HTMLElement} */ (document.getElementById('banner'));
  const empty = /** @type {HTMLElement} */ (document.getElementById('empty'));
  const problems = /** @type {HTMLElement} */ (document.getElementById('problems'));
  const problemList = /** @type {HTMLElement} */ (document.getElementById('problem-list'));
  const search = /** @type {HTMLInputElement} */ (document.getElementById('search'));
  const matchCount = /** @type {HTMLElement} */ (document.getElementById('match-count'));
  const focusPill = /** @type {HTMLElement} */ (document.getElementById('focus-pill'));
  const focusName = /** @type {HTMLElement} */ (document.getElementById('focus-name'));
  const details = /** @type {HTMLElement} */ (document.getElementById('details'));

  /** @typedef {{edge: any, group: SVGGElement, label?: SVGGElement}} Drawn */
  /** @typedef {{x: number, y: number, component: any}} Placed */

  /** The pan and zoom, applied to `viewport`. */
  const view = { x: 0, y: 0, k: 1 };
  /** Whether the person has panned or zoomed; until then a resize refits. */
  let moved = false;

  /**
   * What is on screen, kept for the interactions that come after drawing.
   * @type {{
   *   components: any[], edges: any[], findings: any[],
   *   at: Map<string, Placed>, groups: Map<string, SVGGElement>, drawn: Drawn[],
   *   findingsOf: Map<string, any[]>, focus: string | null,
   * }}
   */
  let current = {
    components: [],
    edges: [],
    findings: [],
    at: new Map(),
    groups: new Map(),
    drawn: [],
    findingsOf: new Map(),
    focus: null,
  };
  /** The component clicked, whose paths stay lit until something else is. */
  /** @type {string | null} */
  let selected = null;
  /** The components the search matches, in reading order, and which one Enter went to last. */
  /** @type {string[]} */
  let matches = [];
  let matchIndex = -1;

  /**
   * The files the pipeline on screen was read from. A component's or a
   * finding's `file` is an index into this.
   * @type {string[]}
   */
  let files = [];

  // ---------------------------------------------------------------- helpers

  /**
   * @param {string} name
   * @param {Record<string, string | number>} attributes
   * @param {Element} [parent]
   */
  function el(name, attributes, parent) {
    const node = document.createElementNS(NS, name);
    for (const [key, value] of Object.entries(attributes)) {
      node.setAttribute(key, String(value));
    }
    if (parent) {
      parent.appendChild(node);
    }
    return node;
  }

  /** @param {any} a @param {any} b */
  function sameRange(a, b) {
    return (
      a.start.line === b.start.line &&
      a.start.character === b.start.character &&
      a.end.line === b.end.line &&
      a.end.character === b.end.character
    );
  }

  /** @param {number} n @param {string} one @param {string} many */
  function count(n, one, many) {
    return `${n} ${n === 1 ? one : many}`;
  }

  /**
   * Writes a message with `code` spans into `parent` without ever treating it
   * as HTML: a component ID is whatever the config says it is.
   *
   * @param {HTMLElement} parent
   * @param {string} message
   */
  function writeMessage(parent, message) {
    message.split('`').forEach((part, index) => {
      if (index % 2 === 1) {
        const code = document.createElement('code');
        code.textContent = part;
        parent.appendChild(code);
      } else {
        parent.appendChild(document.createTextNode(part));
      }
    });
  }

  /**
   * Shortens `text` inside `node` until it fits `width`, with an ellipsis.
   *
   * @param {SVGTextElement} node
   * @param {string} text
   * @param {number} width
   */
  function fitText(node, text, width) {
    node.textContent = text;
    if (node.getComputedTextLength() <= width) {
      return;
    }
    let length = text.length;
    while (length > 1) {
      length--;
      node.textContent = `${text.slice(0, length)}…`;
      if (node.getComputedTextLength() <= width) {
        return;
      }
    }
  }

  /** A key for two component IDs, unambiguous whatever characters they hold. */
  /** @param {string} from @param {string} to */
  function pair(from, to) {
    return JSON.stringify([from, to]);
  }

  /** Where something is, for a reader: the file only when there are several. */
  /** @param {number} file @param {any} range */
  function where(file, range) {
    const line = `line ${range.start.line + 1}`;
    return files.length > 1 ? `${files[file] ?? '?'}, ${line}` : line;
  }

  /** @param {any} range @param {number} file */
  function reveal(range, file) {
    vscode.postMessage({ type: 'reveal', range, file });
  }

  /** Asks for the graph narrowed to the paths through `id`, or for all of it. */
  /** @param {string | null} id */
  function focusOn(id) {
    vscode.postMessage({ type: 'focus', id });
  }

  // -------------------------------------------------------------- rendering

  /** @param {any} analysis */
  function render(analysis) {
    edgeLayer.replaceChildren();
    labelLayer.replaceChildren();
    nodeLayer.replaceChildren();

    const components = /** @type {any[]} */ (analysis.components);
    const edges = /** @type {any[]} */ (analysis.edges);
    const findings = /** @type {any[]} */ (analysis.findings);
    const layout = /** @type {{components: any[], routes: any[]}} */ (analysis.layout);
    files = analysis.files ?? [];

    renderSummary(components, edges);
    renderProblems(findings);
    renderFocus(analysis.focus ?? null);
    empty.classList.toggle('visible', components.length === 0);

    const at = place(components, layout);
    const lanes = laneCentres(components, layout);

    // A finding points at a place in the config: the component's name, or
    // one of its inputs. Either way it belongs to that component's box.
    /** @type {Map<string, any[]>} */
    const findingsOf = new Map();
    for (const finding of findings) {
      // Ranges are per file, so two files can hold the same one.
      const owner = components.find(
        (c) =>
          c.file === finding.file &&
          (sameRange(c.range, finding.range) ||
            c.inputs.some((/** @type {any} */ input) => sameRange(input.range, finding.range))),
      );
      if (owner) {
        findingsOf.set(owner.id, [...(findingsOf.get(owner.id) ?? []), finding]);
      }
    }

    const drawn = drawEdges(edges, at, lanes);
    const groups = drawNodes(at, findingsOf);

    current = { components, edges, findings, at, groups, drawn, findingsOf, focus: analysis.focus ?? null };

    // What the person was doing survives a redraw while they edit: the
    // selection if the component still exists, the search as typed.
    if (selected && !at.has(selected)) {
      selected = null;
    }
    applySearch(false);
    showSelection();
    drawMinimap();
  }

  /**
   * Pixel positions from the layout's columns and rows.
   *
   * Each column is a stack of boxes and lanes in the layout's row order. A
   * lane is only as tall as the arrow passing through it, and two lanes next
   * to each other sit close, so a bundle of long arrows takes the room of a
   * bundle of lines rather than of as many boxes. Columns are then centred on
   * the tallest, so a pipeline that fans out and back in reads as a shape
   * rather than a staircase.
   *
   * @param {any[]} components
   * @param {{components: any[], routes: any[]}} layout
   * @returns {Map<string, Placed>}
   */
  function place(components, layout) {
    const { top, shift } = stacks(layout);
    /** @type {Map<string, Placed>} */
    const at = new Map();
    for (const placement of layout.components) {
      const component = components[placement.component];
      at.set(component.id, {
        x: columnX(placement.column),
        y: shift(placement.column) + (top.get(`c${placement.component}`) ?? 0),
        component,
      });
    }
    return at;
  }

  /**
   * @param {any[]} components
   * @param {{components: any[], routes: any[]}} layout
   * @returns {Map<string, {x: number, y: number}[]>} lane centres per pair of components
   */
  function laneCentres(components, layout) {
    const { top, shift } = stacks(layout);
    const lanes = new Map();
    for (const route of layout.routes) {
      lanes.set(
        pair(components[route.from].id, components[route.to].id),
        route.via.map((/** @type {any} */ slot) => ({
          x: columnX(slot.column),
          y: shift(slot.column) + (top.get(`l${slot.column}:${slot.row}`) ?? 0) + LANE_H / 2,
        })),
      );
    }
    return lanes;
  }

  /** @param {{components: any[], routes: any[]}} layout */
  function stacks(layout) {
    /** @type {Map<number, {row: number, height: number, lane: boolean, key: string}[]>} */
    const columns = new Map();
    const add = (/** @type {number} */ column, /** @type {any} */ item) =>
      columns.set(column, [...(columns.get(column) ?? []), item]);
    for (const p of layout.components) {
      add(p.column, { row: p.row, height: NODE_H, lane: false, key: `c${p.component}` });
    }
    for (const route of layout.routes) {
      for (const slot of route.via) {
        add(slot.column, { row: slot.row, height: LANE_H, lane: true, key: `l${slot.column}:${slot.row}` });
      }
    }

    /** @type {Map<string, number>} */
    const top = new Map();
    /** @type {Map<number, number>} */
    const heights = new Map();
    let tallest = 0;
    for (const [column, items] of columns) {
      items.sort((a, b) => a.row - b.row);
      let y = 0;
      items.forEach((item, index) => {
        const previous = items[index - 1];
        if (previous) {
          y += previous.lane && item.lane ? LANE_GAP : ROW_GAP;
        }
        top.set(item.key, y);
        y += item.height;
      });
      heights.set(column, y);
      tallest = Math.max(tallest, y);
    }
    const shift = (/** @type {number} */ column) => PAD + (tallest - (heights.get(column) ?? 0)) / 2;
    return { top, shift };
  }

  /** @param {number} column */
  function columnX(column) {
    return PAD + column * (NODE_W + COL_GAP);
  }

  /**
   * @param {any[]} edges
   * @param {Map<string, Placed>} at
   * @param {Map<string, {x: number, y: number}[]>} lanes
   * @returns {Drawn[]}
   */
  function drawEdges(edges, at, lanes) {
    // Several arrows leaving one box — a route's outputs — leave from
    // different points down its right side, ordered by where they are going,
    // so they fan out instead of starting as one thick line. The same on the
    // way in.
    /** @type {Map<string, any[]>} */
    const leaving = new Map();
    /** @type {Map<string, any[]>} */
    const arriving = new Map();
    for (const edge of edges) {
      if (!at.has(edge.from) || !at.has(edge.to)) {
        continue;
      }
      leaving.set(edge.from, [...(leaving.get(edge.from) ?? []), edge]);
      arriving.set(edge.to, [...(arriving.get(edge.to) ?? []), edge]);
    }
    const byY = (/** @type {(e: any) => string} */ other) => (/** @type {any} */ a, /** @type {any} */ b) =>
      (at.get(other(a))?.y ?? 0) - (at.get(other(b))?.y ?? 0);
    for (const list of leaving.values()) list.sort(byY((e) => e.to));
    for (const list of arriving.values()) list.sort(byY((e) => e.from));

    /** @param {any[]} list @param {any} edge @param {number} top */
    const port = (list, edge, top) => {
      const n = list.length;
      const span = Math.min(NODE_H - 24, (n - 1) * 12);
      const index = list.indexOf(edge);
      return top + NODE_H / 2 - span / 2 + (n > 1 ? (index * span) / (n - 1) : 0);
    };

    /** @type {Drawn[]} */
    const drawn = [];
    for (const edge of edges) {
      const from = at.get(edge.from);
      const to = at.get(edge.to);
      if (!from || !to) {
        continue;
      }

      const x1 = from.x + NODE_W;
      const y1 = port(leaving.get(edge.from) ?? [], edge, from.y);
      const x2 = to.x;
      const y2 = port(arriving.get(edge.to) ?? [], edge, to.y);
      const backwards = x2 <= x1;

      let d;
      if (backwards) {
        // Only a loop draws right to left, and a loop is an error: it goes
        // round underneath, dashed, rather than through the boxes.
        const below = Math.max(from.y, to.y) + NODE_H + 40;
        const middle = (x1 + x2) / 2;
        d =
          `M ${x1} ${y1} C ${x1 + 70} ${y1} ${x1 + 70} ${below} ${middle} ${below} ` +
          `C ${x2 - 70} ${below} ${x2 - 70} ${y2} ${x2} ${y2}`;
      } else {
        // Through every lane it crosses: straight along the column, curving
        // in the gaps, so it never runs through a box.
        d = `M ${x1} ${y1}`;
        let [cx, cy] = [x1, y1];
        for (const lane of lanes.get(pair(edge.from, edge.to)) ?? []) {
          d += curve(cx, cy, lane.x, lane.y) + ` L ${lane.x + NODE_W} ${lane.y}`;
          [cx, cy] = [lane.x + NODE_W, lane.y];
        }
        d += curve(cx, cy, x2, y2);
      }

      const group = /** @type {SVGGElement} */ (
        el('g', { class: backwards ? 'edge backwards' : 'edge' }, edgeLayer)
      );
      const path = /** @type {SVGPathElement} */ (
        el('path', { d, 'marker-end': backwards ? 'url(#arrow-error)' : 'url(#arrow)' }, group)
      );

      /** @type {SVGGElement | undefined} */
      let label;
      if (edge.output) {
        // In the middle of the first gap: the arrows leaving a route have
        // fanned apart there, and it is still close enough to the source to
        // read as where the output is chosen.
        const point = pointAtX(path, x1 + (backwards ? 30 : COL_GAP * 0.5));
        label = /** @type {SVGGElement} */ (el('g', { class: 'edge-label' }, labelLayer));
        const rect = el('rect', { rx: 8, ry: 8, height: 17 }, label);
        const text = /** @type {SVGTextElement} */ (
          el('text', { 'text-anchor': 'middle', 'dominant-baseline': 'central' }, label)
        );
        text.textContent = edge.output;
        const width = text.getComputedTextLength() + 14;
        rect.setAttribute('width', String(width));
        rect.setAttribute('x', String(point.x - width / 2));
        rect.setAttribute('y', String(point.y - 8.5));
        text.setAttribute('x', String(point.x));
        text.setAttribute('y', String(point.y));
      }

      drawn.push({ edge, group, label });
    }

    separateLabels(drawn);
    return drawn;
  }

  /**
   * A cubic from one point to another that leaves and arrives horizontally.
   *
   * @param {number} ax @param {number} ay @param {number} bx @param {number} by
   */
  function curve(ax, ay, bx, by) {
    const bend = Math.max(36, (bx - ax) * 0.5);
    return ` C ${ax + bend} ${ay} ${bx - bend} ${by} ${bx} ${by}`;
  }

  /**
   * The point on `path` whose x is `x`, by bisection along its length. The
   * forward arrows drawn here only ever move right, which is what makes
   * bisection valid; for a loop it still lands somewhere on the path.
   *
   * @param {SVGPathElement} path
   * @param {number} x
   */
  function pointAtX(path, x) {
    let low = 0;
    let high = path.getTotalLength();
    for (let i = 0; i < 24; i++) {
      const middle = (low + high) / 2;
      if (path.getPointAtLength(middle).x < x) low = middle;
      else high = middle;
    }
    return path.getPointAtLength((low + high) / 2);
  }

  /**
   * Pushes apart the labels leaving one component that would overlap: a
   * route whose outputs go to places close together puts them close too.
   *
   * @param {Drawn[]} drawn
   */
  function separateLabels(drawn) {
    /** @type {Map<string, SVGGElement[]>} */
    const byComponent = new Map();
    for (const { edge, label } of drawn) {
      if (label) byComponent.set(edge.from, [...(byComponent.get(edge.from) ?? []), label]);
    }
    for (const labels of byComponent.values()) {
      const boxes = labels
        .map((label) => ({ label, box: label.getBBox() }))
        .sort((a, b) => a.box.y - b.box.y);
      let floor = -Infinity;
      for (const { label, box } of boxes) {
        const dy = Math.max(0, floor + 3 - box.y);
        if (dy > 0) label.setAttribute('transform', `translate(0 ${dy})`);
        floor = box.y + dy + box.height;
      }
    }
  }

  /**
   * @param {Map<string, Placed>} at
   * @param {Map<string, any[]>} findingsOf
   * @returns {Map<string, SVGGElement>}
   */
  function drawNodes(at, findingsOf) {
    /** @type {Map<string, SVGGElement>} */
    const groups = new Map();

    for (const [id, { x, y, component }] of at) {
      const own = findingsOf.get(id) ?? [];
      const errors = own.filter((f) => f.severity === 'error').length;
      const warnings = own.length - errors;

      const classes = ['node', component.role];
      if (errors > 0) classes.push('has-error');
      else if (warnings > 0) classes.push('has-warning');

      const group = /** @type {SVGGElement} */ (
        el(
          'g',
          {
            class: classes.join(' '),
            transform: `translate(${x} ${y})`,
            tabindex: 0,
            role: 'button',
            'aria-label': `${component.role} ${id}, type ${component.type || 'missing'}`,
          },
          nodeLayer,
        )
      );
      groups.set(id, group);

      const tooltip = el('title', {}, group);
      tooltip.textContent = [
        `${id} (${component.type || 'no type'})`,
        ...(files.length > 1 ? [where(component.file, component.range)] : []),
        ...own.map((f) => `${f.severity}: ${f.message.replaceAll('`', '')}`),
        'Click to select, double-click to open in the config',
      ].join('\n');

      el('rect', { class: 'card', width: NODE_W, height: NODE_H, rx: 8, ry: 8 }, group);
      el('rect', { class: 'accent', x: 9, y: 12, width: 3, height: NODE_H - 24, rx: 1.5 }, group);

      const role = el('text', { class: 'role detail', x: 22, y: 20 }, group);
      role.textContent = component.role;

      const name = /** @type {SVGTextElement} */ (el('text', { class: 'id detail', x: 22, y: 38 }, group));
      fitText(name, id, NODE_W - 36);

      const type = /** @type {SVGTextElement} */ (el('text', { class: 'type detail', x: 22, y: 53 }, group));
      // With several files, the one declaring it, where the type has room.
      const declared = files.length > 1 ? ` · ${(files[component.file] ?? '').split('/').pop()}` : '';
      fitText(type, `${component.type || '(no type)'}${declared}`, NODE_W - 36);

      // The overview's name: one line, big enough to read from far out. It is
      // hidden by visibility rather than display, so it can be measured now.
      const big = /** @type {SVGTextElement} */ (
        el('text', { class: 'id-big', x: 22, y: NODE_H / 2, 'dominant-baseline': 'central' }, group)
      );
      fitText(big, id, NODE_W - 32);

      if (own.length > 0) {
        const badge = el('g', { class: `badge ${errors > 0 ? 'error' : 'warning'}` }, group);
        el('circle', { cx: NODE_W - 2, cy: 2, r: 9 }, badge);
        const number = el(
          'text',
          { x: NODE_W - 2, y: 2, 'text-anchor': 'middle', 'dominant-baseline': 'central' },
          badge,
        );
        number.textContent = String(own.length);
      }

      group.addEventListener('click', (event) => {
        event.stopPropagation();
        select(selected === id ? null : id);
      });
      group.addEventListener('dblclick', (event) => {
        event.stopPropagation();
        reveal(component.range, component.file);
      });
      group.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          reveal(component.range, component.file);
        } else if (event.key === ' ') {
          event.preventDefault();
          select(selected === id ? null : id);
        }
      });
      group.addEventListener('mouseenter', () => trace(id));
      group.addEventListener('mouseleave', () => trace(selected));
    }
    return groups;
  }

  // ------------------------------------------------------ paths and selection

  /**
   * Everything that can reach `id`, and everything it can reach.
   * @param {string} id
   */
  function reach(id) {
    /** @param {(e: any) => string} near @param {(e: any) => string} far */
    const walk = (near, far) => {
      const seen = new Set([id]);
      const queue = [id];
      while (queue.length > 0) {
        const at = queue.shift();
        for (const edge of current.edges) {
          if (near(edge) === at && !seen.has(far(edge))) {
            seen.add(far(edge));
            queue.push(far(edge));
          }
        }
      }
      return seen;
    };
    return {
      upstream: walk((e) => e.to, (e) => e.from),
      downstream: walk((e) => e.from, (e) => e.to),
    };
  }

  /**
   * Lights up every path events can take through `id`, or nothing.
   * @param {string | null} id
   */
  function trace(id) {
    if (!id || !current.groups.has(id)) {
      svg.classList.remove('tracing');
      for (const group of current.groups.values()) group.classList.remove('on-path', 'selected');
      for (const { group, label } of current.drawn) {
        group.classList.remove('on-path');
        label?.classList.remove('on-path');
        if (!group.classList.contains('backwards')) {
          group.querySelector('path')?.setAttribute('marker-end', 'url(#arrow)');
        }
      }
      return;
    }

    const { upstream, downstream } = reach(id);
    for (const [other, group] of current.groups) {
      group.classList.toggle('on-path', upstream.has(other) || downstream.has(other));
      group.classList.toggle('selected', other === selected);
    }
    for (const { edge, group, label } of current.drawn) {
      const onPath =
        (upstream.has(edge.from) && upstream.has(edge.to)) ||
        (downstream.has(edge.from) && downstream.has(edge.to));
      group.classList.toggle('on-path', onPath);
      label?.classList.toggle('on-path', onPath);
      if (!group.classList.contains('backwards')) {
        group.querySelector('path')?.setAttribute('marker-end', onPath ? 'url(#arrow-active)' : 'url(#arrow)');
      }
    }
    svg.classList.add('tracing');
  }

  /** @param {string | null} id */
  function select(id) {
    selected = id;
    showSelection();
  }

  /** Keeps the selection's paths lit and fills in the details bar. */
  function showSelection() {
    trace(selected);
    details.replaceChildren();
    const placed = selected ? current.at.get(selected) : undefined;
    details.classList.toggle('visible', Boolean(placed));
    if (!placed || !selected) {
      return;
    }

    const component = placed.component;
    const { upstream, downstream } = reach(selected);
    const own = current.findingsOf.get(selected) ?? [];

    const role = document.createElement('span');
    role.className = `chip ${component.role}`;
    role.textContent = component.role;

    const name = document.createElement('strong');
    name.textContent = selected;

    const type = document.createElement('code');
    type.textContent = component.type || '(no type)';

    const place = document.createElement('span');
    place.className = 'muted';
    place.textContent = files.length > 1 ? where(component.file, component.range) : `line ${component.range.start.line + 1}`;

    const flow = document.createElement('span');
    flow.className = 'muted';
    flow.textContent = `${count(upstream.size - 1, 'component', 'components')} upstream · ${downstream.size - 1} downstream`;

    details.append(role, name, type, place, flow);

    if (own.length > 0) {
      const issues = document.createElement('span');
      issues.className = `issues ${own.some((f) => f.severity === 'error') ? 'error' : 'warning'}`;
      issues.textContent = count(own.length, 'problem', 'problems');
      details.append(issues);
    }

    const spacer = document.createElement('span');
    spacer.className = 'spacer';
    details.append(spacer);

    const open = document.createElement('button');
    open.textContent = 'Open in config';
    open.title = 'Go to where it is declared (double-click the box does the same)';
    open.addEventListener('click', () => reveal(component.range, component.file));
    details.append(open);

    const narrow = document.createElement('button');
    if (current.focus === selected) {
      narrow.textContent = 'Show whole pipeline';
      narrow.addEventListener('click', () => focusOn(null));
    } else {
      narrow.textContent = 'Show only its paths';
      narrow.title = 'Redraw just what reaches it and what it reaches (F)';
      narrow.addEventListener('click', () => focusOn(selected));
    }
    details.append(narrow);
  }

  /** @param {string | null} focus */
  function renderFocus(focus) {
    focusPill.classList.toggle('visible', Boolean(focus));
    focusName.textContent = focus ?? '';
  }

  // ------------------------------------------------------------------ search

  /**
   * Marks the components matching the search box: by name, type or file.
   * @param {boolean} jump whether to go to the first match
   */
  function applySearch(jump) {
    const query = search.value.trim().toLowerCase();
    svg.classList.toggle('searching', query !== '');
    matches = [];
    for (const [id, group] of current.groups) {
      const component = current.at.get(id)?.component;
      const hit =
        query !== '' &&
        [id, component?.type ?? '', files[component?.file] ?? ''].some((text) =>
          text.toLowerCase().includes(query),
        );
      group.classList.toggle('match', hit);
      if (hit) matches.push(id);
    }
    // Reading order: left to right, then top to bottom.
    matches.sort((a, b) => {
      const pa = current.at.get(a);
      const pb = current.at.get(b);
      return (pa?.x ?? 0) - (pb?.x ?? 0) || (pa?.y ?? 0) - (pb?.y ?? 0);
    });
    matchCount.textContent =
      query === '' ? '' : matches.length === 0 ? 'no match' : count(matches.length, 'match', 'matches');
    matchIndex = -1;
    if (jump && matches.length > 0) {
      next(1);
    }
  }

  /** Goes to the next (or previous) match, and selects it. @param {number} step */
  function next(step) {
    if (matches.length === 0) return;
    matchIndex = (matchIndex + step + matches.length) % matches.length;
    const id = matches[matchIndex];
    select(id);
    centreOn(id);
    matchCount.textContent = `${matchIndex + 1} of ${matches.length}`;
  }

  search.addEventListener('input', () => applySearch(false));
  search.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      if (matchIndex === -1) applySearch(true);
      else next(event.shiftKey ? -1 : 1);
    } else if (event.key === 'Escape') {
      search.value = '';
      applySearch(false);
      search.blur();
    }
  });

  // ------------------------------------------------------------ summary, list

  /** @param {any[]} components @param {any[]} edges */
  function renderSummary(components, edges) {
    const of = (/** @type {string} */ role) => components.filter((c) => c.role === role).length;
    summary.textContent = [
      count(of('source'), 'source', 'sources'),
      count(of('transform'), 'transform', 'transforms'),
      count(of('sink'), 'sink', 'sinks'),
      count(edges.length, 'connection', 'connections'),
    ].join(' · ');
  }

  /** @param {any[]} findings */
  function renderProblems(findings) {
    problemList.replaceChildren();
    problems.classList.toggle('visible', findings.length > 0);

    const sorted = [...findings].sort(
      (a, b) =>
        (a.severity === 'error' ? 0 : 1) - (b.severity === 'error' ? 0 : 1) ||
        a.range.start.line - b.range.start.line,
    );
    for (const finding of sorted) {
      const item = document.createElement('li');
      item.tabIndex = 0;

      const severity = document.createElement('span');
      severity.className = `severity ${finding.severity}`;
      severity.textContent = finding.severity;
      item.appendChild(severity);

      const message = document.createElement('span');
      writeMessage(message, finding.message);
      item.appendChild(message);

      const line = document.createElement('span');
      line.className = 'line';
      line.textContent = where(finding.file, finding.range);
      item.appendChild(line);

      item.addEventListener('click', () => reveal(finding.range, finding.file));
      item.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') reveal(finding.range, finding.file);
      });
      problemList.appendChild(item);
    }
  }

  // ----------------------------------------------------------- pan and zoom

  function apply() {
    viewport.setAttribute('transform', `translate(${view.x} ${view.y}) scale(${view.k})`);
    // Far out, the small print is noise and the names are what matter.
    svg.classList.toggle('overview', view.k < OVERVIEW_BELOW);
    updateMinimap();
  }

  function fit() {
    const box = viewport.getBBox();
    const frame = svg.getBoundingClientRect();
    if (box.width === 0 || frame.width === 0) {
      return;
    }
    const k = Math.min((frame.width - PAD) / box.width, (frame.height - PAD) / box.height, 1.25);
    view.k = Math.max(MIN_ZOOM, k);
    view.x = (frame.width - box.width * view.k) / 2 - box.x * view.k;
    view.y = (frame.height - box.height * view.k) / 2 - box.y * view.k;
    moved = false;
    apply();
  }

  /** Zooms by `factor` keeping the point (px, py) of the frame still. */
  /** @param {number} factor @param {number} px @param {number} py */
  function zoomAbout(factor, px, py) {
    const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.k * factor));
    view.x = px - ((px - view.x) * k) / view.k;
    view.y = py - ((py - view.y) * k) / view.k;
    view.k = k;
    moved = true;
    apply();
  }

  /** Brings a component to the middle of the view, readable. @param {string} id */
  function centreOn(id) {
    const placed = current.at.get(id);
    if (!placed) return;
    const frame = svg.getBoundingClientRect();
    view.k = Math.max(view.k, READABLE);
    view.x = frame.width / 2 - (placed.x + NODE_W / 2) * view.k;
    view.y = frame.height / 2 - (placed.y + NODE_H / 2) * view.k;
    moved = true;
    apply();
  }

  // The wheel scrolls, as it does everywhere else in the editor; zooming is
  // Ctrl (or Cmd) with the wheel, which is also what a trackpad pinch sends.
  svg.addEventListener(
    'wheel',
    (event) => {
      event.preventDefault();
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? svg.clientHeight : 1;
      if (event.ctrlKey || event.metaKey) {
        const frame = svg.getBoundingClientRect();
        zoomAbout(Math.exp(-event.deltaY * unit * 0.0025), event.clientX - frame.left, event.clientY - frame.top);
        return;
      }
      const dx = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX;
      const dy = event.shiftKey && event.deltaX === 0 ? 0 : event.deltaY;
      view.x -= dx * unit;
      view.y -= dy * unit;
      moved = true;
      apply();
    },
    { passive: false },
  );

  /** @type {{x: number, y: number, startX: number, startY: number} | undefined} */
  let grab;
  svg.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || /** @type {Element} */ (event.target).closest('.node')) {
      return;
    }
    grab = { x: event.clientX - view.x, y: event.clientY - view.y, startX: event.clientX, startY: event.clientY };
    svg.setPointerCapture(event.pointerId);
    svg.classList.add('panning');
  });
  svg.addEventListener('pointermove', (event) => {
    if (!grab) return;
    view.x = event.clientX - grab.x;
    view.y = event.clientY - grab.y;
    moved = true;
    apply();
  });
  svg.addEventListener('pointerup', (event) => {
    // A click on empty canvas, not the end of a drag, clears the selection.
    if (grab && Math.hypot(event.clientX - grab.startX, event.clientY - grab.startY) < 4) {
      select(null);
    }
    grab = undefined;
    svg.classList.remove('panning');
  });
  svg.addEventListener('pointercancel', () => {
    grab = undefined;
    svg.classList.remove('panning');
  });

  new ResizeObserver(() => {
    if (!moved) fit();
    else updateMinimap();
  }).observe(svg);

  document.getElementById('fit')?.addEventListener('click', fit);
  document.getElementById('export')?.addEventListener('click', () => {
    vscode.postMessage({ type: 'export' });
  });
  document.getElementById('unfocus')?.addEventListener('click', () => focusOn(null));

  // The keyboard, for everything the pointer does. Keys typed into the search
  // box are the search box's.
  document.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
      event.preventDefault();
      search.focus();
      search.select();
      return;
    }
    if (event.target === search) return;

    const frame = svg.getBoundingClientRect();
    const step = 80;
    switch (event.key) {
      case '+':
      case '=':
        zoomAbout(1.25, frame.width / 2, frame.height / 2);
        break;
      case '-':
      case '_':
        zoomAbout(0.8, frame.width / 2, frame.height / 2);
        break;
      case '0':
        fit();
        break;
      case 'ArrowLeft':
        view.x += step;
        break;
      case 'ArrowRight':
        view.x -= step;
        break;
      case 'ArrowUp':
        view.y += step;
        break;
      case 'ArrowDown':
        view.y -= step;
        break;
      case 'f':
      case 'F':
        if (selected) focusOn(current.focus === selected ? null : selected);
        return;
      case 'Escape':
        if (selected) select(null);
        else if (current.focus) focusOn(null);
        return;
      default:
        return;
    }
    if (event.key.startsWith('Arrow')) {
      event.preventDefault();
      moved = true;
      apply();
    }
  });

  // ----------------------------------------------------------------- minimap

  /** The content's bounds, cached per render for the minimap. */
  /** @type {DOMRect | undefined} */
  let bounds;

  /**
   * A small map of the whole graph with the view drawn on it. Shown only
   * when the graph does not fit, which is when knowing where the view is
   * becomes a question.
   */
  function drawMinimap() {
    minimap.replaceChildren();
    bounds = current.at.size > 0 ? viewport.getBBox() : undefined;
    if (!bounds) {
      minimap.classList.remove('visible');
      return;
    }
    minimap.setAttribute('viewBox', `${bounds.x - 20} ${bounds.y - 20} ${bounds.width + 40} ${bounds.height + 40}`);
    for (const [id, placed] of current.at) {
      el(
        'rect',
        {
          class: `mini-node ${placed.component.role}${id === selected ? ' selected' : ''}`,
          x: placed.x,
          y: placed.y,
          width: NODE_W,
          height: NODE_H,
          rx: 10,
        },
        minimap,
      );
    }
    el('rect', { id: 'mini-view', x: 0, y: 0, width: 0, height: 0 }, minimap);
    updateMinimap();
  }

  function updateMinimap() {
    const rect = minimap.querySelector('#mini-view');
    if (!bounds || !rect) return;
    const frame = svg.getBoundingClientRect();
    const x = -view.x / view.k;
    const y = -view.y / view.k;
    const w = frame.width / view.k;
    const h = frame.height / view.k;
    rect.setAttribute('x', String(x));
    rect.setAttribute('y', String(y));
    rect.setAttribute('width', String(w));
    rect.setAttribute('height', String(h));
    const fits =
      x <= bounds.x && y <= bounds.y && x + w >= bounds.x + bounds.width && y + h >= bounds.y + bounds.height;
    minimap.classList.toggle('visible', !fits);
  }

  /** Moves the view so the minimap point under the pointer is its centre. */
  /** @param {PointerEvent} event */
  function panFromMinimap(event) {
    const matrix = minimap.getScreenCTM();
    if (!matrix) return;
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
    const frame = svg.getBoundingClientRect();
    view.x = frame.width / 2 - point.x * view.k;
    view.y = frame.height / 2 - point.y * view.k;
    moved = true;
    apply();
  }
  minimap.addEventListener('pointerdown', (event) => {
    event.stopPropagation();
    minimap.setPointerCapture(event.pointerId);
    panFromMinimap(event);
  });
  minimap.addEventListener('pointermove', (event) => {
    if (minimap.hasPointerCapture(event.pointerId)) panFromMinimap(event);
  });

  // --------------------------------------------------------------- messages

  window.addEventListener('message', (event) => {
    const message = event.data;
    if (message.type === 'graph') {
      title.textContent = message.title;
      // Drawn with a file that does not parse, when there was no complete
      // graph to keep: say which, and that it is missing from the picture.
      if (message.warning) {
        banner.textContent = `Not every file parses right now, so components are missing from this graph: ${message.warning}`;
        banner.classList.add('visible');
      } else {
        banner.classList.remove('visible');
      }
      render(message.analysis);
      // Edits keep the view where the person left it; a different config, or
      // narrowing to one component's paths, starts framed.
      if (message.refit || !moved) {
        fit();
      }
    } else if (message.type === 'unreadable') {
      // The last graph that could be read stays on screen: a config being
      // typed does not parse most of the time, and a blank panel flickering
      // on every keystroke would be useless exactly when it is watched.
      banner.textContent = `This config does not parse right now, so the graph shows the last version that did: ${message.message}`;
      banner.classList.add('visible');
    }
  });

  vscode.postMessage({ type: 'ready' });
})();
