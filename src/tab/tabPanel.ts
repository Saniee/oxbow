// The SlothGit tab: lazygit's layout in an editor tab. Status, Files, Branches, Commits and Stash panels on the
// left, the selected row's details (its patch, a branch's log…) on the right, with the command log below.
//
// The lists show the providers' tree items as rows. Each row carries its node's kind and state (data-vscode-context),
// so right-clicking it opens VS Code's own menu with SlothGit's commands (package.json webview/context), and the
// commands get the node back through registerWebviewNodes. Hover and header buttons are drawn by the page from
// tab/actions.ts.
import * as path from 'node:path';
import * as vscode from 'vscode';
import { contextValues, onDidChangeContext } from '../contextKeys';
import type { CommandLog } from '../git/commandLog';
import { getBisectState, getCommitMessage, getLocalBranches, getOperation, getRebaseTodo, getRemotes, headSha, type Operation } from '../git/queries';
import { isStaged } from '../git/workingTree';
import type { RepoManager } from '../repoManager';
import type { Runner } from '../runner';
import type { BaseProvider, ProviderHost } from '../views/baseProvider';
import type { BranchesProvider } from '../views/branches';
import type { FilesProvider } from '../views/files';
import type { Node } from '../views/nodes';
import type { CommitsProvider, ReflogProvider, StashProvider } from '../views/providers';
import { registerWebviewNodes, type ListView } from '../views/webviewNodes';
import { pageHtml, webviewOptions } from '../views/webviewShell';
import body from '../webview/tab/body.html';
import type { FromTab, TabAction, TabCommands, TabDetail, TabList, TabRow, TabStatus, ToTab } from '../webview/messages';
import { SYNC_MENUS } from '../webview/syncMenus';
import { BUTTON_COMMANDS, LIST_ACTIONS, ROW_ACTIONS, STATUS_ACTIONS, type HeaderEntry, type ListId } from './actions';
import { buildDetail, detailKey, type Source } from './detail';
import { toRow } from './rows';
import { evaluate } from './when';

const OPERATION_LABELS: Record<Operation, string> = {
  rebase: 'Rebasing',
  'cherry-pick': 'Cherry-picking',
  revert: 'Reverting',
  merge: 'Merging',
};

const EMPTY: Record<ListId, string> = {
  files: 'No changes.',
  local: 'No branches yet.',
  remotes: 'No remotes. Add one to push and pull.',
  tags: 'No tags.',
  worktrees: 'Only the main worktree.',
  commits: 'No commits yet.',
  reflog: 'The reflog is empty.',
  stash: 'No stashes.',
};

/** The kind of list each list's rows come from, as the old tree views were named (menus use it). */
const LIST_VIEWS: Record<ListId, string> = {
  files: 'files',
  local: 'branches',
  remotes: 'branches',
  tags: 'branches',
  worktrees: 'branches',
  commits: 'commits',
  reflog: 'reflog',
  stash: 'stashes',
};

export interface TabProviders {
  files: FilesProvider;
  branches: BranchesProvider;
  commits: CommitsProvider;
  reflog: ReflogProvider;
  stashes: StashProvider;
}

interface Entry extends Source {
  list?: ListId;
}

export class SlothGitTab implements ProviderHost, vscode.Disposable {
  static readonly viewType = 'slothgit.tab';

  private panel: vscode.WebviewPanel | undefined;
  private readonly visibility = new vscode.EventEmitter<boolean>();
  readonly onDidChangeVisibility = this.visibility.event;

