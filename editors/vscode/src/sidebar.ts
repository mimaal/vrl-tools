import * as vscode from 'vscode';

import { originOf } from './checker';
import type {
  Topology,
  TopologyComponent,
  TopologyFinding,
  TopologyTable,
  VrlChecker,
} from './checker';
import { mainPipeline } from './grouping';
import { affectsPipeline, allPipelines, CONFIG_GLOB, sameFile } from './pipeline';
import type { Pipeline, PipelineChoice } from './pipeline';
import { consumers, destination, evaluationOrder, outputsOf, routeLine, written } from './outputs';
import { readPipeline, rowsOf } from './reading';
import type { Counted } from './reading';
import { describeTable } from './tablefiles';
import { componentNames, REVEAL_COMMAND } from './topology';

/**
 * The Vector view in the activity bar: the pipeline, whatever is in the editor.
 *
 * The graph button in the editor title only exists while a Vector config is
 * the file in front, which is the wrong moment most of the time — the file in
 * front is usually the `.vrl` program whose transform you are writing, and
 * that is exactly when "where do these events come from?" gets asked. This
 * view is always there, reads the same pipeline as the graph does, and every
 * row opens it narrowed to what was clicked.
 *
 * It is a tree rather than a second drawing of the graph. A sidebar is
 * 300 pixels wide; a pipeline laid out left to right is not going to fit in
 * one, and a list of what is in the pipeline — with the problems underneath —
 * is what the shape of the panel is actually good for.
 */
export const VIEW_ID = 'vrl-tools.pipeline';

/** Pick which pipeline the view and the graph are showing. */
export const CHOOSE_COMMAND = 'vrl-tools.choosePipeline';

/** How long to wait after a keystroke before re-reading the pipeline. */
const DEBOUNCE_MS = 400;

/** The file extensions a Vector config has. */
const CONFIG_FILE = /\.(ya?ml|toml|json)$/i;

/** The roles events travel through, in the order they do. */
const ROLES = [
  { role: 'source', title: 'Sources', icon: 'symbol-event', colour: 'charts.green' },
  { role: 'transform', title: 'Transforms', icon: 'wand', colour: 'charts.blue' },
  { role: 'sink', title: 'Sinks', icon: 'export', colour: 'charts.purple' },
] as const;

/** How a table is drawn. It is not one of the roles above: nothing flows through most. */
const TABLE = { icon: 'table', colour: 'charts.orange' } as const;

type Node =
  | { readonly kind: 'selector' }
  | { readonly kind: 'group'; readonly role: (typeof ROLES)[number] }
  | { readonly kind: 'problems' }
  /** Every enrichment table of the pipeline, in one row. */
  | { readonly kind: 'tables' }
  /** The tables one file declares. */
  | { readonly kind: 'tableFile'; readonly file: number }
  | { readonly kind: 'table'; readonly table: TopologyTable }
  | { readonly kind: 'component'; readonly component: TopologyComponent }
  /** One output of a component; `null` is its default one. */
  | {
      readonly kind: 'output';
      readonly component: TopologyComponent;
      readonly output: string | null;
    }
  | { readonly kind: 'finding'; readonly finding: TopologyFinding };

/** How each severity is drawn: the icon and its colour. */
const SEVERITY = {
  error: { icon: 'error', colour: 'list.errorForeground' },
  warning: { icon: 'warning', colour: 'list.warningForeground' },
  info: { icon: 'info', colour: 'charts.blue' },
} as const;

/** What one read of the pipeline produced. */
interface Read {
  readonly pipeline: Pipeline;
  readonly analysis: Topology;
  /**
   * Each component's own findings, by ID, worked out once per read.
   *
   * The tree asks for them twice per component — once to decide whether the
   * row has children, once to draw it — and every ask used to walk the whole
   * findings list. On a pipeline with a hundred components and a few dozen
   * problems that is thousands of range comparisons per redraw, for an answer
   * that cannot change until the next read.
   */
  readonly findingsOf: ReadonlyMap<string, TopologyFinding[]>;
  /** Who reads each output, by its written name. See `./outputs.ts`. */
  readonly consumers: ReadonlyMap<string, string[]>;
}

