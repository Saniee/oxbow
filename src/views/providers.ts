import * as vscode from 'vscode';
import { setContext } from '../contextKeys';
import {
  getBisectState,
  getCommitFiles,
  getLog,
  getOperation,
  getRebaseApplied,
  getRebaseTodo,
  getReflog,
  getStashes,
  getStashFiles,
  getMainRefs,
  getUnpushed,
  notOnHead,
  reachableFrom,
  type BisectState,
  type Operation,
} from '../git/queries';
import type { TodoEntry } from '../git/todo';
import type { Git } from '../git/runner';
import type { RepoManager } from '../repoManager';
import { BaseProvider } from './baseProvider';
import type { CherryPickClipboard } from './cherryPickClipboard';
import type { CommitNode, FileNode, Node, StashNode, TodoNode } from './nodes';

export function config<T>(key: string, fallback: T): T {
  return vscode.workspace.getConfiguration('slothgit').get<T>(key, fallback);
}

/** "Ada Lovelace" -> "AL", as lazygit shows authors. */
export function initials(name: string | undefined): string {
  const parts = (name ?? '').split(/\s+/).filter(Boolean);
  if (!parts.length) return '?';
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : '')).toUpperCase();
}

/**
 * lazygit-style label: SHA and author first, then the subject. Tree items can't wrap and VS Code cuts off the end
 * of a row, so only the subject gets shortened in a narrow view; the tooltip has everything.
 */
export function commitLabel(node: CommitNode, withAuthor = true): string {
  if (!withAuthor) return `${node.shortSha}  ${node.subject}`;
  const people = [node.author, ...(node.coAuthors ?? []).map((p) => p.name)].map(initials).join('+');
  return `${node.shortSha} ${people}  ${node.subject}`;
}

/** Tooltip lines for a commit's co-authors. */
export function coAuthorLines(node: CommitNode): string[] {
  return (node.coAuthors ?? []).map((p) => `Co-authored by ${p.name}${p.email ? ` <${p.email}>` : ''}`);
}

export async function commitFiles(git: Git, node: CommitNode): Promise<FileNode[]> {
  const pairs = await getCommitFiles(git, node.sha, node.parents[0]);
  return pairs.map((pair) => ({ kind: 'file', pair, label: node.shortSha, commit: node }));
}

const TODO_ICONS: Record<TodoEntry['action'], string> = {
  pick: 'circle-large-outline',
  reword: 'edit',
  edit: 'debug-pause',
  squash: 'fold-up',
  fixup: 'fold-up',
  drop: 'trash',
  merge: 'git-merge',
  'update-ref': 'git-branch',
};

const TODO_MEANINGS: Record<TodoEntry['action'], string> = {
  pick: 'Keep the commit as it is.',
  reword: 'Keep the commit with a new message.',
  edit: 'Stop after applying it, so you can change its files.',
  squash: 'Meld it into the step below, combining both messages.',
  fixup: "Meld it into the step below, keeping that step's message.",
  drop: 'Leave the commit out.',
  merge: 'Recreates a merge commit; it cannot be changed here.',
  'update-ref': 'Moves a stacked branch.',
};

export class CommitsProvider extends BaseProvider<CommitNode | TodoNode> {
  private unpushed: Set<string> | null = null;
  /** Commits already on a main branch (slothgit.mainBranches), and which one. */
  private merged = new Set<string>();
  private mainRef: string | undefined;
  private head: string | undefined;
  /** Commit marked with "Mark as Base Commit"; rebasing onto a branch then only moves the commits after it. */
  baseSha: string | undefined;
  /** Repository-relative path the log is filtered to (lazygit's "filter by path"). */
  filterPath: string | undefined;
  /** Text search over messages, authors and SHAs. */
  search: string | undefined;
  /** List every branch, remote branch and tag, not only HEAD. */
  allBranches = false;
  /** Set while `git bisect` is running. */
  bisect: BisectState | undefined;
  private todoCount = 0;
  /** In-progress rebase, merge, cherry-pick or revert. */
  private operation: Operation | undefined;
  /** Commit steps a stopped rebase has applied; squash and fixup need one to meld into. */
  private rebaseApplied = 0;

  constructor(
    repos: RepoManager,
    output: vscode.LogOutputChannel,
    private readonly clipboard: CherryPickClipboard,
  ) {
    super(repos, output);
  }

  /** A rebase, merge, cherry-pick or revert is stopped. */
  get operationInProgress(): boolean {
    return !!this.operation;
  }