  /** Rows on the page, by id: the lists' and the detail pane's. */
  private readonly entries = new Map<string, Entry>();
  private readonly rowIds = new Map<ListId, string[]>();
  private readonly selections = new Map<ListId, string[]>();
  private readonly views = new Map<ListId, ListView>();
  /** What the detail pane shows, and what "Back" returns to. */
  private detail: { source: Source; full: boolean } | undefined;
  private readonly backStack: Source[] = [];
  private detailVersion = 0;
  private statusVersion = 0;
  /** The list whose selection the detail pane last followed. */
  private activeList: ListId | undefined;
  /** Lists in the order they were last selected in, most recent last. */
  private selectOrder: ListId[] = [];
  private contextTimer: NodeJS.Timeout | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly sent = new Map<string, ToTab>();
  /** Rows per list the page last drew, for tests. */
  renderedLists: Partial<Record<ListId, number>> = {};

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly repos: RepoManager,
    private readonly providers: TabProviders,
    private readonly runner: Runner,
    private readonly log: CommandLog,
  ) {
    const lists: Record<ListId, BaseProvider<Node>> = {
      files: providers.files,
      local: providers.branches,
      remotes: providers.branches,
      tags: providers.branches,
      worktrees: providers.branches,
      commits: providers.commits,
      reflog: providers.reflog,
      stash: providers.stashes,
    };
    for (const [list, provider] of Object.entries(lists) as [ListId, BaseProvider<Node>][]) {
      this.views.set(list, this.listView(list));
      this.disposables.push(provider.onDidChangeTreeData(() => this.renderListsOf(provider)));
    }
    const logListener = log.onDidChange(() => this.post({ type: 'log', entries: [...log.entries] }));
    this.disposables.push(
      this.visibility,
      { dispose: logListener },
      registerWebviewNodes(SlothGitTab.viewType, (ctx) => this.nodesFor(typeof ctx.slothgitId === 'string' ? ctx.slothgitId : '')),
      onDidChangeContext(() => this.contextChanged()),
      vscode.workspace.onDidChangeConfiguration((e) => e.affectsConfiguration('slothgit.simpleMode') && this.contextChanged()),
      vscode.window.registerWebviewPanelSerializer(SlothGitTab.viewType, {
        deserializeWebviewPanel: async (panel) => this.adopt(panel),
      }),
    );
  }

  get visible(): boolean {
    return !!this.panel?.visible;
  }

  /** The list's selection and reveal, for commands run without arguments. */
  view(list: ListId): ListView {
    return this.views.get(list)!;
  }

  /** One view over several lists (the Branches panel's): the selection of whichever was selected last. */
  combinedView(lists: ListId[]): ListView {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const tab = this;
    const last = () => tab.selectOrder.filter((l) => lists.includes(l)).pop() ?? lists[0];
    return {
      get selection() {
        return tab.view(last()).selection;
      },
      get visible() {
        return tab.visible;
      },
      reveal: async (node, options) => {
        const list = lists.find((l) => tab.idOf(l, node));
        if (list) await tab.view(list).reveal(node, options);
      },
    };
  }

  /** Opens the tab, or brings it to the front, showing `list` if given. */
  show(list?: ListId): void {
    if (this.panel) this.panel.reveal(undefined, false);
    else this.adopt(vscode.window.createWebviewPanel(SlothGitTab.viewType, 'SlothGit', vscode.ViewColumn.Active, { retainContextWhenHidden: true }));
    if (list) this.post({ type: 'show', list });
  }

  private adopt(panel: vscode.WebviewPanel): void {
    this.panel?.dispose();
    this.panel = panel;
    panel.iconPath = vscode.Uri.joinPath(this.context.extensionUri, 'media', 'icon-mono.svg');
    panel.webview.options = webviewOptions(this.context.extensionUri);
    panel.webview.html = pageHtml(panel.webview, this.context.extensionUri, 'tab', 'SlothGit', body);
    const listeners = [
      panel.webview.onDidReceiveMessage((m: FromTab) => this.onMessage(m)),
      panel.onDidChangeViewState(() => {
        this.visibility.fire(panel.visible);
        // The lists reload themselves if they missed changes while hidden; the Status panel is cheap to reload.
        if (panel.visible) void this.reload();
      }),
      panel.onDidDispose(() => {
        listeners.forEach((l) => l.dispose());
        if (this.panel === panel) {
          this.panel = undefined;
          this.entries.clear();
          this.detail = undefined;
          this.backStack.length = 0;
          this.visibility.fire(false);
        }
      }),
    ];
  }

  private post(message: ToTab): void {
    this.sent.set(message.type === 'list' ? `list:${message.list.id}` : message.type, message);
    void this.panel?.webview.postMessage(message);
  }

  /** The last message of a kind sent to the page, for tests. */
  lastSent<K extends ToTab['type']>(type: K): Extract<ToTab, { type: K }> | undefined {
    return this.sent.get(type) as Extract<ToTab, { type: K }> | undefined;
  }

  /** The rows last sent for a list, for tests. */
  lastList(list: ListId): TabList | undefined {
    return (this.sent.get(`list:${list}`) as Extract<ToTab, { type: 'list' }> | undefined)?.list;
  }

  /** Handles a message as if the page had sent it, for tests. */
  receive(message: FromTab): Promise<void> {
    return this.onMessage(message);
  }

  // --- lists ----------------------------------------------------------------------------------

  private listView(list: ListId): ListView {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const tab = this;
    return {
      get selection() {
        return (tab.selections.get(list) ?? []).flatMap((id) => {
          const entry = tab.entries.get(id);
          return entry ? [entry.node] : [];
        });
      },
      get visible() {
        return tab.visible;
      },
      reveal: async (node, options) => {
        const id = tab.idOf(list, node);
        if (!id || options?.select === false) return;
        tab.selections.set(list, [id]);
        tab.post({ type: 'select', list, ids: [id] });
        if (tab.activeList === list) await tab.showDetail(tab.entries.get(id)!);
      },
    };
  }

  /** The row id a node has in `list`: the same node, or one showing the same thing after a reload. */
  private idOf(list: ListId, node: Node): string | undefined {
    const ids = this.rowIds.get(list) ?? [];
    const key = detailKey(node);
    return ids.find((id) => this.entries.get(id)?.node === node) ?? ids.find((id) => detailKey(this.entries.get(id)!.node) === key);
  }

  private renderListsOf(provider: BaseProvider<Node>): void {
    const { branches, commits, files, reflog, stashes } = this.providers;
    const lists: ListId[] =
      provider === branches ? ['local', 'remotes', 'tags', 'worktrees'] : provider === commits ? ['commits'] : provider === files ? ['files'] : provider === reflog ? ['reflog'] : provider === stashes ? ['stash'] : [];
    for (const list of lists) this.renderList(list);
    void this.refreshDetail(provider);
  }

  private renderList(list: ListId): void {
    if (!this.panel) return;
    for (const id of this.rowIds.get(list) ?? []) this.entries.delete(id);
    const view = LIST_VIEWS[list];
    const rows: TabRow[] = [];
    const ids: string[] = [];
    const add = (node: Node, provider: BaseProvider<Node>, parent?: string, expandable?: boolean) => {
      const row = this.row({ node, provider, view }, `${list}:`, parent, expandable);
      this.entries.set(row.id, { node, provider, view, list });
      ids.push(row.id);
      rows.push(row);
      return row.id;
    };
    const { branches } = this.providers;
    let provider: BaseProvider<Node>;
    let description: string | undefined;
    switch (list) {
      case 'files':
      case 'commits':
      case 'reflog':
      case 'stash': {
        provider = { files: this.providers.files, commits: this.providers.commits, reflog: this.providers.reflog, stash: this.providers.stashes }[list] as BaseProvider<Node>;
        for (const node of provider.items) add(node, provider);
        description = provider.summary;
        break;
      }
      default: {
        provider = branches as BaseProvider<Node>;
        const section = branches.items.find((s) => s.section === list);
        for (const node of section?.children ?? []) {
          if (node.kind === 'remote') {
            const parent = add(node, provider, undefined, true);
            for (const branch of node.branches) add(branch, provider, parent);
          } else {
            add(node, provider);
          }
        }
      }
    }
    this.rowIds.set(list, ids);
    // Keep only selected rows that are still there.
    const kept = (this.selections.get(list) ?? []).filter((id) => this.entries.has(id));
    this.selections.set(list, kept);
    const message: TabList = { id: list, rows, description, actions: this.headerActions(LIST_ACTIONS[list]), empty: EMPTY[list] };
    this.post({ type: 'list', list: message });
  }

  private renderAll(): void {
    for (const list of Object.keys(LIST_ACTIONS) as ListId[]) this.renderList(list);
  }

  /** A row for `source`, with an id unique on the page. */
  private row(source: Source, prefix: string, parent?: string, expandable?: boolean): TabRow {
    const item = source.provider.getTreeItem(source.node) as vscode.TreeItem;
    const values = this.contextValues({ view: source.view, item: item.contextValue ?? source.node.kind });
    const actions = ROW_ACTIONS.filter((a) => evaluate(a.when, values)).map((a) => a.command);
    const id = `${prefix}${item.id ?? detailKey(source.node)}`;
    return toRow(item, source.node, { id, view: source.view, actions, parent, expandable, remotes: this.providers.branches.remoteNames });
  }

  private contextValues(extra: Record<string, unknown> = {}): Record<string, unknown> {
    const simple = vscode.workspace.getConfiguration('slothgit').get('simpleMode', false);
    return { ...contextValues(), 'config.slothgit.simpleMode': simple, ...extra };
  }

  private headerActions(entries: HeaderEntry[]): TabAction[] {
    const values = this.contextValues();
    return entries.filter((e) => evaluate(e.when, values)).map((e) => ({ command: e.command, primary: !!e.primary }));
  }

  /** Context keys decide which buttons show; redraw once a burst of changes settles. */
  private contextChanged(): void {
    clearTimeout(this.contextTimer);
    this.contextTimer = setTimeout(() => {
      this.renderAll();
      void this.reload();
    }, 30);
  }

  /** The node of a row, and the list's selection when the row is part of it (commands act on all of them). */
  private nodesFor(id: string): unknown[] {
    const entry = this.entries.get(id);
    if (!entry) return [];
    const selection = entry.list ? (this.selections.get(entry.list) ?? []) : [];
    if (selection.length > 1 && selection.includes(id)) {
      return [entry.node, selection.flatMap((s) => (this.entries.has(s) ? [this.entries.get(s)!.node] : []))];
    }
    return [entry.node];
  }

  // --- status ---------------------------------------------------------------------------------

  /** Reloads the Status panel. Called with the providers after every change (refreshAll). */
  async reload(): Promise<void> {
    if (!this.panel) return;
    const version = ++this.statusVersion;
    const status = await this.status();
    if (version === this.statusVersion) this.post({ type: 'status', status });
  }

  /** What the Status panel shows. */
  async status(): Promise<TabStatus> {
    const git = this.repos.git();
    const simple = vscode.workspace.getConfiguration('slothgit').get('simpleMode', false);
    const base: TabStatus = { hasRemotes: false, bisecting: false, actions: this.headerActions(STATUS_ACTIONS), simple };
    if (!git) return base;
    try {
      const [branches, remotes, operation, bisect] = await Promise.all([getLocalBranches(git, 'name'), getRemotes(git), getOperation(git), getBisectState(git)]);
      const branch = branches.find((b) => b.current);
      const todo = operation === 'rebase' ? (await getRebaseTodo(git).catch(() => [])).length : undefined;
      return {
        ...base,
        repo: path.basename(git.cwd),
        branch: branch && { name: branch.name, upstream: branch.upstream?.name, ahead: branch.ahead, behind: branch.behind, gone: branch.gone },
        detached: branch ? undefined : (await headSha(git).catch(() => '')).slice(0, 7) || undefined,
        hasRemotes: remotes.length > 0,
        operation: operation && OPERATION_LABELS[operation],
        todo,
        bisecting: !!bisect,
      };
    } catch {
      return { ...base, repo: path.basename(git.cwd) };
    }
  }

  // --- detail pane ----------------------------------------------------------------------------

  private async showDetail(source: Source, full = false): Promise<void> {
    this.detail = { source, full };
    const version = ++this.detailVersion;
    const git = this.repos.git();
    if (!git || !this.panel) return;
    // Rows of the previous detail are no longer on the page.
    for (const id of [...this.entries.keys()]) if (id.startsWith('detail:')) this.entries.delete(id);
    let detail: TabDetail;
    try {
      detail = await buildDetail(source, {
        git,
        row: (s) => {
          const row = this.row(s, 'detail:');
          this.entries.set(row.id, { ...s });
          return row;
        },
        remotes: this.providers.branches.remoteNames,
        commits: this.providers.commits as BaseProvider<Node>,
      }, full);
    } catch (error) {
      detail = { title: 'Error', key: detailKey(source.node), blocks: [{ type: 'text', text: `Could not load the details: ${error instanceof Error ? error.message : error}` }] };
    }
    if (version !== this.detailVersion) return;
    const back = this.backStack[this.backStack.length - 1];
    if (back) detail.back = back.node.kind === 'ref' || back.node.kind === 'remote' ? `Back to ${back.node.kind === 'ref' ? back.node.name : back.node.remote.name}` : 'Back';
    this.post({ type: 'detail', detail });
  }

  /** After a list reloaded: show the same thing again (with fresh data), if it still exists. */
  private async refreshDetail(provider: BaseProvider<Node>): Promise<void> {
    const current = this.detail;
    if (!current || current.source.provider !== provider || !this.activeList) return;
    // A row of the active list: the reloaded node for it, found by what it shows.
    const id = this.idOf(this.activeList, current.source.node);
    if (id) {
      const entry = this.entries.get(id)!;
      if (this.backStack.length === 0) await this.showDetail(entry, current.full);
    } else if (!this.backStack.length) {
      this.selections.set(this.activeList, []);
      this.detail = undefined;
      this.post({ type: 'detail', detail: { title: '', key: '', blocks: [] } });
    }
  }

  /**
   * After any change to the working tree: an uncommitted file's diff in the detail pane can change without its
   * status changing (another edit to a modified file), which the Files list doesn't report.
   */
  async workingTreeChanged(): Promise<void> {
    const current = this.detail;
    if (current?.source.node.kind === 'change' && !this.backStack.length && this.visible) await this.showDetail(current.source, current.full);
  }

  // --- messages -------------------------------------------------------------------------------

  private async onMessage(message: FromTab): Promise<void> {
    switch (message.type) {
      case 'ready': {
        this.visibility.fire(this.visible);
        this.post({ type: 'init', commands: this.commandInfo() });
        this.renderAll();
        this.post({ type: 'log', entries: [...this.log.entries] });
        // The draft lives in the Source Control view's message box, so it outlives the tab.
        this.post({ type: 'draft', message: this.repos.repository?.inputBox.value ?? '' });
        await this.reload();
        return;
      }
      case 'select': {
        this.selections.set(message.list, message.ids.filter((id) => this.entries.has(id)));
        this.activeList = message.list;
        this.selectOrder = [...this.selectOrder.filter((l) => l !== message.list), message.list];
        const entry = this.entries.get(message.ids[0]);
        this.backStack.length = 0;
        if (entry) await this.showDetail(entry);
        return;
      }
      case 'detailSelect': {
        const entry = this.entries.get(message.id);
        if (!entry) return;
        if (entry.node.kind === 'file') {
          await vscode.commands.executeCommand('slothgit.openFileDiff', entry.node);
          return;
        }
        if (this.detail) this.backStack.push(this.detail.source);
        await this.showDetail(entry);
        return;
      }
      case 'back': {
        const previous = this.backStack.pop();
        if (previous) await this.showDetail(previous);
        return;
      }
      case 'open':
        return this.open(message.id);
      case 'command': {
        if (!BUTTON_COMMANDS.has(message.command)) return;
        const args = message.id ? this.nodesFor(message.id) : [];
        if (message.id && !args.length) return;
        await vscode.commands.executeCommand(message.command, ...args);
        return;
      }
      case 'commit':
        await this.commit(message.message, message.amend);
        return;
      case 'draft': {
        const repo = this.repos.repository;
        if (repo) repo.inputBox.value = message.message;
        return;
      }
      case 'openDiff':
        return this.openDiff(message.path);
      case 'fullDiff':
        if (this.detail) await this.showDetail(this.detail.source, true);
        return;
      case 'pullPush':
        await this.pullPushMenu(message.menu);
        return;
      case 'clearLog':
        this.log.clear();
        return;
      case 'rendered':
        this.renderedLists = message.lists;
        return;
    }
  }

  /** Double click: what a row is mostly there for. */
  private async open(id: string): Promise<void> {
    const entry = this.entries.get(id);
    if (!entry) return;
    const { node } = entry;
    if (node.kind === 'commit') await vscode.commands.executeCommand('slothgit.commit.openChanges', node);
    else if (node.kind === 'file') await vscode.commands.executeCommand('slothgit.openFileDiff', node);
    else if (node.kind === 'change') await vscode.commands.executeCommand('slothgit.file.openChange', node);
    else if (node.kind === 'ref' && !(node.type === 'local' && node.branch.current)) await vscode.commands.executeCommand('slothgit.branch.checkout', node);
    else if (node.kind === 'worktree' && !node.worktree.current) await vscode.commands.executeCommand('slothgit.worktree.open', node);
  }

  /** A file in the detail pane's diff: its row in the file list, or the uncommitted change shown. */
  private async openDiff(file: string): Promise<void> {
    const node = this.detail?.source.node;
    if (node?.kind === 'change') {
      await vscode.commands.executeCommand('slothgit.file.openChange', node);
      return;
    }
    for (const [id, entry] of this.entries) {
      if (id.startsWith('detail:') && entry.node.kind === 'file' && entry.node.pair.change.path === file) {
        await vscode.commands.executeCommand('slothgit.openFileDiff', entry.node);
        return;
      }
    }
  }

  private commandInfo(): TabCommands {
    const contributed: { command: string; title: string; icon?: string }[] = this.context.extension.packageJSON.contributes.commands;
    const wanted = new Set([...BUTTON_COMMANDS, ...Object.values(SYNC_MENUS).flat().map((i) => i.command)]);
    const info: TabCommands = {};
    for (const c of contributed) {
      if (wanted.has(c.command)) info[c.command] = { title: c.title, icon: /^\$\(([\w-]+)\)$/.exec(c.icon ?? '')?.[1] };
    }
    return info;
  }

  /** The ⌄ next to Pull or Push: the same entries as the right-click menu, as a quick pick. */
  async pullPushMenu(menu: 'pull' | 'push'): Promise<void> {
    const simple = vscode.workspace.getConfiguration('slothgit').get('simpleMode', false);
    const info = this.commandInfo();
    const items = SYNC_MENUS[menu]
      .filter((item) => !(simple && item.advanced))
      .map((item) => ({ label: info[item.command]?.title ?? item.command, detail: item.detail, command: item.command }));
    const picked = await vscode.window.showQuickPick(items, { title: menu === 'pull' ? 'Pull' : 'Push' });
    if (picked) await vscode.commands.executeCommand(picked.command);
  }

  // --- committing -----------------------------------------------------------------------------

  /** The Files panel's commit box. Commits through the Git extension, so hooks, signing and its settings apply. */
  async commit(message: string, amend: boolean): Promise<boolean> {
    const git = this.repos.git();
    if (!git) return false;
    let text = message.trim();
    if (!text && !amend) {
      void vscode.window.showInformationMessage('Write a commit message first.');
      return false;
    }
    const staged = this.providers.files.entries.some(isStaged);
    if (!staged && !amend) {
      const files = this.providers.files.entries;
      if (!files.length) {
        void vscode.window.showInformationMessage('There are no changes to commit.');
        return false;
      }
      const stageAll = await this.runner.confirm('There are no staged changes. Stage all your changes and commit them?', 'Stage All and Commit');
      if (!stageAll) return false;
      if (!(await this.runner.run('Staging…', (ops) => ops.stageAll()))) return false;
    }
    // An amend without a new message keeps the old one.
    if (amend && !text) text = await getCommitMessage(git, 'HEAD');
    const ok = await this.runner.run(amend ? 'Amending…' : 'Committing…', async (_ops, repo) => {
      await repo.commit(text, { amend });
      repo.inputBox.value = '';
    });
    if (ok) this.post({ type: 'committed' });
    return ok;
  }

  dispose(): void {
    clearTimeout(this.contextTimer);
    this.panel?.dispose();
    this.disposables.forEach((d) => d.dispose());
  }
}