/**
 * Attaches each finding to the component it points inside: at its
 * declaration, or at one of its inputs. That is how the graph colours its
 * boxes, and the same rule keeps the two agreeing.
 */
function attachFindings(analysis: Topology): Map<string, TopologyFinding[]> {
  const byComponent = new Map<string, TopologyFinding[]>();
  for (const component of analysis.components) {
    const own = analysis.findings.filter(
      (finding) =>
        (finding.file === component.file && covers(component.range, finding.range)) ||
        component.inputs.some(
          (input) => finding.file === input.file && covers(input.range, finding.range),
        ),
    );
    if (own.length > 0) {
      byComponent.set(component.id, own);
    }
  }
  return byComponent;
}

export function registerSidebar(
  checker: VrlChecker,
  output: vscode.OutputChannel,
  choice: PipelineChoice,
): vscode.Disposable[] {
  const provider = new PipelineTree(checker, output, choice);
  const view = vscode.window.createTreeView(VIEW_ID, {
    treeDataProvider: provider,
    showCollapseAll: true,
  });

  const watcher = vscode.workspace.createFileSystemWatcher(CONFIG_GLOB);
  const refresh = (): void => provider.schedule();

  return [
    provider,
    view,
    watcher,
    watcher.onDidCreate(refresh),
    watcher.onDidDelete(refresh),
    watcher.onDidChange(refresh),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (provider.belongs(event.document)) {
        provider.schedule();
      }
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (affectsPipeline(event)) {
        provider.schedule();
      }
    }),
    vscode.workspace.onDidChangeWorkspaceFolders(refresh),
    choice.onDidChange(() => provider.refresh()),
    vscode.commands.registerCommand(CHOOSE_COMMAND, () => provider.choosePipeline()),
  ];
}

class PipelineTree implements vscode.TreeDataProvider<Node>, vscode.Disposable {
  private read: Read | undefined;
  /** The read in flight, so a burst of edits does not land out of order. */
  private generation = 0;
  private pending: NodeJS.Timeout | undefined;
  private readonly emitter = new vscode.EventEmitter<Node | undefined>();

  readonly onDidChangeTreeData = this.emitter.event;

  /**
   * Where the pipeline on screen sits among the workspace's, one-based, and
   * how many there are. The count decides whether a choice is offered at all.
   */
  private at = { index: 0, count: 0 };

  /**
   * The rows of the tables whose file has been opened in the tree, by table.
   * Counted when a file's tables are unfolded and not before: 372 tables are
   * 372 files to look for, and the group opens folded.
   */
  private rows = new Map<string, Counted>();

  constructor(
    private readonly checker: VrlChecker,
    private readonly output: vscode.OutputChannel,
    private readonly choice: PipelineChoice,
  ) {}

  refresh(): void {
    this.emitter.fire(undefined);
  }

  /**
   * Offers the workspace's pipelines and remembers the answer.
   *
   * A quick pick rather than rows in the tree: the list is a thing you open,
   * decide and close, and it would otherwise sit above the components taking
   * up the space they need.
   */
  async choosePipeline(): Promise<void> {
    const all = await allPipelines(componentNames(this.checker));
    if (all.length === 0) {
      return;
    }

    const picked = await vscode.window.showQuickPick(
      all.map((pipeline) => ({
        label: pipeline.title,
        description: `${pipeline.files.length} file${pipeline.files.length === 1 ? '' : 's'}`,
        detail: pipeline.files.map((file) => file.name).join(', '),
        key: pipeline.key,
      })),
      {
        title: 'Which pipeline?',
        placeHolder: 'Files whose component names clash cannot be one config, so they are listed apart',
      },
    );

    if (picked) {
      this.choice.set(picked.key);
    }
  }

