import * as fs from 'node:fs';
import * as vscode from 'vscode';
import { setContext } from './contextKeys';
import { fromWebview, type ListView } from './views/webviewNodes';
import { openMultiDiff } from './commands';
import { getCommitFiles, getCommitMessage, getOperation, rebaseTodoPath } from './git/queries';
import { transformTodo, type TodoAction } from './git/todo';
import type { UndoAction } from './git/undo';
import type { RepoManager } from './repoManager';
import type { Runner } from './runner';
import type { CommitNode, FileNode, Node, RefNode, TodoNode } from './views/nodes';
import type { CommitsProvider } from './views/providers';

export interface HistoryCommandDeps {
  repos: RepoManager;
  commits: CommitsProvider;
  commitsView: ListView;
  runner: Runner;
  refreshAll(force: boolean): Promise<void>;
}

/** Something that can be compared: a commit or a branch/tag. */
interface Revision {
  rev: string;
  label: string;
}

function revisionOf(node: Node | undefined): Revision | undefined {
  if (node?.kind === 'commit') return { rev: node.sha, label: node.shortSha };
  if (node?.kind === 'ref') return { rev: (node as RefNode).rev, label: (node as RefNode).name };
  return undefined;
}

export function registerHistoryCommands(deps: HistoryCommandDeps): vscode.Disposable[] {
  const { repos, commits, commitsView, runner } = deps;
  const { run, confirm } = runner;
  let compareBase: Revision | undefined;

  // --- undo / redo -----------------------------------------------------------------

  function describe(action: UndoAction, direction: 'undo' | 'redo'): string {
    const target = direction === 'undo' ? action.from : action.to;
    const short = (rev: string) => (/^[0-9a-f]{40}$/.test(rev) ? rev.slice(0, 7) : rev);
    const branch = repos.repository?.state.HEAD?.name ?? 'HEAD';
    const verb = direction === 'undo' ? 'Undo' : 'Redo';
    if (action.kind === 'checkout') return `${verb} "${action.description}"? This checks out ${short(target)}.`;
    if (action.kind === 'commit') {
      return direction === 'undo'
        ? `Undo "${action.description}"? This moves ${branch} back to ${short(target)} and leaves the commit's changes staged.`
        : `Redo "${action.description}"? This moves ${branch} to ${short(target)} again. Your files are not touched.`;
    }
    return `${verb} "${action.description}"? This resets ${branch} to ${short(target)}. Uncommitted changes are kept.`;
  }

  async function undoRedo(direction: 'undo' | 'redo'): Promise<void> {
    const ops = repos.ops();
    if (!ops) return;
    if (await getOperation(ops.git)) {
      void vscode.window.showWarningMessage('Finish or abort the operation in progress before undoing.');
      return;
    }
    if (commits.bisect) {
      void vscode.window.showWarningMessage('Reset the bisect before undoing.');
      return;
    }
    const plan = await ops.planUndo();
    const action = direction === 'undo' ? plan.undo : plan.redo;
    if (!action) {
      void vscode.window.showInformationMessage(direction === 'undo' ? 'Nothing to undo.' : 'Nothing to redo.');
      return;
    }
    if (!(await confirm(describe(action, direction), direction === 'undo' ? 'Undo' : 'Redo'))) return;
    await run(direction === 'undo' ? 'Undoing…' : 'Redoing…', (o) => o.applyUndo(action, direction));
  }

  // --- bisect ------------------------------------------------------------------------

  async function bisect(mark: 'bad' | 'good' | 'skip', arg?: Node): Promise<void> {
    const node = arg?.kind === 'commit' ? arg : (commitsView.selection.find((n) => n.kind === 'commit') as CommitNode | undefined);
    if (!node) {
      void vscode.window.showInformationMessage('Select a commit first.');
      return;
    }
    let summary = '';
    const ok = await run('Bisecting…', async (ops) => {
      summary = await ops.bisectMark(node.sha, mark);
    });
    if (!ok) return;
    if (/is the first/.test(summary)) {
      const choice = await vscode.window.showInformationMessage(summary, 'Stop Bisecting');
      if (choice) await run('Resetting bisect…', (ops) => ops.bisectReset());
    } else if (summary) {
      void vscode.window.showInformationMessage(`${summary} Test the checked-out commit, then mark it.`);
    }
  }

  /** Changes what the Commits view lists. Only that view depends on it, and nothing reloads when nothing changed. */
  async function setView(change: Partial<Pick<CommitsProvider, 'filterPath' | 'search' | 'allBranches'>>): Promise<void> {
    const keys = Object.keys(change) as (keyof typeof change)[];
    if (keys.every((key) => commits[key] === change[key])) return;
    Object.assign(commits, change);
    await commits.reload(true);
  }

  // --- rebase todo -------------------------------------------------------------------

  async function editTodo(arg: Node | undefined, change: (node: TodoNode) => Parameters<typeof transformTodo>[1] | undefined) {
    const node = arg?.kind === 'todo' ? arg : (commitsView.selection.find((n) => n.kind === 'todo') as TodoNode | undefined);
    const plan = node && change(node);
    if (plan) await writeTodo(plan);
  }

  /** The rebase steps a command applies to: the selection when the clicked step is part of it. */
  function todoTargets(arg?: Node, all?: Node[]): TodoNode[] {
    const selected = (all?.length ? all : commitsView.selection).filter((n): n is TodoNode => n.kind === 'todo');
    const nodes = arg?.kind === 'todo' && !selected.includes(arg) ? [arg] : selected;
    return nodes.filter((n) => n.action !== 'merge' && n.action !== 'update-ref');
  }

  /** lazygit's p / s / f / d / e on rebase steps, for one or several selected steps. */
  async function setTodoAction(action: TodoAction, arg?: Node, all?: Node[]): Promise<void> {
    const nodes = todoTargets(arg, all);
    if (!nodes.length) return;
    if (action === 'reword') {
      if (nodes.length > 1) void vscode.window.showInformationMessage('Reword one rebase step at a time.');
      else await rewordTodo(nodes[0]);
      return;
    }
    await writeTodo({ actions: Object.fromEntries(nodes.map((n) => [n.sha, action])) });
  }

  async function writeTodo(plan: Parameters<typeof transformTodo>[1]): Promise<void> {
    const git = repos.git();
    if (!git) return;
    try {
      const file = await rebaseTodoPath(git);
      fs.writeFileSync(file, transformTodo(fs.readFileSync(file, 'utf8'), plan));
    } catch (error) {
      void vscode.window.showWarningMessage(`Could not update the rebase todo list: ${error instanceof Error ? error.message : error}`);
    }
    await deps.refreshAll(true);
  }

  /** Asks for the step's new subject (the body is kept), like the Commits view's quick Reword. */
  async function rewordTodo(node: TodoNode): Promise<void> {
    const git = repos.git();
    if (!git) return;
    const [subject, ...body] = (await getCommitMessage(git, node.sha)).split('\n');
    const newSubject = await vscode.window.showInputBox({
      title: `Reword ${node.sha} when the rebase gets to it`,
      value: subject,
      prompt: body.length ? 'The commit body is kept as is.' : undefined,
      validateInput: (v) => (v.trim() ? undefined : 'The message cannot be empty.'),
    });
    if (newSubject === undefined) return;
    await run('Updating rebase step…', (ops) => ops.rewordTodo(node.sha, [newSubject, ...body].join('\n')));
  }

  // --- commands ------------------------------------------------------------------------

  const commands: Record<string, (...args: any[]) => unknown> = {
    'slothgit.undo': () => undoRedo('undo'),
    'slothgit.redo': () => undoRedo('redo'),

    'slothgit.bisect.markBad': (arg?: Node) => bisect('bad', arg),
    'slothgit.bisect.markGood': (arg?: Node) => bisect('good', arg),
    'slothgit.bisect.skip': (arg?: Node) => bisect('skip', arg),
    'slothgit.bisect.reset': () => run('Resetting bisect…', (ops) => ops.bisectReset()),

    'slothgit.compare.select': (arg?: Node) => {
      compareBase = revisionOf(arg ?? commitsView.selection[0]);
      setContext('slothgit.hasCompareBase', !!compareBase);
      if (compareBase) {
        void vscode.window.showInformationMessage(
          `Selected ${compareBase.label} for comparison. Pick another commit or branch and choose "Compare with Selected".`,
        );
      }
    },
    'slothgit.compare.withSelected': async (arg?: Node) => {
      const other = revisionOf(arg ?? commitsView.selection[0]);
      const git = repos.git();
      if (!compareBase || !other || !git) return;
      const pairs = await getCommitFiles(git, other.rev, compareBase.rev);
      const title = `${compareBase.label} ↔ ${other.label}`;
      const files: FileNode[] = pairs.map((pair) => ({ kind: 'file', pair, label: title }));
      await openMultiDiff(repos, title, files);
    },
    'slothgit.compare.clear': () => {
      compareBase = undefined;
      setContext('slothgit.hasCompareBase', false);
    },

    'slothgit.commits.filterByFile': async () => {
      const root = repos.repository?.rootUri;
      if (!root) return;
      const active = vscode.window.activeTextEditor?.document.uri;
      const items: (vscode.QuickPickItem & { uri?: vscode.Uri })[] = [];
      if (active?.scheme === 'file') {
        items.push({ label: '$(file) Current File', description: vscode.workspace.asRelativePath(active, false), uri: active });
      }
      items.push({ label: '$(folder-opened) Choose File or Folder…' });
      const picked = items.length === 1 ? items[0] : await vscode.window.showQuickPick(items, { title: 'Filter commits by path' });
      if (!picked) return;
      let uri = picked.uri;
      if (!uri) {
        const chosen = await vscode.window.showOpenDialog({ defaultUri: root, canSelectFolders: true, canSelectFiles: true });
        uri = chosen?.[0];
      }
      if (uri) await vscode.commands.executeCommand('slothgit.commits.showFileHistory', uri);
    },
    'slothgit.commits.showFileHistory': async (arg?: vscode.Uri | Node) => {
      // A file from the Explorer or an editor, or a row of the Files panel.
      const current = repos.repository?.rootUri;
      const uri = arg instanceof vscode.Uri ? arg : arg?.kind === 'change' && current ? vscode.Uri.joinPath(current, arg.entry.path) : undefined;
      const target = uri ?? vscode.window.activeTextEditor?.document.uri;
      const repo = target && repos.api.getRepository(target);
      const root = repo?.rootUri;
      if (!target || !root) {
        void vscode.window.showInformationMessage('That file is not in a Git repository.');
        return;
      }
      const relative = vscode.workspace.asRelativePath(target, false);
      const rel = target.fsPath.startsWith(root.fsPath)
        ? target.fsPath.slice(root.fsPath.length + 1).replace(/\\/g, '/')
        : relative;
      await setView({ filterPath: rel || '.' });
      await vscode.commands.executeCommand('slothgit.open', 'commits');
    },
    'slothgit.commits.clearFilter': () => setView({ filterPath: undefined }),

    'slothgit.commits.search': async () => {
      const query = await vscode.window.showInputBox({
        title: 'Search commits',
        value: commits.search,
        placeHolder: 'Text in the message or author name, or a SHA',
        prompt: 'Case-insensitive. Leave empty to show all commits.',
      });
      if (query === undefined) return;
      await setView({ search: query.trim() || undefined });
      await vscode.commands.executeCommand('slothgit.open', 'commits');
    },
    'slothgit.commits.clearSearch': () => setView({ search: undefined }),
    'slothgit.commits.showAllBranches': () => setView({ allBranches: true }),
    'slothgit.commits.showCurrentBranch': () => setView({ allBranches: false }),

    'slothgit.todo.moveUp': (arg?: Node) => editTodo(arg, (node) => ({ move: { sha: node.sha, direction: 'up' } })),
    'slothgit.todo.moveDown': (arg?: Node) => editTodo(arg, (node) => ({ move: { sha: node.sha, direction: 'down' } })),
    'slothgit.todo.pick': (arg?: Node, all?: Node[]) => setTodoAction('pick', arg, all),
    'slothgit.todo.reword': (arg?: Node, all?: Node[]) => setTodoAction('reword', arg, all),
    'slothgit.todo.squash': (arg?: Node, all?: Node[]) => setTodoAction('squash', arg, all),
    'slothgit.todo.fixup': (arg?: Node, all?: Node[]) => setTodoAction('fixup', arg, all),
    'slothgit.todo.edit': (arg?: Node, all?: Node[]) => setTodoAction('edit', arg, all),
    'slothgit.todo.drop': (arg?: Node, all?: Node[]) => setTodoAction('drop', arg, all),
    'slothgit.todo.setAction': async (arg?: Node) => {
      const node = arg?.kind === 'todo' ? arg : (commitsView.selection.find((n) => n.kind === 'todo') as TodoNode | undefined);
      if (!node) return;
      const actions: { label: TodoAction; description: string }[] = [
        { label: 'pick', description: 'Keep the commit' },
        { label: 'reword', description: 'Keep the commit, with a new message' },
        { label: 'squash', description: 'Meld into the step below, combining both messages' },
        { label: 'fixup', description: "Meld into the step below, keeping that step's message" },
        { label: 'edit', description: 'Stop after applying it, to change its files' },
        { label: 'drop', description: 'Leave the commit out' },
      ];
      const picked = await vscode.window.showQuickPick(
        actions.filter((a) => a.label !== node.action),
        { title: `Rebase step for ${node.sha}: ${node.subject}` },
      );
      if (picked?.label === 'reword') await rewordTodo(node);
      else if (picked) await editTodo(node, (n) => ({ actions: { [n.sha]: picked.label } }));
    },
  };

  // fromWebview: the tab's and the graph's context menus pass a webview context instead of a node.
  return Object.entries(commands).map(([id, handler]) => vscode.commands.registerCommand(id, (...args: unknown[]) => handler(...fromWebview(args))));
}