  get commits(): readonly CommitNode[] {
    return this.roots.filter((n): n is CommitNode => n.kind === 'commit');
  }

  /** True when the commit is already on the upstream branch, or on a main branch. */
  isPushed(sha: string): boolean {
    return (this.unpushed !== null && !this.unpushed.has(sha)) || this.merged.has(sha);
  }

  protected async load(git: Git): Promise<(CommitNode | TodoNode)[]> {
    const [bisect, operation] = await Promise.all([getBisectState(git), getOperation(git)]);
    this.bisect = bisect;
    setContext('slothgit.bisecting', !!bisect);
    setContext('slothgit.commitsFiltered', !!this.filterPath);
    setContext('slothgit.commitsSearching', !!this.search);
    setContext('slothgit.commitsAllBranches', this.allBranches);

    // While bisecting HEAD jumps around; keep showing the branch the bisect started from.
    const ref = bisect ? bisect.start : 'HEAD';
    const paths = this.filterPath ? [this.filterPath] : [];
    const [log, unpushed, head, todo, mainRefs] = await Promise.all([
      getLog(git, config('commitLimit', 300), ref, paths, { all: this.allBranches, search: this.search }),
      getUnpushed(git),
      git.exec(['rev-parse', '-q', '--verify', 'HEAD'], { allowedExitCodes: [1, 128] }),
      operation === 'rebase' ? getRebaseTodo(git) : Promise.resolve([]),
      getMainRefs(git, config<string[]>('mainBranches', ['main', 'master'])),
    ]);
    this.unpushed = unpushed;
    this.head = head.stdout.trim() || undefined;
    this.mainRef = mainRefs[0]?.replace(/^refs\/(heads|remotes)\//, '');
    this.todoCount = todo.length;
    this.operation = operation;
    this.rebaseApplied = operation === 'rebase' ? await getRebaseApplied(git).catch(() => 0) : 0;

    // The todo list runs oldest first; show it newest first, above the commits.
    const todoNodes = todo.map((t): TodoNode => ({ kind: 'todo', ...t })).reverse();
    const shas = log.map((c) => c.sha);
    const [others, merged] = await Promise.all([
      this.allBranches ? notOnHead(git, shas) : Promise.resolve(new Set<string>()),
      reachableFrom(git, shas, mainRefs),
    ]);
    this.merged = merged;
    const commits = log.map((c, index): CommitNode => ({ kind: 'commit', source: 'log', index, ...c, onHead: !others.has(c.sha) }));
    return [...todoNodes, ...commits];
  }

  protected keyOf(roots: (CommitNode | TodoNode)[]): string {
    const unpushed = `${this.unpushed ? [...this.unpushed].join(',') : 'none'} ${[...this.merged].join(',')}`;
    const state = JSON.stringify([this.head, this.bisect, this.filterPath, this.search, this.allBranches, this.operation, this.rebaseApplied]);
    const items = roots.map((n) => (n.kind === 'todo' ? `${n.action} ${n.sha} ${n.ref ?? ''}` : `${n.sha} ${n.refs?.join(',')}`));
    return `${unpushed}\n${state}\n${items.join('\n')}`;
  }

  protected description(): string | undefined {
    const parts = [
      this.todoCount && `rebasing · ${this.todoCount} left`,
      this.bisect && 'bisecting',
      this.filterPath && `file: ${this.filterPath}`,
      this.search && `search: ${this.search}`,
      this.allBranches && 'all branches',
      this.clipboard.items.length && `${this.clipboard.items.length} copied`,
    ];
    return parts.filter(Boolean).join(' · ') || undefined;
  }

  protected item(n: Node): vscode.TreeItem {
    if (n.kind === 'todo') return this.todoItem(n);
    const node = n as CommitNode;
    const item = new vscode.TreeItem(commitLabel(node), vscode.TreeItemCollapsibleState.Collapsed);
    item.id = `log:${node.sha}`;
    item.description = [node.relDate, ...(node.refs ?? [])].join(' · ');

    const pushed = this.isPushed(node.sha);
    const merged = this.merged.has(node.sha);
    const copied = this.clipboard.has(node.sha);
    const isBase = node.sha === this.baseSha;
    const bisect = this.bisectMark(node.sha);
    const merge = node.parents.length > 1;
    if (bisect) {
      item.iconPath = new vscode.ThemeIcon(bisect.icon, new vscode.ThemeColor(bisect.color));
      item.description = `${bisect.label} · ${item.description}`;
    } else if (isBase) {
      item.iconPath = new vscode.ThemeIcon('debug-breakpoint', new vscode.ThemeColor('slothgit.rebaseBaseIcon'));
      item.description = `base · ${item.description}`;
    } else if (copied) {
      item.iconPath = new vscode.ThemeIcon('clippy', new vscode.ThemeColor('slothgit.cherryPickedIcon'));
    } else if (node.onHead === false) {
      // Another branch's commit (all-branches mode): not part of this branch's push state.
      item.iconPath = new vscode.ThemeIcon(merge ? 'git-merge' : 'git-commit', new vscode.ThemeColor('slothgit.commitOtherBranchIcon'));
    } else {
      // As in lazygit: green when on a main branch, yellow when pushed, red when not pushed yet.
      const color = merged ? 'slothgit.commitMergedIcon' : pushed ? 'slothgit.commitPushedIcon' : this.unpushed !== null ? 'slothgit.commitUnpushedIcon' : undefined;
      item.iconPath = new vscode.ThemeIcon(merge ? 'git-merge' : 'git-commit', color && new vscode.ThemeColor(color));
    }

    const tooltip = new vscode.MarkdownString();
    tooltip.appendMarkdown(`**${escape(node.subject)}**\n\n`);
    const when = node.date ? `${new Date(node.date).toLocaleString()} (${node.relDate})` : node.relDate;
    const email = node.authorEmail ? ` <${node.authorEmail}>` : '';
    tooltip.appendMarkdown(`\`${node.sha}\`\n\n${escape(`${node.author ?? ''}${email}`)}, ${when}\n\n`);
    for (const line of coAuthorLines(node)) tooltip.appendMarkdown(`${escape(line)}\n\n`);
    if (merge) tooltip.appendMarkdown(`$(git-merge) Merge commit (${node.parents.length} parents)\n\n`);
    if (node.onHead === false) tooltip.appendMarkdown('$(git-branch) Not on the current branch\n\n');
    if (node.refs?.length) tooltip.appendMarkdown(`${escape(node.refs.join(', '))}\n\n`);
    if (merged && node.onHead !== false) tooltip.appendMarkdown(`$(git-merge) On ${escape(this.mainRef ?? 'the main branch')}`);
    else if (this.unpushed !== null && node.onHead !== false) tooltip.appendMarkdown(pushed ? '$(cloud) Pushed' : '$(cloud-upload) Not pushed yet');
    if (copied) tooltip.appendMarkdown('\n\n$(clippy) Copied for cherry-pick');
    if (isBase) tooltip.appendMarkdown('\n\n$(debug-breakpoint) Marked as base: rebasing onto a branch moves only the commits above this one');
    if (bisect) tooltip.appendMarkdown(`\n\n$(${bisect.icon}) Bisect: ${bisect.label}`);
    tooltip.supportThemeIcons = true;
    item.tooltip = tooltip;

    const flags = [
      node.sha === this.head && 'head',
      node.parents.length === 0 && 'root',
      merge && 'merge',
      node.onHead === false && 'other',
      copied && 'copied',
    ].filter(Boolean);
    item.contextValue = ['commit', ...flags].join(':');
    return item;
  }

  private bisectMark(sha: string): { label: string; icon: string; color: string } | undefined {
    const b = this.bisect;
    if (!b) return undefined;
    if (sha === b.bad) return { label: b.terms.bad, icon: 'bug', color: 'slothgit.bisectBadIcon' };
    if (b.good.includes(sha)) return { label: b.terms.good, icon: 'pass', color: 'slothgit.bisectGoodIcon' };
    if (b.skipped.includes(sha)) return { label: 'skipped', icon: 'debug-step-over', color: 'slothgit.bisectSkippedIcon' };
    if (sha === this.head) return { label: 'testing', icon: 'arrow-right', color: 'slothgit.bisectTestingIcon' };
    return undefined;
  }

  /** The next commit step to apply. It is shown last among the steps, since the list is newest first. */
  private nextTodo(): TodoNode | undefined {
    const todos = this.roots.filter((n): n is TodoNode => n.kind === 'todo' && n.action !== 'update-ref');
    return todos[todos.length - 1];
  }

  private todoItem(node: TodoNode): vscode.TreeItem {
    const item = new vscode.TreeItem(node.subject, vscode.TreeItemCollapsibleState.None);
    if (node.action === 'update-ref') {
      item.id = `todo:${node.ref}`;
      item.description = 'stacked branch';
      item.tooltip = `Pending rebase step: ${node.subject} to the commit above it`;
      item.iconPath = new vscode.ThemeIcon(TODO_ICONS[node.action], new vscode.ThemeColor('slothgit.rebaseTodoIcon'));
      item.contextValue = 'todo:update-ref';
      return item;
    }
    // A merge step without -C has no SHA; leave its id to VS Code.
    if (node.sha) item.id = `todo:${node.sha}`;
    // As in lazygit, the action comes first, so the plan reads like the todo file.
    item.label = { label: `${node.action}  ${node.subject}`, highlights: [[0, node.action.length]] };
    item.description = node.sha;
    const tooltip = new vscode.MarkdownString();
    tooltip.appendMarkdown(`**${escape(node.subject)}**\n\n\`${node.sha}\`\n\n`);
    tooltip.appendMarkdown(`Pending rebase step: **${node.action}**. ${TODO_MEANINGS[node.action]}`);
    if (node.action !== 'merge') tooltip.appendMarkdown('\n\nRight-click to change it. Steps run from the bottom up.');
    item.tooltip = tooltip;
    const color = node.action === 'drop' ? 'disabledForeground' : 'slothgit.rebaseTodoIcon';
    item.iconPath = new vscode.ThemeIcon(TODO_ICONS[node.action], new vscode.ThemeColor(color));
    // Merge steps recreate a merge commit and can't be changed or moved. The action and position let the
    // menu hide what doesn't apply: the current action, and squash/fixup on the first step when nothing has
    // been applied yet (there is no commit of this branch to meld it into).
    const first = this.operation === 'rebase' && this.rebaseApplied === 0 && node === this.nextTodo();
    if (node.action === 'merge') item.contextValue = 'todo:merge';
    else item.contextValue = ['todo', node.action, first && 'first'].filter(Boolean).join(':');
    return item;
  }

  protected async children(git: Git, n: Node): Promise<Node[]> {
    return n.kind === 'commit' ? commitFiles(git, n) : [];
  }
}

export class ReflogProvider extends BaseProvider<CommitNode> {
  protected async load(git: Git): Promise<CommitNode[]> {
    const entries = await getReflog(git, config('reflogLimit', 200));
    return entries.map((e, index) => ({ kind: 'commit', source: 'reflog', index, ...e }));
  }