  /**
   * Whether an edit to `document` can change what this view shows.
   *
   * Any config file counts, not only the ones already in the pipeline: a file
   * joins the pipeline the moment its first `sources:` is typed, and that edit
   * is the one that has to be noticed. The watcher cannot see it, because
   * nothing has been saved.
   */
  belongs(document: vscode.TextDocument): boolean {
    return (
      CONFIG_FILE.test(document.uri.path) ||
      (this.read?.pipeline.files.some((file) => sameFile(file.uri, document.uri)) ?? false)
    );
  }

  schedule(): void {
    clearTimeout(this.pending);
    this.pending = setTimeout(() => this.emitter.fire(undefined), DEBOUNCE_MS);
  }

  async getChildren(node?: Node): Promise<Node[]> {
    if (!node) {
      return this.roots();
    }

    const analysis = this.read?.analysis;
    if (!analysis) {
      return [];
    }

    switch (node.kind) {
      case 'selector':
        return [];
      case 'group':
        return analysis.components
          .filter((component) => component.role === node.role.role)
          .map((component) => ({ kind: 'component', component }));
      case 'problems':
        return analysis.findings.map((finding) => ({ kind: 'finding', finding }));
      case 'tables': {
        // By the file that declares them: that is how they are maintained,
        // and three rows of files open faster than 372 of tables. The source
        // half of a `memory` table is a component like any other, so it is
        // listed as one.
        const halves = analysis.components
          .filter((component) => component.role === 'table' && hasOutputs(component))
          .map((component): Node => ({ kind: 'component', component }));
        const files = [...new Set(analysis.tables.map((table) => table.file))];
        const tables: Node[] =
          analysis.files.length > 1
            ? files.map((file): Node => ({ kind: 'tableFile', file }))
            : await this.tablesOf(analysis.tables);
        return [...tables, ...halves];
      }
      case 'tableFile':
        return this.tablesOf(analysis.tables.filter((table) => table.file === node.file));
      case 'table':
        return [];
      // A component with more than one way out lists them, each saying who
      // reads it and going to the file that added it — for a router written
      // across files, not the one that declares it. One with a single output
      // says where it goes on its own row instead. Then the component's own
      // problems, so a row with a badge can be opened to see what the badge
      // is about without hunting for it in the list below.
      case 'component':
        return [
          ...(node.component.namedOutputs.length > 0
            ? outputsOf(node.component).map(
                (output): Node => ({ kind: 'output', component: node.component, output }),
              )
            : []),
          ...this.findingsOf(node.component).map((finding): Node => ({ kind: 'finding', finding })),
        ];
      case 'output':
      case 'finding':
        return [];
    }
  }

  getTreeItem(node: Node): vscode.TreeItem {
    switch (node.kind) {
      case 'selector':
        return this.selector();
      case 'group':
        return this.group(node.role);
      case 'problems':
        return this.problems();
      case 'tables':
        return this.tables();
      case 'tableFile':
        return this.tableFile(node.file);
      case 'table':
        return this.table(node.table);
      case 'component':
        return this.component(node.component);
      case 'output':
        return this.namedOutput(node.component, node.output);
      case 'finding':
        return this.finding(node.finding);
    }
  }

  /**
   * The groups that have anything in them, plus the problems.
   *
   * An empty result is what puts the "no Vector configuration here" welcome
   * message on screen, so a workspace with nothing to show says so rather than
   * showing four empty folders.
   */
  private async roots(): Promise<Node[]> {
    const generation = ++this.generation;
    const read = await this.reload();
    if (generation !== this.generation) {
      return [];
    }
    this.read = read;

    if (!read || read.analysis.components.length === 0) {
      return [];
    }

    const nodes: Node[] = [];
    // Offered only when there is something to choose between; one pipeline
    // needs no row saying so.
    if (this.at.count > 1) {
      nodes.push({ kind: 'selector' });
    }
    nodes.push(
      ...ROLES.filter((role) =>
        read.analysis.components.some((component) => component.role === role.role),
      ).map((role): Node => ({ kind: 'group', role })),
    );
    if (read.analysis.components.some((component) => component.role === 'table')) {
      nodes.push({ kind: 'tables' });
    }

    if (read.analysis.findings.length > 0) {
      nodes.push({ kind: 'problems' });
    }
    return nodes;
  }

