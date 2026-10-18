// Test harness for the end-to-end run inside VS Code: scripted answers for prompts, repository snapshots,
// and lookups for SlothGit's tree nodes.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import type { SlothGitTestApi } from '../../src/extension';
import type { CommitNode, FileNode, Node, RefNode, RemoteNode, SectionNode, StashNode, TodoNode, WorktreeNode } from '../../src/views/nodes';

type ProviderName = keyof SlothGitTestApi['providers'];

// --- scripted prompts ---------------------------------------------------------------------

/** One answer to the next prompt SlothGit shows. `expect` is matched against the prompt's title or message. */
export type Answer =
  | { pick: string | string[]; expect?: RegExp }
  | { input: string; expect?: RegExp }
  | { button: string | undefined; expect?: RegExp };

export const pick = (label: string | string[], expect?: RegExp): Answer => ({ pick: label, expect });
export const input = (value: string, expect?: RegExp): Answer => ({ input: value, expect });
/** Click a button on a message or confirmation dialog. */
export const button = (label: string, expect?: RegExp): Answer => ({ button: label, expect });
/** Dismiss a message that offers buttons. */
export const dismiss = (expect?: RegExp): Answer => ({ button: undefined, expect });

interface Message {
  level: 'info' | 'warning' | 'error';
  text: string;
}

class Prompts {
  queue: Answer[] = [];
  messages: Message[] = [];
  problems: string[] = [];
  opened: string[] = [];
  expectedErrors: RegExp[] = [];

  reset(): void {
    this.queue = [];
    this.messages = [];
    this.problems = [];
    this.opened = [];
    this.expectedErrors = [];
  }

  next<K extends 'pick' | 'input' | 'button'>(kind: K, prompt: string): Extract<Answer, Record<K, unknown>> | undefined {
    const head = this.queue[0];
    if (!head || !(kind in head)) {
      this.problems.push(`unexpected ${kind === 'button' ? 'dialog' : kind === 'pick' ? 'quick pick' : 'input box'}: "${prompt}"`);
      return undefined;
    }
    this.queue.shift();
    if (head.expect && !head.expect.test(prompt)) this.problems.push(`prompt "${prompt}" does not match ${head.expect}`);
    return head as Extract<Answer, Record<K, unknown>>;
  }
}

export const prompts = new Prompts();

const plain = (label: string) => label.replace(/^\$\([^)]*\)\s*/, '');

function install(): void {
  const w = vscode.window as any;

  w.showQuickPick = async (itemsOrPromise: any, options?: vscode.QuickPickOptions & { canPickMany?: boolean }) => {
    const items: (string | vscode.QuickPickItem)[] = await itemsOrPromise;
    const labelOf = (i: string | vscode.QuickPickItem) => plain(typeof i === 'string' ? i : i.label);
    const title = options?.title ?? options?.placeHolder ?? '';
    const answer = prompts.next('pick', `${title} [${items.map(labelOf).join(' | ')}]`);
    if (!answer) return undefined;
    const wanted = Array.isArray(answer.pick) ? answer.pick : [answer.pick];
    const found = wanted.map((label) => items.find((i) => labelOf(i) === label));
    if (found.some((f) => !f)) {
      prompts.problems.push(`quick pick "${title}" has no item ${JSON.stringify(wanted)}; items: ${items.map(labelOf).join(', ')}`);
      return undefined;
    }
    return options?.canPickMany ? found : found[0];
  };

  w.showInputBox = async (options?: vscode.InputBoxOptions) => {
    const answer = prompts.next('input', options?.title ?? options?.prompt ?? '');
    if (!answer) return undefined;
    const invalid = await options?.validateInput?.(answer.input);
    if (invalid) {
      prompts.problems.push(`input "${answer.input}" rejected: ${typeof invalid === 'string' ? invalid : invalid.message}`);
      return undefined;
    }
    return answer.input;
  };

  const message = (level: Message['level']) => async (text: string, ...rest: any[]) => {
    prompts.messages.push({ level, text });
    const items = rest.filter((r) => typeof r === 'string' || (r && typeof r.title === 'string' && !('modal' in r)));
    if (level === 'error' && !prompts.expectedErrors.some((re) => re.test(text))) prompts.problems.push(`error shown: ${text}`);
    if (!items.length || level === 'error') return undefined;
    const answer = prompts.next('button', text);
    if (!answer?.button) return undefined;
    const item = items.find((i) => (typeof i === 'string' ? i : i.title) === answer.button);
    if (!item) prompts.problems.push(`dialog "${text}" has no button "${answer.button}"`);
    return item;
  };
  w.showInformationMessage = message('info');
  w.showWarningMessage = message('warning');
  w.showErrorMessage = message('error');
  w.showOpenDialog = async () => undefined;

  // Opening folders or URLs would leave the test window; record them instead.
  (vscode.env as any).openExternal = async (uri: vscode.Uri) => {
    prompts.opened.push(uri.toString(true));
    return true;
  };
  const execute = vscode.commands.executeCommand;
  (vscode.commands as any).executeCommand = (id: string, ...args: any[]) => {
    if (id === 'vscode.openFolder') {
      prompts.opened.push(`folder:${(args[0] as vscode.Uri).fsPath}`);
      return Promise.resolve();
    }
    return execute(id, ...args);
  };
}