  protected keyOf(roots: CommitNode[]): string {
    return roots.map((e) => `${e.sha} ${e.subject}`).join('\n');
  }

  protected item(n: Node): vscode.TreeItem {
    const node = n as CommitNode;
    const item = new vscode.TreeItem(commitLabel(node, false), vscode.TreeItemCollapsibleState.Collapsed);
    item.id = `reflog:${node.index}:${node.sha}`;
    item.description = node.relDate;
    item.tooltip = `${node.selector}  ${node.sha}\n${node.subject}`;
    item.iconPath = new vscode.ThemeIcon('history');
    item.contextValue = 'reflog';
    return item;
  }

  protected children(git: Git, n: Node): Promise<Node[]> {
    const node = n as CommitNode;
    return commitFiles(git, node);
  }
}

export class StashProvider extends BaseProvider<StashNode> {
  protected async load(git: Git): Promise<StashNode[]> {
    return (await getStashes(git)).map((stash) => ({ kind: 'stash', stash }));
  }

  protected keyOf(roots: StashNode[]): string {
    return roots.map((s) => `${s.stash.sha} ${s.stash.message}`).join('\n');
  }

  protected item(n: Node): vscode.TreeItem {
    const node = n as StashNode;
    const { stash } = node;
    const item = new vscode.TreeItem(stash.message, vscode.TreeItemCollapsibleState.Collapsed);
    item.id = `stash:${stash.sha}`;
    item.description = `${stash.ref} · ${stash.relDate}`;
    item.tooltip = `${stash.ref}  ${stash.sha}\n${stash.message}`;
    item.iconPath = new vscode.ThemeIcon('archive');
    item.contextValue = 'stash';
    return item;
  }

  protected async children(git: Git, n: Node): Promise<Node[]> {
    const node = n as StashNode;
    const pairs = await getStashFiles(git, node.stash);
    return pairs.map((pair) => ({ kind: 'file', pair, label: node.stash.ref }));
  }
}

function escape(text: string): string {
  return text.replace(/[\\`*_{}[\]()#+\-.!|<>]/g, '\\$&');
}