  private async reload(): Promise<Read | undefined> {
    try {
      const names = componentNames(this.checker);
      const all = await allPipelines(names);
      const chosen = all.findIndex((entry) => entry.key === this.choice.key);
      // Nothing chosen, or a remembered choice naming a pipeline that has
      // since been renamed or split differently: the main one, not the first
      // alphabetically. See `mainPipeline`.
      const index = chosen >= 0 ? chosen : Math.max(0, mainPipeline(all, names));
      this.at = { index: index + 1, count: all.length };
      const pipeline = all[index];
      if (!pipeline) {
        return undefined;
      }
      const analysis = await readPipeline(this.checker, pipeline);
      this.rows = new Map();
      return {
        pipeline,
        analysis,
        findingsOf: attachFindings(analysis),
        consumers: consumers(analysis.edges),
      };
    } catch (error) {
      this.output.appendLine(`Reading the pipeline for the sidebar failed: ${String(error)}`);
      return undefined;
    }
  }

  /** Whether an output, as written, is marked as ending on purpose. */
  private terminal(written: string): boolean {
    return this.read?.analysis.terminal.includes(written) ?? false;
  }

  private findingsOf(component: TopologyComponent): TopologyFinding[] {
    return this.read?.findingsOf.get(component.id) ?? [];
  }

  private selector(): vscode.TreeItem {
    const item = new vscode.TreeItem(
      `Pipeline: ${this.read?.pipeline.title ?? ''}`,
      vscode.TreeItemCollapsibleState.None,
    );
    item.description = `${this.at.index} of ${this.at.count}`;
    item.iconPath = new vscode.ThemeIcon('list-selection');
    item.tooltip =
      'This workspace holds more than one Vector pipeline. Files whose component names clash cannot be one config, so they are kept apart.';
    item.command = { command: CHOOSE_COMMAND, title: 'Choose a pipeline' };
    item.contextValue = 'vrl-tools.selector';
    return item;
  }

  private group(role: (typeof ROLES)[number]): vscode.TreeItem {
    const count = (this.read?.analysis.components ?? []).filter((c) => c.role === role.role).length;
    const item = new vscode.TreeItem(role.title, vscode.TreeItemCollapsibleState.Expanded);
    item.description = `${count}`;
    item.iconPath = new vscode.ThemeIcon(role.icon, new vscode.ThemeColor(role.colour));
    item.contextValue = 'vrl-tools.group';
    return item;
  }

  /**
   * The table rows for `tables`, the ones this pipeline reads first, with
   * their CSVs counted where they can be found.
   */
  private async tablesOf(tables: readonly TopologyTable[]): Promise<Node[]> {
    const pipeline = this.read?.pipeline;
    if (pipeline) {
      const counted = await Promise.all(tables.map((table) => rowsOf(pipeline, table)));
      tables.forEach((table, index) => {
        const rows = counted[index];
        if (rows) {
          this.rows.set(table.id, rows);
        }
      });
    }
    const read = (table: TopologyTable): number => (table.readers.length > 0 ? 0 : 1);
    return [...tables]
      .sort((a, b) => read(a) - read(b))
      .map((table): Node => ({ kind: 'table', table }));
  }

  /**
   * One row for every table there is. Folded: a pipeline has few components
   * and can have hundreds of tables, and none of them is on the way anywhere.
   */
  private tables(): vscode.TreeItem {
    const analysis = this.read?.analysis;
    const tables = analysis?.tables ?? [];
    const read = tables.filter((table) => table.readers.length > 0).length;

    const item = new vscode.TreeItem(
      `Enrichment tables (${tables.length})`,
      vscode.TreeItemCollapsibleState.Collapsed,
    );
    item.description = `${read} read here`;
    item.iconPath = new vscode.ThemeIcon(TABLE.icon, new vscode.ThemeColor(TABLE.colour));
    item.tooltip = new vscode.MarkdownString(
      [
        `${read} of ${tables.length} are looked up by this pipeline's VRL, by a literal name in \`find_enrichment_table_records\` or \`get_enrichment_table_record\`.`,
        'A table nothing here reads is not a problem: table files are shared across pipelines, and Vector says nothing about one.',
        ...this.unknowns(),
      ].join('\n\n'),
    );
    item.contextValue = 'vrl-tools.tables';
    return item;
  }

