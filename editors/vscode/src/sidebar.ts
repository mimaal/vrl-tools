import * as vscode from 'vscode';

import { originOf } from './checker';
import type { Topology, TopologyComponent, TopologyFinding, VrlChecker } from './checker';
import { mainPipeline } from './grouping';
import { affectsPipeline, allPipelines, CONFIG_GLOB, sameFile } from './pipeline';
import type { Pipeline, PipelineChoice } from './pipeline';
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

/** The roles, in the order events travel through them. */
const ROLES = [
  { role: 'source', title: 'Sources', icon: 'symbol-event', colour: 'charts.green' },
  { role: 'transform', title: 'Transforms', icon: 'wand', colour: 'charts.blue' },
  { role: 'sink', title: 'Sinks', icon: 'export', colour: 'charts.purple' },
  { role: 'table', title: 'Enrichment tables', icon: 'table', colour: 'charts.orange' },
] as const;

type Node =
  | { readonly kind: 'selector' }
  | { readonly kind: 'group'; readonly role: (typeof ROLES)[number] }
  | { readonly kind: 'problems' }
  | { readonly kind: 'component'; readonly component: TopologyComponent }
  | { readonly kind: 'output'; readonly component: TopologyComponent; readonly output: string }
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
      // A router's outputs hang off it, each going to the file that added it
      // — for a router written across files, not the one that declares it.
      // Then the component's own problems, so a row with a badge can be
      // opened to see what the badge is about without hunting for it in the
      // list below.
      case 'component':
        return [
          ...node.component.namedOutputs.map(
            (output): Node => ({ kind: 'output', component: node.component, output }),
          ),
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
      const analysis = this.checker.topologyFiles(
        pipeline.files.map((file) => ({
          name: file.name,
          source: file.source,
          standalone: file.standalone,
        })),
        pipeline.title,
      );
      return { pipeline, analysis, findingsOf: attachFindings(analysis) };
    } catch (error) {
      this.output.appendLine(`Reading the pipeline for the sidebar failed: ${String(error)}`);
      return undefined;
    }
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
    const role = ROLES.find((entry) => entry.role === component.role);
    const files = this.read?.analysis.files ?? [];

    const item = new vscode.TreeItem(
      component.id,
      own.length > 0 || component.namedOutputs.length > 0
        ? vscode.TreeItemCollapsibleState.Collapsed
        : vscode.TreeItemCollapsibleState.None,
    );

    // The type, and the outputs when there is more than one way out: that is
    // the thing a list cannot show and the graph can, so it is worth a word.
    const outputs =
      component.namedOutputs.length > 0
        ? ` · ${[...(component.defaultOutput ? ['(default)'] : []), ...component.namedOutputs].join(', ')}`
        : '';
    item.description = `${component.type || 'no type'}${outputs}`;

    item.iconPath = new vscode.ThemeIcon(
      role?.icon ?? 'circle-outline',
      new vscode.ThemeColor(
        own.some((finding) => finding.severity === 'error')
          ? 'list.errorForeground'
          : own.some((finding) => finding.severity === 'warning')
            ? 'list.warningForeground'
            : (role?.colour ?? 'foreground'),
      ),
    );

    item.tooltip = new vscode.MarkdownString(
      [
        `**${component.id}** — ${component.role}, \`${component.type || 'no type'}\``,
        ...(files.length > 1 ? [`\n\n${files[component.file] ?? ''}`] : []),
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
  private namedOutput(component: TopologyComponent, output: string): vscode.TreeItem {
    const files = this.read?.analysis.files ?? [];
    const origin = originOf(component, output);
    const file = this.read?.pipeline.files[origin.file];

    const item = new vscode.TreeItem(`${component.id}.${output}`);
    item.description =
      files.length > 1
        ? `${(files[origin.file] ?? '').split('/').pop() ?? ''}:${origin.range.start.line + 1}`
        : `line ${origin.range.start.line + 1}`;
    item.iconPath = new vscode.ThemeIcon('arrow-right');
    item.tooltip = `Output \`${output}\` of \`${component.id}\`, added in ${files[origin.file] ?? 'this file'}`;
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