// --- repository snapshots -----------------------------------------------------------------

const isGitDir = (p: string) => path.basename(p) === '.git' || p.endsWith('.git');
const skipObjects = (p: string) => !(path.basename(p) === 'objects' && isGitDir(path.dirname(p)));

/** Copies `src` into `dst`, merging into existing folders. (fs.cpSync fails on the hidden .git folder on Windows.) */
function copyInto(src: string, dst: string, filter: (p: string) => boolean = () => true): void {
  fs.mkdirSync(dst, { recursive: true });
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    const from = path.join(src, entry.name);
    const to = path.join(dst, entry.name);
    if (!filter(from)) continue;
    if (entry.isDirectory()) copyInto(from, to, filter);
    else fs.copyFileSync(from, to);
  }
}

/**
 * Makes `dst` match `src`: removes what `src` doesn't have and rewrites files that differ. Folders that exist
 * in both are never removed, so the Git extension never sees the repository disappear. Object databases are
 * left alone; they only ever grow.
 */
function sync(src: string, dst: string): void {
  const wanted = new Map(fs.readdirSync(src, { withFileTypes: true }).map((e) => [e.name, e]));
  for (const entry of fs.readdirSync(dst, { withFileTypes: true })) {
    const p = path.join(dst, entry.name);
    if (!skipObjects(p)) continue;
    const other = wanted.get(entry.name);
    if (!other || other.isDirectory() !== entry.isDirectory()) fs.rmSync(p, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
  for (const entry of wanted.values()) {
    const from = path.join(src, entry.name);
    const to = path.join(dst, entry.name);
    if (entry.isDirectory()) {
      fs.mkdirSync(to, { recursive: true });
      sync(from, to);
    } else {
      const content = fs.readFileSync(from);
      let current: Buffer | undefined;
      try {
        current = fs.readFileSync(to);
      } catch {
        // missing
      }
      if (!current || !current.equals(content)) fs.writeFileSync(to, content);
    }
  }
}

/**
 * Closes every editor except the SlothGit tab, and makes sure that one is open and in front: its lists only load
 * while it is visible.
 */
export async function closeEditorsButTab(): Promise<void> {
  const isTab = (tab: vscode.Tab) => tab.input instanceof vscode.TabInputWebview && tab.input.viewType.endsWith('slothgit.tab');
  const others = vscode.window.tabGroups.all.flatMap((g) => g.tabs).filter((tab) => !isTab(tab));
  if (others.length) await vscode.window.tabGroups.close(others);
  await vscode.commands.executeCommand('slothgit.open');
}

// --- the test context -----------------------------------------------------------------------

export class Context {
  readonly repo: string;
  readonly origin: string;
  readonly upstream: string;
  private readonly snapshot: string;
  baselineStatus = '';

  constructor(
    readonly api: SlothGitTestApi,
    readonly playground: string,
    private readonly gitRepo: { status(): Promise<void> },
  ) {
    this.repo = path.join(playground, 'repo');
    this.origin = path.join(playground, 'origin.git');
    this.upstream = path.join(playground, 'upstream.git');
    this.snapshot = `${playground}-snapshot`;
    install();
  }

  takeSnapshot(): void {
    copyInto(this.playground, this.snapshot, skipObjects);
    this.baselineStatus = this.status();
  }

  /** Puts the playground back exactly as the snapshot, including SlothGit's in-memory state. */
  async restore(): Promise<void> {
    await closeEditorsButTab();
    for (const id of [
      'slothgit.commits.clearFilter',
      'slothgit.commits.clearSearch',
      'slothgit.commits.showCurrentBranch',
      'slothgit.commit.clearBase',
      'slothgit.compare.clear',
      'slothgit.commit.clearCherryPick',
      'slothgit.patch.reset',
    ]) {
      await vscode.commands.executeCommand(id);
    }
    const settings = vscode.workspace.getConfiguration('slothgit');
    for (const key of ['branchSort', 'updateStackedBranches', 'simpleMode']) {
      // Only when set: changing settings reloads every view.
      if (settings.inspect(key)?.globalValue !== undefined) await settings.update(key, undefined, vscode.ConfigurationTarget.Global);
    }
    sync(this.snapshot, this.playground);
    await this.refresh();
    prompts.reset();
  }

  // --- git ---

  git(cwd: string, ...args: string[]): string {
    return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trimEnd();
  }
  sh(...args: string[]): string {
    return this.git(this.repo, ...args);
  }
  rev(ref: string): string {
    return this.sh('rev-parse', ref);
  }
  exists(ref: string, cwd = this.repo): boolean {
    try {
      this.git(cwd, 'rev-parse', '--verify', '-q', ref);
      return true;
    } catch {
      return false;
    }
  }
  branch(): string {
    return this.sh('branch', '--show-current');
  }
  subjects(count: number, ref = 'HEAD'): string[] {
    return this.sh('log', `-${count}`, '--format=%s', ref).split('\n');
  }
  status(): string {
    return this.sh('status', '--porcelain');
  }
  read(name: string): string {
    return fs.readFileSync(path.join(this.repo, name), 'utf8');
  }
  operationInProgress(): boolean {
    const gitDir = path.join(this.repo, '.git');
    return ['rebase-merge', 'rebase-apply', 'MERGE_HEAD', 'CHERRY_PICK_HEAD', 'REVERT_HEAD', 'sequencer'].some((f) => fs.existsSync(path.join(gitDir, f)));
  }
  /** Discards the playground's uncommitted changes, for scenarios that switch branches. */
  async cleanTree(): Promise<void> {
    this.discardChanges();
    await this.refresh();
  }
  async checkout(branch: string): Promise<void> {
    this.discardChanges();
    this.sh('switch', '-q', branch);
    await this.refresh();
  }
  private discardChanges(): void {
    this.sh('reset', '-q', '--hard');
    this.sh('clean', '-qfd');
  }

  // --- SlothGit ---

  async refresh(): Promise<void> {
    await this.gitRepo.status().catch(() => undefined);
    await this.api.refreshAll(true);
  }

  /** Runs a command with scripted answers, and checks every answer was used and nothing went wrong. */
  async run(command: string, args: unknown[] = [], answers: Answer[] = []): Promise<void> {
    prompts.queue.push(...answers);
    await vscode.commands.executeCommand(command, ...args);
    this.check(command);
  }

  check(what: string): void {
    const unused = prompts.queue.splice(0);
    const problems = prompts.problems.splice(0);
    if (unused.length) problems.push(`${unused.length} answer(s) not used: ${JSON.stringify(unused)}`);
    assert.deepEqual(problems, [], `${what}:\n  ${problems.join('\n  ')}`);
  }

  saw(level: Message['level'], pattern: RegExp): boolean {
    return prompts.messages.some((m) => m.level === level && pattern.test(m.text));
  }
  expectError(pattern: RegExp): void {
    prompts.expectedErrors.push(pattern);
  }

  // --- nodes ---

  private roots(provider: ProviderName): Promise<Node[]> {
    return Promise.resolve(this.api.providers[provider].getChildren() as Node[]);
  }
  async children(provider: ProviderName, node: Node): Promise<Node[]> {
    return ((await this.api.providers[provider].getChildren(node)) ?? []) as Node[];
  }

  /** Subjects of the commits selected in the Commits panel, once the selection settles (reveal is async). */
  async selectedSubjects(expected: string[]): Promise<string[]> {
    const read = () =>
      this.api.views.commits.selection.filter((n): n is CommitNode => n.kind === 'commit').map((n) => n.subject);
    for (let i = 0; i < 20 && JSON.stringify(read()) !== JSON.stringify(expected); i++) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return read();
  }
  get selection(): readonly Node[] {
    return this.api.views.commits.selection;
  }

  treeItem(provider: ProviderName, node: Node): Thenable<vscode.TreeItem> | vscode.TreeItem {
    return this.api.providers[provider].getTreeItem(node);
  }

  async commits(): Promise<CommitNode[]> {
    return (await this.roots('commits')).filter((n): n is CommitNode => n.kind === 'commit');
  }
  async commit(subject: string): Promise<CommitNode> {
    const found = (await this.commits()).find((c) => c.subject === subject);
    assert.ok(found, `commit "${subject}" in the Commits view`);
    return found;
  }
  async commitAt(sha: string): Promise<CommitNode> {
    const found = (await this.commits()).find((c) => c.sha === sha);
    assert.ok(found, `commit ${sha.slice(0, 7)} in the Commits view`);
    return found;
  }
  async todos(): Promise<TodoNode[]> {
    return (await this.roots('commits')).filter((n): n is TodoNode => n.kind === 'todo');
  }
  async files(commit: Node, provider: ProviderName = 'commits'): Promise<FileNode[]> {
    return (await this.children(provider, commit)) as FileNode[];
  }
  async file(commit: Node, filePath: string, provider: ProviderName = 'commits'): Promise<FileNode> {
    const found = (await this.files(commit, provider)).find((f) => f.pair.change.path === filePath);
    assert.ok(found, `file ${filePath} in the commit`);
    return found;
  }

  /** The Branches view's sections, in order. */
  async sectionNames(): Promise<string[]> {
    return (await this.roots('branches')).flatMap((n) => (n.kind === 'section' ? [n.section] : []));
  }
  private async section(section: SectionNode['section']): Promise<SectionNode> {
    const found = (await this.roots('branches')).find((n): n is SectionNode => n.kind === 'section' && n.section === section);
    assert.ok(found, `${section} section in the Branches view`);
    return found;
  }
  async localBranch(name: string): Promise<RefNode> {
    const found = (await this.section('local')).children.find((n) => n.kind === 'ref' && n.name === name);
    assert.ok(found, `local branch ${name} in the Branches view`);
    return found as RefNode;
  }
  async localBranchNames(): Promise<string[]> {
    return (await this.section('local')).children.map((n) => (n as RefNode).name);
  }
  async remote(name: string): Promise<RemoteNode> {
    const found = (await this.section('remotes')).children.find((n): n is RemoteNode => n.kind === 'remote' && n.remote.name === name);
    assert.ok(found, `remote ${name} in the Branches view`);
    return found;
  }
  async remoteBranch(name: string): Promise<RefNode> {
    const [remote] = name.split('/');
    const found = (await this.remote(remote)).branches.find((b) => b.name === name);
    assert.ok(found, `remote branch ${name} in the Branches view`);
    return found;
  }
  async tag(name: string): Promise<RefNode> {
    const found = (await this.section('tags')).children.find((n) => n.kind === 'ref' && n.name === name);
    assert.ok(found, `tag ${name} in the Branches view`);
    return found as RefNode;
  }
  async worktree(folder: string): Promise<WorktreeNode> {
    const found = (await this.section('worktrees')).children.find(
      (n): n is WorktreeNode => n.kind === 'worktree' && path.basename(n.worktree.path) === folder,
    );
    assert.ok(found, `worktree ${folder} in the Branches view`);
    return found;
  }
  async stash(message: string): Promise<StashNode> {
    const found = (await this.roots('stashes')).find((n): n is StashNode => n.kind === 'stash' && n.stash.message.includes(message));
    assert.ok(found, `stash "${message}" in the Stashes view`);
    return found;
  }
  async reflog(): Promise<CommitNode[]> {
    return (await this.roots('reflog')) as CommitNode[];
  }
}