  /**
   * What keeps "unused in this pipeline" from being certain, said rather
   * than hidden: a lookup through a variable, a program that is not here.
   */
  private unknowns(): string[] {
    const analysis = this.read?.analysis;
    if (!analysis) {
      return [];
    }
    const missing = analysis.programs.filter((program) => !program.read);
    return [
      ...(analysis.opaqueLookups.length > 0
        ? [
            `${analysis.opaqueLookups.map((id) => `\`${id}\``).join(', ')} name a table through a variable, which only the compiler can follow, so a table shown as unused may be read after all.`,
          ]
        : []),
      ...missing.map(
        (program) =>
          `\`${program.path}\`, the program of \`${program.component}\`, is not in this workspace, so what it looks up is unknown.`,
      ),
    ];
  }

  private tableFile(file: number): vscode.TreeItem {
    const analysis = this.read?.analysis;
    const tables = (analysis?.tables ?? []).filter((table) => table.file === file);
    const read = tables.filter((table) => table.readers.length > 0).length;
    const name = analysis?.files[file] ?? '';

    const item = new vscode.TreeItem(
      name.split('/').pop() ?? name,
      vscode.TreeItemCollapsibleState.Collapsed,
    );
    item.description = `${tables.length} · ${read} read here`;
    item.iconPath = vscode.ThemeIcon.File;
    item.resourceUri = this.read?.pipeline.files[file]?.uri;
    item.tooltip = name;
    item.contextValue = 'vrl-tools.tableFile';
    return item;
  }

  private table(table: TopologyTable): vscode.TreeItem {
    const tables = this.read?.analysis.tables ?? [];
    const files = this.read?.analysis.files ?? [];
    // 372 rows each saying `file` say nothing; the type is worth a word only
    // when it tells two tables apart.
    const showType = !tables.every((other) => other.type === 'file');
    const counted = this.rows.get(table.id);
    const unused = table.readers.length === 0;

    const item = new vscode.TreeItem(table.id, vscode.TreeItemCollapsibleState.None);
    item.description = describeTable(table, showType, counted?.rows);
    // Dimmed, not flagged: unused here is information, never a warning.
    item.iconPath = new vscode.ThemeIcon(
      TABLE.icon,
      new vscode.ThemeColor(unused ? 'disabledForeground' : TABLE.colour),
    );
    item.tooltip = new vscode.MarkdownString(
      [
        `**${table.id}** — enrichment table, \`${table.type || 'no type'}\``,
        unused
          ? 'Unused in this pipeline: no VRL here looks it up by name.'
          : `Read by ${table.readers.map((reader) => `\`${reader}\``).join(', ')}.`,
        ...(table.path ? [`Data: \`${table.path}\`, as the config writes it.`] : []),
        ...(counted
          ? [`${counted.rows} row${counted.rows === 1 ? '' : 's'}, counted in \`${counted.counted}\`.`]
          : table.type === 'file' && table.path
            ? ['Not found in this workspace, so its rows are not counted.']
            : []),
        ...(files.length > 1 ? [files[table.file] ?? ''] : []),
        ...(unused ? this.unknowns() : []),
      ].join('\n\n'),
    );

    const file = this.read?.pipeline.files[table.file];
    if (file) {
      item.command = {
        command: 'vscode.open',
        title: 'Go to the table',
        arguments: [
          file.uri,
          {
            selection: new vscode.Range(
              table.range.start.line,
              table.range.start.character,
              table.range.end.line,
              table.range.end.character,
            ),
          },
        ],
      };
    }
    item.contextValue = 'vrl-tools.table';
    return item;
  }

  private problems(): vscode.TreeItem {
    const findings = this.read?.analysis.findings ?? [];
    const count = (severity: TopologyFinding['severity']) =>
      findings.filter((finding) => finding.severity === severity).length;
    const [errors, warnings, notes] = [count('error'), count('warning'), count('info')];

    const item = new vscode.TreeItem(
      'Problems',
      // Collapsed: it is the part you look at when something is wrong, not the
      // part you want unfolded over the components every time.
      vscode.TreeItemCollapsibleState.Collapsed,
    );
    // Both counts, because they mean different things: an error is Vector
    // refusing to start, a warning is Vector running and throwing events
    // away. "2 errors" next to a list of three hides the one you can miss.
    // A note is neither: something the editor cannot know.
    item.description = [
      ...(errors > 0 ? [`${errors} error${errors === 1 ? '' : 's'}`] : []),
      ...(warnings > 0 ? [`${warnings} warning${warnings === 1 ? '' : 's'}`] : []),
      ...(notes > 0 ? [`${notes} note${notes === 1 ? '' : 's'}`] : []),
    ].join(', ');
    const worst = SEVERITY[errors > 0 ? 'error' : warnings > 0 ? 'warning' : 'info'];
    item.iconPath = new vscode.ThemeIcon(worst.icon, new vscode.ThemeColor(worst.colour));
    item.contextValue = 'vrl-tools.problems';
    return item;
  }

  private component(component: TopologyComponent): vscode.TreeItem {
    const own = this.findingsOf(component);
    const role = ROLES.find((entry) => entry.role === component.role) ?? TABLE;
    const files = this.read?.analysis.files ?? [];

    const item = new vscode.TreeItem(
      component.id,
      own.length > 0 || component.namedOutputs.length > 0
        ? vscode.TreeItemCollapsibleState.Collapsed
        : vscode.TreeItemCollapsibleState.None,
    );

    // The type, then where its events go: named outright when there is one
    // way out, counted when there are several, since each then has a row of
    // its own underneath. A sink, or a table, has nowhere to say.
    const outputs = outputsOf(component);
    const going =
      outputs.length === 0
        ? ''
        : outputs.length === 1 && outputs[0] === null
          ? ` ${destination(this.read?.consumers.get(component.id), this.terminal(component.id))}`
          : ` · ${outputs.length} outputs`;
    item.description = `${component.type || 'no type'}${going}`;

    item.iconPath = new vscode.ThemeIcon(
      role.icon,
      new vscode.ThemeColor(
        own.some((finding) => finding.severity === 'error')
          ? 'list.errorForeground'
          : own.some((finding) => finding.severity === 'warning')
            ? 'list.warningForeground'
            : role.colour,
      ),
    );

    item.tooltip = new vscode.MarkdownString(
      [
        `**${component.id}** — ${component.role}, \`${component.type || 'no type'}\``,
        ...(files.length > 1 ? [`\n\n${files[component.file] ?? ''}`] : []),
        // The order its routes are tried in, with the file each comes from:
        // the first to match takes the event.
        ...[...evaluationOrder(component)].map(
          ([output, order]) =>
            `\n\n${routeLine(order, output, files[originOf(component, output).file] ?? '')}`,
        ),
        ...own.map((finding) => `\n\n- ${finding.severity}: ${finding.message}`),
      ].join(''),
    );

    const file = this.read?.pipeline.files[component.file];
    if (file) {
      item.command = {
        command: REVEAL_COMMAND,
        title: 'Go to the component and follow its paths',
        arguments: [{ uri: file.uri, range: component.range, id: component.id }],
      };
    }
    item.contextValue = 'vrl-tools.component';
    return item;
  }

  /**
   * One named output, opening the file that added it: `route_by_product` is
   * declared in `topology.toml`, but its `cloudflare-waf` route is in
   * `cloudflare_waf.toml`, and that is where somebody clicking the route wants
   * to be.
   */
  private namedOutput(component: TopologyComponent, output: string | null): vscode.TreeItem {
    const files = this.read?.analysis.files ?? [];
    const origin =
      output === null
        ? { file: component.file, range: component.range }
        : originOf(component, output);
    const file = this.read?.pipeline.files[origin.file];
    const name = written(component.id, output);
    const readers = this.read?.consumers.get(name);
    const place =
      files.length > 1
        ? `${(files[origin.file] ?? '').split('/').pop() ?? ''}:${origin.range.start.line + 1}`
        : `line ${origin.range.start.line + 1}`;

    // The whole name, not the port: three routers each have a `_unmatched`,
    // and this is the string an input would be written with.
    // An exclusive_route takes the first route that matches, so a route's
    // turn is part of what it means; said first, since it is the order the
    // rows are in.
    const order = output === null ? undefined : evaluationOrder(component).get(output);
    const tried =
      order === undefined
        ? ''
        : ` Tried ${order} of ${evaluationOrder(component).size}, in the order the files merge.`;

    const ends = this.terminal(name);
    const fate = readers
      ? `Read by ${readers.join(', ')}.`
      : ends
        ? 'Nothing reads it, and it is marked as ending here on purpose.'
        : 'Nothing reads it.';
    const item = new vscode.TreeItem(order === undefined ? name : `${order}. ${name}`);
    item.description = `${destination(readers, ends)} · ${place}`;
    // An end that is meant is not a warning, and is not drawn as one.
    item.iconPath = readers
      ? new vscode.ThemeIcon('arrow-right')
      : ends
        ? new vscode.ThemeIcon('debug-stop', new vscode.ThemeColor('descriptionForeground'))
        : new vscode.ThemeIcon('circle-slash', new vscode.ThemeColor('list.warningForeground'));
    item.tooltip =
      output === null
        ? `The default output of \`${component.id}\`: what an input naming \`${component.id}\` reads. ${fate}`
        : `Output \`${output}\` of \`${component.id}\`, added in ${files[origin.file] ?? 'this file'}.${tried} ${fate}`;
    if (file) {
      item.command = {
        command: 'vscode.open',
        title: 'Go to where the output is added',
        arguments: [
          file.uri,
          {
            selection: new vscode.Range(
              origin.range.start.line,
              origin.range.start.character,
              origin.range.end.line,
              origin.range.end.character,
            ),
          },
        ],
      };
    }
    item.contextValue = 'vrl-tools.output';
    return item;
  }

  private finding(finding: TopologyFinding): vscode.TreeItem {
    const files = this.read?.analysis.files ?? [];
    const file = this.read?.pipeline.files[finding.file];

    // Backticks are the message's own emphasis; in a tree row they are noise.
    const item = new vscode.TreeItem(finding.message.replaceAll('`', ''));
    item.description =
      files.length > 1
        ? `${files[finding.file] ?? ''}:${finding.range.start.line + 1}`
        : `line ${finding.range.start.line + 1}`;
    const drawn = SEVERITY[finding.severity] ?? SEVERITY.warning;
    item.iconPath = new vscode.ThemeIcon(drawn.icon, new vscode.ThemeColor(drawn.colour));
    item.tooltip = finding.message.replaceAll('`', '');

    if (file) {
      item.command = {
        command: 'vscode.open',
        title: 'Go to the problem',
        arguments: [
          file.uri,
          {
            selection: new vscode.Range(
              finding.range.start.line,
              finding.range.start.character,
              finding.range.end.line,
              finding.range.end.character,
            ),
          },
        ],
      };
    }
    item.contextValue = 'vrl-tools.finding';
    return item;
  }

  dispose(): void {
    clearTimeout(this.pending);
    this.emitter.dispose();
  }
}

/** Whether anything can read `component`: the source half of a `memory` table can. */
function hasOutputs(component: TopologyComponent): boolean {
  return component.defaultOutput || component.namedOutputs.length > 0;
}

/** Whether `outer` contains `inner`, by line and character. */
function covers(
  outer: { start: { line: number; character: number }; end: { line: number; character: number } },
  inner: { start: { line: number; character: number }; end: { line: number; character: number } },
): boolean {
  const after =
    inner.start.line > outer.start.line ||
    (inner.start.line === outer.start.line && inner.start.character >= outer.start.character);
  const before =
    inner.end.line < outer.end.line ||
    (inner.end.line === outer.end.line && inner.end.character <= outer.end.character);
  return after && before;
}
