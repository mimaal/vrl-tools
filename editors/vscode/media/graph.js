// @ts-check
/*
 * Draws a Vector pipeline inside the graph panel.
 *
 * Everything that is a decision about the graph — which component feeds
 * which, through which output, what is wrong, which column and row each one
 * belongs in — arrives already made, from the wasm module's topology reader
 * (`crates/vector-topology`). This file turns that into pixels, and handles
 * the pointer. It owns no knowledge of Vector.
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
  const LANE_H = 8;
  const PAD = 48;
  const MIN_ZOOM = 0.2;
  const MAX_ZOOM = 3;

  const svg = /** @type {SVGSVGElement} */ (document.querySelector('#canvas'));
  const viewport = /** @type {SVGGElement} */ (document.querySelector('#viewport'));
  const edgeLayer = /** @type {SVGGElement} */ (document.querySelector('#edges'));
  const labelLayer = /** @type {SVGGElement} */ (document.querySelector('#labels'));
  const nodeLayer = /** @type {SVGGElement} */ (document.querySelector('#nodes'));
  const title = /** @type {HTMLElement} */ (document.getElementById('title'));
  const summary = /** @type {HTMLElement} */ (document.getElementById('summary'));
  const banner = /** @type {HTMLElement} */ (document.getElementById('banner'));
  const empty = /** @type {HTMLElement} */ (document.getElementById('empty'));
  const problems = /** @type {HTMLElement} */ (document.getElementById('problems'));
  const problemList = /** @type {HTMLElement} */ (document.getElementById('problem-list'));

  /** @typedef {{edge: any, group: SVGGElement, label?: SVGGElement}} Drawn */

  /** The pan and zoom, applied to `viewport`. */
  const view = { x: 0, y: 0, k: 1 };
  /** Whether the person has panned or zoomed; until then a resize refits. */
  let moved = false;

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

  /** A key for two component IDs, which can hold any character but NUL. */
  /** @param {string} from @param {string} to */
  function pair(from, to) {
    return `${from}\u0000${to}`;
  }

  /**
   * The files the pipeline on screen was read from. A component's or a
   * finding's `file` is an index into this.
   * @type {string[]}
   */
  let files = [];

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

  // -------------------------------------------------------------- rendering

  /** @param {any} analysis */
  function render(analysis) {
    edgeLayer.replaceChildren();
    labelLayer.replaceChildren();
    nodeLayer.replaceChildren();
    svg.classList.remove('tracing');

    const components = /** @type {any[]} */ (analysis.components);
    const edges = /** @type {any[]} */ (analysis.edges);
    const findings = /** @type {any[]} */ (analysis.findings);
    const layout = /** @type {{components: any[], routes: any[]}} */ (analysis.layout);
    files = analysis.files ?? [];

    renderSummary(components, edges);
    renderProblems(findings);
    empty.classList.toggle('visible', components.length === 0);
    if (components.length === 0) {
      return;
    }

    // Each column is a stack of boxes and lanes in the layout's row order. A
    // lane is only as tall as the arrow passing through it. Columns are then
    // centred on the tallest, so a pipeline that fans out and back in reads
    // as a shape rather than a staircase.
    /** @type {Map<number, {row: number, height: number, key: string}[]>} */
    const stacks = new Map();
    const stack = (/** @type {number} */ column, /** @type {any} */ item) =>
      stacks.set(column, [...(stacks.get(column) ?? []), item]);
    for (const place of layout.components) {
      stack(place.column, { row: place.row, height: NODE_H, key: `c${place.component}` });
    }
    for (const route of layout.routes) {
      for (const slot of route.via) {
        stack(slot.column, { row: slot.row, height: LANE_H, key: `l${slot.column}:${slot.row}` });
      }
    }

    /** @type {Map<string, number>} the top of every box and lane in its column */
    const top = new Map();
    /** @type {Map<number, number>} */
    const heights = new Map();
    let tallest = 0;
    for (const [column, items] of stacks) {
      items.sort((a, b) => a.row - b.row);
      let y = 0;
      for (const item of items) {
        top.set(item.key, y);
        y += item.height + ROW_GAP;
      }
      heights.set(column, y - ROW_GAP);
      tallest = Math.max(tallest, y - ROW_GAP);
    }
    const columnX = (/** @type {number} */ column) => PAD + column * (NODE_W + COL_GAP);
    const shift = (/** @type {number} */ column) =>
      PAD + (tallest - (heights.get(column) ?? 0)) / 2;

    /** @type {Map<string, {x: number, y: number, component: any}>} */
    const at = new Map();
    for (const place of layout.components) {
      const component = components[place.component];
      at.set(component.id, {
        x: columnX(place.column),
        y: shift(place.column) + (top.get(`c${place.component}`) ?? 0),
        component,
      });
    }

    /** @type {Map<string, {x: number, y: number}[]>} lane centres, per pair of components */
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
    drawNodes(at, findingsOf, edges, drawn);
  }

  /**
   * @param {any[]} edges
   * @param {Map<string, {x: number, y: number, component: any}>} at
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
        el(
          'path',
          { d, 'marker-end': backwards ? 'url(#arrow-error)' : 'url(#arrow)' },
          group,
        )
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
   * @param {Map<string, {x: number, y: number, component: any}>} at
   * @param {Map<string, any[]>} findingsOf
   * @param {any[]} edges
   * @param {Drawn[]} drawn
   */
  function drawNodes(at, findingsOf, edges, drawn) {
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
      ].join('\n');

      el('rect', { class: 'card', width: NODE_W, height: NODE_H, rx: 8, ry: 8 }, group);
      el('rect', { class: 'accent', x: 9, y: 12, width: 3, height: NODE_H - 24, rx: 1.5 }, group);

      const role = el('text', { class: 'role', x: 22, y: 20 }, group);
      role.textContent = component.role;

      const name = /** @type {SVGTextElement} */ (el('text', { class: 'id', x: 22, y: 38 }, group));
      fitText(name, id, NODE_W - 36);

      const type = /** @type {SVGTextElement} */ (el('text', { class: 'type', x: 22, y: 53 }, group));
      // With several files, the one declaring it, where the type has room.
      const declared = files.length > 1 ? ` · ${(files[component.file] ?? '').split('/').pop()}` : '';
      fitText(type, `${component.type || '(no type)'}${declared}`, NODE_W - 36);

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

      group.addEventListener('click', () => reveal(component.range, component.file));
      group.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          reveal(component.range, component.file);
        }
      });
      group.addEventListener('mouseenter', () => trace(id, edges, groups, drawn));
      group.addEventListener('focus', () => trace(id, edges, groups, drawn));
      group.addEventListener('mouseleave', untrace);
      group.addEventListener('blur', untrace);
    }
  }

  /**
   * Lights up every path events can take through `id`: what can reach it,
   * and what it can reach.
   *
   * @param {string} id
   * @param {any[]} edges
   * @param {Map<string, SVGGElement>} groups
   * @param {Drawn[]} drawn
   */
  function trace(id, edges, groups, drawn) {
    /** @param {(e: any) => string} near @param {(e: any) => string} far */
    const walk = (near, far) => {
      const seen = new Set([id]);
      const queue = [id];
      while (queue.length > 0) {
        const current = queue.shift();
        for (const edge of edges) {
          if (near(edge) === current && !seen.has(far(edge))) {
            seen.add(far(edge));
            queue.push(far(edge));
          }
        }
      }
      return seen;
    };
    const upstream = walk((e) => e.to, (e) => e.from);
    const downstream = walk((e) => e.from, (e) => e.to);

    for (const [other, group] of groups) {
      group.classList.toggle('on-path', upstream.has(other) || downstream.has(other));
    }
    for (const { edge, group, label } of drawn) {
      const onPath =
        (upstream.has(edge.from) && upstream.has(edge.to)) ||
        (downstream.has(edge.from) && downstream.has(edge.to));
      group.classList.toggle('on-path', onPath);
      label?.classList.toggle('on-path', onPath);
      group
        .querySelector('path')
        ?.setAttribute(
          'marker-end',
          group.classList.contains('backwards')
            ? 'url(#arrow-error)'
            : onPath
              ? 'url(#arrow-active)'
              : 'url(#arrow)',
        );
    }
    svg.classList.add('tracing');
  }

  function untrace() {
    svg.classList.remove('tracing');
    for (const path of edgeLayer.querySelectorAll('.edge:not(.backwards) path')) {
      path.setAttribute('marker-end', 'url(#arrow)');
    }
  }

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
  }

  function fit() {
    const box = viewport.getBBox();
    const frame = svg.getBoundingClientRect();
    if (box.width === 0 || frame.width === 0) {
      return;
    }
    const k = Math.min(
      (frame.width - PAD) / box.width,
      (frame.height - PAD) / box.height,
      1.25,
    );
    view.k = Math.max(MIN_ZOOM, k);
    view.x = (frame.width - box.width * view.k) / 2 - box.x * view.k;
    view.y = (frame.height - box.height * view.k) / 2 - box.y * view.k;
    moved = false;
    apply();
  }

  svg.addEventListener(
    'wheel',
    (event) => {
      event.preventDefault();
      const frame = svg.getBoundingClientRect();
      const px = event.clientX - frame.left;
      const py = event.clientY - frame.top;
      const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.k * Math.exp(-event.deltaY * 0.0015)));
      // Zoom about the pointer, so what is under it stays under it.
      view.x = px - ((px - view.x) * k) / view.k;
      view.y = py - ((py - view.y) * k) / view.k;
      view.k = k;
      moved = true;
      apply();
    },
    { passive: false },
  );

  /** @type {{x: number, y: number} | undefined} */
  let grab;
  svg.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || /** @type {Element} */ (event.target).closest('.node')) {
      return;
    }
    grab = { x: event.clientX - view.x, y: event.clientY - view.y };
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
  const release = () => {
    grab = undefined;
    svg.classList.remove('panning');
  };
  svg.addEventListener('pointerup', release);
  svg.addEventListener('pointercancel', release);

  new ResizeObserver(() => {
    if (!moved) fit();
  }).observe(svg);

  document.getElementById('fit')?.addEventListener('click', fit);
  document.getElementById('export')?.addEventListener('click', () => {
    vscode.postMessage({ type: 'export' });
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
      // Edits keep the view where the person left it; a different config
      // starts framed.
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
