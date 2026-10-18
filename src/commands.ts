import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { setContext } from './contextKeys';
import { editorColumn, leaveTab } from './views/editorPlacement';
import { fromWebview, type ListView } from './views/webviewNodes';
import type { GitOps } from './git/operations';
import { commitUrl, toWebRepo, type WebRepo } from './git/hosting';
import { removalPatch, selectionPatch, type LineRange, type Side } from './git/patch';
import { getCommitMessage, getOperation, getRemotes, type Operation } from './git/queries';
import type { Git } from './git/runner';
import type { RepoManager } from './repoManager';
import type { Runner } from './runner';
import type { CherryPickClipboard } from './views/cherryPickClipboard';
import type { CustomPatch } from './views/customPatch';
import type { CommitNode, FileNode, Node, StashNode } from './views/nodes';
import type { CommitsProvider } from './views/providers';

export interface CommandDeps {
  repos: RepoManager;
  commits: CommitsProvider;
  commitsView: ListView;
  stashView: ListView;
  clipboard: CherryPickClipboard;
  patch: CustomPatch;
  runner: Runner;
  refreshAll(force: boolean): Promise<void>;
}

export function registerCommands(deps: CommandDeps): vscode.Disposable[] {
  const { repos, commits, commitsView, stashView, clipboard, patch, runner } = deps;
  const { run, confirm } = runner;

  const patchContext = vscode.window.onDidChangeActiveTextEditor(updatePatchContext);

  // --- target resolution ----------------------------------------------------

  /** Nodes a command acts on: the multi-selection, the clicked node, or (when invoked without arguments) the view selection. */
  function resolve<T extends Node>(kind: T['kind'], view: ListView, arg?: Node, all?: Node[]): T[] {
    const nodes = all?.length ? all : arg ? [arg] : [...view.selection];
    return nodes.filter((n): n is T => n.kind === kind);
  }

  function commitTargets(arg?: Node, all?: Node[]): CommitNode[] {
    const nodes = resolve<CommitNode>('commit', commitsView, arg, all);
    if (!nodes.length) void vscode.window.showInformationMessage('Select a commit first.');
    return nodes;
  }

  /** Commits from the Commits view only (history rewriting is meaningless for reflog entries). */
  function logTargets(arg?: Node, all?: Node[]): CommitNode[] {
    return commitTargets(arg, all).filter((n) => n.source === 'log');
  }

  function single<T>(nodes: T[]): T | undefined {
    if (nodes.length > 1) {
      void vscode.window.showInformationMessage('This action works on a single commit.');
      return undefined;
    }
    return nodes[0];
  }

  function stashTarget(arg?: Node): StashNode | undefined {
    const node = resolve<StashNode>('stash', stashView, arg)[0];
    if (!node) void vscode.window.showInformationMessage('Select a stash first.');
    return node;
  }

  async function confirmRewrite(nodes: CommitNode[]): Promise<boolean> {
    const enabled = vscode.workspace.getConfiguration('slothgit').get('confirmRewritingPushedCommits', true);
    if (!enabled || !nodes.some((n) => commits.isPushed(n.sha))) return true;
    return confirm(
      'This rewrites commits that are already pushed to the upstream branch. You will need to force push afterwards.',
      'Rewrite History',
    );
  }

  /**
   * Whether `nodes` can be rewritten right now. `fullHistory` marks actions that depend on neighbouring
   * commits (squash into previous, move), which a file filter or search hides.
   */
  async function canRewrite(nodes: CommitNode[], fullHistory = false): Promise<boolean> {
    if (nodes.some((n) => n.onHead === false)) {
      void vscode.window.showWarningMessage('That commit is not on the current branch. Check out its branch to rewrite it.');
      return false;
    }
    if (commits.bisect) {
      void vscode.window.showWarningMessage('Finish or reset the bisect before rewriting history.');
      return false;
    }
    const git = repos.git();
    const operation = git && (await getOperation(git));
    if (operation) {
      void vscode.window.showWarningMessage(`A ${operation} is in progress. Continue or abort it first.`);
      return false;
    }
    if (fullHistory && (commits.filterPath || commits.search)) {
      const what = commits.filterPath ? 'file filter' : 'search';
      void vscode.window.showWarningMessage(`This action needs the full history. Clear the ${what} first.`);
      return false;
    }
    return true;
  }

  /**
   * Where the commit a rewrite produced ends up, relative to the commit it was run on: rewriting changes
   * SHAs, so the view would otherwise lose the selection. lazygit keeps its cursor on the commit the same way.
   */
  type Landing = 'same' | 'up' | 'down' | 'top' | 'none';

  /** Selects the commit at `from`'s position (moved by `landing`) once the view has reloaded. */
  async function keepSelection(from: CommitNode[], landing: Landing): Promise<void> {
    if (landing === 'none' || !commitsView.visible) return;
    const list = commits.commits;
    let index = 0;
    if (landing !== 'top') {
      // The newest of the targets; a squash or drop of several leaves the result where the newest was.
      const positions = from.map((n) => n.index ?? -1).filter((i) => i >= 0);
      if (!positions.length) return;
      index = Math.min(...positions) + (landing === 'up' ? -1 : landing === 'down' ? 1 : 0);
    }
    const node = list[Math.max(0, Math.min(index, list.length - 1))];
    if (node) await commitsView.reveal(node, { select: true, focus: false }).then(undefined, () => undefined);
  }

  /** run() for a commit command, then keeps the cursor on the resulting commit. */
  async function runOn(nodes: CommitNode[], landing: Landing, title: string, fn: (ops: GitOps) => Promise<string | void>) {
    const ok = await run(title, fn);
    // A stopped rebase lists its steps and HEAD isn't the branch tip, so positions don't carry over.
    if (ok && !commits.operationInProgress) await keepSelection(nodes, landing);
    return ok;
  }

  /** Common shape for commands that rewrite history in the Commits view. */
  function rewrite(
    title: string,
    fn: (ops: GitOps, nodes: CommitNode[]) => Promise<string | void>,
    options: {
      single?: boolean;
      fullHistory?: boolean;
      confirm?: (nodes: CommitNode[]) => string | undefined;
      landing?: Landing;
    } = {},
  ) {
    return async (arg?: Node, all?: Node[]) => {
      let nodes = logTargets(arg, all);
      if (options.single) {
        const node = single(nodes);
        nodes = node ? [node] : [];
      }
      if (!nodes.length || !(await canRewrite(nodes, options.fullHistory))) return;
      const question = options.confirm?.(nodes);
      if (question && !(await confirm(question, 'Continue'))) return;
      if (!(await confirmRewrite(nodes))) return;
      await runOn(nodes, options.landing ?? 'same', title, (ops) => fn(ops, nodes));
    };
  }

  const shas = (nodes: CommitNode[]) => nodes.map((n) => n.sha);
  const plural = (nodes: unknown[], word: string) => (nodes.length === 1 ? `1 ${word}` : `${nodes.length} ${word}s`);

  // --- commands ---------------------------------------------------------------

  const commands: Record<string, (...args: any[]) => unknown> = {
    'slothgit.refresh': () => deps.refreshAll(true),
    'slothgit.switchRepository': () => repos.pickRepository(),

    'slothgit.commit.squash': rewrite('Squashing…', (ops, nodes) => ops.squash(shas(nodes), 'squash'), {
      fullHistory: true,
    }),
    'slothgit.commit.fixup': rewrite('Fixing up…', (ops, nodes) => ops.squash(shas(nodes), 'fixup'), {
      fullHistory: true,
    }),
    'slothgit.commit.drop': rewrite('Dropping…', (ops, nodes) => ops.drop(shas(nodes)), {
      confirm: (nodes) => `Drop ${plural(nodes, 'commit')}? You can get it back with Undo or from the Reflog view.`,
    }),
    'slothgit.commit.moveUp': rewrite('Moving commit…', (ops, [node]) => ops.move(node.sha, 'up'), {
      single: true,
      fullHistory: true,
      landing: 'up',
    }),
    'slothgit.commit.moveDown': rewrite('Moving commit…', (ops, [node]) => ops.move(node.sha, 'down'), {
      single: true,
      fullHistory: true,
      landing: 'down',
    }),
    'slothgit.commit.autosquash': rewrite('Squashing fixup commits…', (ops, [node]) => ops.autosquash(node.sha), {
      single: true,
      fullHistory: true,
      landing: 'none',
    }),
    'slothgit.commit.createFixup': async (arg?: Node, all?: Node[]) => {
      const node = single(logTargets(arg, all));
      if (node && (await canRewrite([node]))) {
        await runOn([node], 'top', 'Creating fixup commit…', async (ops) => {
          await ops.createFixupCommit(node.sha);
          return `Created "fixup! ${node.subject}". Use "Squash In Fixup Commits Above This" on the original commit to fold it in.`;
        });
      }
    },
    'slothgit.commit.amendAuthor': async (arg?: Node, all?: Node[]) => {
      const node = single(logTargets(arg, all));
      if (!node || !(await canRewrite([node]))) return;
      const picked = await vscode.window.showQuickPick(
        [
          { label: 'Reset Author', description: 'Use your current user.name and user.email', id: 'reset' },
          { label: 'Set Author…', description: 'Name <email>', id: 'set' },
          { label: 'Add Co-Author…', description: 'Adds a Co-authored-by trailer', id: 'coauthor' },
          ...(node.coAuthors?.length
            ? [{ label: 'Remove Co-Author…', description: node.coAuthors.map((p) => p.name).join(', '), id: 'removeCoauthor' }]
            : []),
        ],
        { title: `Author of ${node.shortSha}` },
      );
      if (!picked) return;
      if (picked.id === 'removeCoauthor') {
        const people = node.coAuthors ?? [];
        const chosen =
          people.length === 1
            ? people
            : (
                await vscode.window.showQuickPick(
                  people.map((p) => ({ label: p.name, description: p.email, person: p })),
                  { title: `Co-authors to remove from ${node.shortSha}`, canPickMany: true },
                )
              )?.map((i) => i.person);
        if (!chosen?.length) return;
        const names = chosen.map((p) => p.name).join(', ');
        if (!(await confirm(`Remove ${names} as co-author of ${node.shortSha}?`, 'Remove'))) return;
        if (!(await confirmRewrite([node]))) return;
        await runOn([node], 'same', 'Removing co-author…', (ops) => ops.removeCoAuthors(node.sha, chosen));
        return;
      }
      let value = '';
      if (picked.id !== 'reset') {
        const input = await vscode.window.showInputBox({
          title: picked.id === 'set' ? `Author for ${node.shortSha}` : `Co-author for ${node.shortSha}`,
          value: picked.id === 'set' ? node.author : undefined,
          placeHolder: 'Name <email@example.com>',
          validateInput: (v) => (/^[^<>]+ <[^<>\s]+>$/.test(v.trim()) ? undefined : 'Use the form: Name <email>'),
        });
        if (!input) return;
        value = input.trim();
      }
      if (!(await confirmRewrite([node]))) return;
      await runOn([node], 'same', 'Amending author…', (ops) =>
        picked.id === 'reset'
          ? ops.resetAuthor(node.sha)
          : picked.id === 'set'
            ? ops.setAuthor(node.sha, value)
            : ops.addCoAuthor(node.sha, value),
      );
    },
    'slothgit.commit.edit': rewrite(
      'Starting rebase…',
      async (ops, [node]) => {
        await ops.edit(node.sha);
        return `Rebase paused at ${node.shortSha}. Make and stage your changes, then run "Continue" to amend the commit.`;
      },
      { single: true },
    ),
    'slothgit.commit.amendStaged': rewrite('Amending commit…', (ops, [node]) => ops.amendStaged(node.sha), { single: true }),

    'slothgit.commit.reword': async (arg?: Node, all?: Node[]) => {
      const node = single(logTargets(arg, all));
      const git = repos.git();
      if (!node || !git || !(await canRewrite([node]))) return;
      const message = await getCommitMessage(git, node.sha);
      const [subject, ...body] = message.split('\n');
      const newSubject = await vscode.window.showInputBox({
        title: `Reword ${node.shortSha}`,
        value: subject,
        prompt: body.length ? 'The commit body is kept as is.' : undefined,
        validateInput: (v) => (v.trim() ? undefined : 'The message cannot be empty.'),
      });
      if (newSubject === undefined || newSubject === subject) return;
      if (!(await confirmRewrite([node]))) return;
      await runOn([node], 'same', 'Rewording…', (ops) => ops.reword(node.sha, [newSubject, ...body].join('\n')));
    },

    'slothgit.commit.rewordInEditor': async (arg?: Node, all?: Node[]) => {
      const node = single(logTargets(arg, all));
      const git = repos.git();
      if (!node || !git || !(await canRewrite([node]))) return;
      const original = await getCommitMessage(git, node.sha);
      const file = await git.gitDirPath('SLOTHGIT_EDITMSG');
      const edited = await editInEditor(file, original, `Reword ${node.shortSha}`);
      if (edited === undefined || edited === original) return;
      if (!(await confirmRewrite([node]))) return;
      await runOn([node], 'same', 'Rewording…', (ops) => ops.reword(node.sha, edited));
    },

    'slothgit.commit.copy': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      const git = repos.git();
      if (!node || !git) return;
      const web = await webRepo(git);
      const items: (vscode.QuickPickItem & { value: () => Promise<string> })[] = [
        { label: 'SHA', description: node.sha, value: async () => node.sha },
        { label: 'Short SHA', description: node.shortSha, value: async () => node.shortSha },
        { label: 'Subject', description: node.subject, value: async () => node.subject },
        { label: 'Message', value: () => getCommitMessage(git, node.sha) },
        { label: 'Author', value: async () => (await git.run(['log', '-1', '--format=%an <%ae>', node.sha])).trim() },
        { label: 'Diff', description: 'As a patch', value: () => git.run(['show', '--format=', '--binary', node.sha]) },
        ...(web ? [{ label: 'URL', description: commitUrl(web, node.sha), value: async () => commitUrl(web, node.sha) }] : []),
      ];
      const picked = await vscode.window.showQuickPick(items, { title: `Copy from ${node.shortSha}` });
      if (picked) await vscode.env.clipboard.writeText(await picked.value());
    },
    'slothgit.commit.openInBrowser': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      const git = repos.git();
      if (!node || !git) return;
      const web = await webRepo(git);
      if (!web) {
        void vscode.window.showWarningMessage('No remote with a recognisable web URL was found.');
        return;
      }
      await vscode.env.openExternal(vscode.Uri.parse(commitUrl(web, node.sha)));
    },

    'slothgit.file.removeFromCommit': (arg?: Node, all?: Node[]) => removeFiles(false, arg, all),
    'slothgit.file.moveToIndex': (arg?: Node, all?: Node[]) => removeFiles(true, arg, all),
    'slothgit.patch.add': async () => {
      const active = activeCommitDiff();
      if (active) await addToPatch(active.diff.file, active.diff.side, active.ranges);
    },
    'slothgit.file.addToPatch': async (arg?: Node, all?: Node[]) => {
      const files = resolve<FileNode>('file', commitsView, arg, all).filter((f) => f.commit?.source === 'log');
      if (files.some((f) => f.commit?.sha !== files[0].commit?.sha)) {
        void vscode.window.showInformationMessage('Select files from a single commit.');
        return;
      }
      for (const file of files) await addToPatch(file);
    },
    'slothgit.patch.menu': () => patchMenu(),
    'slothgit.patch.reset': () => patch.clear(),
    'slothgit.commit.movePatchHere': async (arg?: Node, all?: Node[]) => {
      const target = single(logTargets(arg, all));
      if (target) await movePatch(target);
    },
    'slothgit.patch.remove': () => patchSelection('discard'),
    'slothgit.patch.moveToIndex': () => patchSelection('index'),
    'slothgit.patch.moveToNewCommit': () => patchSelection('newCommit'),
    'slothgit.file.checkout': async (arg?: Node) => {
      const node = resolve<FileNode>('file', commitsView, arg)[0];
      if (!node?.commit) return;
      const { change } = node.pair;
      if (change.status === 'D') {
        void vscode.window.showInformationMessage(`${change.path} was deleted in ${node.label}; there is nothing to check out.`);
        return;
      }
      const question = `Replace ${change.path} in your working tree with its version from ${node.label}? Uncommitted changes to it are lost.`;
      if (await confirm(question, 'Checkout File')) {
        await run('Checking out file…', (ops) => ops.checkoutFileFrom(node.pair.rightRef, change.path));
      }
    },
    'slothgit.file.openAtRevision': async (arg?: Node) => {
      const node = resolve<FileNode>('file', commitsView, arg)[0];
      if (!node) return;
      const { right, left } = diffUris(repos, node);
      const uri = node.pair.change.status === 'D' ? left : right;
      await loadDocuments(`Loading ${path.posix.basename(node.pair.change.path)}…`, [uri]);
      await vscode.window.showTextDocument(uri, { preview: true, viewColumn: editorColumn() });
    },

    'slothgit.commit.copyForCherryPick': (arg?: Node, all?: Node[]) => {
      const nodes = resolve<CommitNode>('commit', commitsView, arg, all);
      if (nodes.length) clipboard.toggle(nodes);
    },
    'slothgit.commit.pasteCherryPick': async () => {
      const items = clipboard.items;
      if (!items.length) {
        void vscode.window.showInformationMessage('No commits copied. Use "Copy for Cherry-Pick" first.');
        return;
      }
      if (!(await confirm(`Cherry-pick ${plural([...items], 'commit')} onto the current HEAD?`, 'Cherry-Pick'))) return;
      await runOn([], 'top', 'Cherry-picking…', async (ops) => {
        await ops.cherryPick(items.map((n) => n.sha));
        clipboard.clear();
      });
    },
    'slothgit.commit.uncopyCherryPick': (arg?: Node, all?: Node[]) => {
      const nodes = resolve<CommitNode>('commit', commitsView, arg, all).filter((n) => clipboard.has(n.sha));
      if (nodes.length) clipboard.toggle(nodes);
    },
    'slothgit.commit.clearCherryPick': () => clipboard.clear(),

    'slothgit.commit.reset': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      if (node) await runner.resetTo(node.sha, node.shortSha);
    },
    'slothgit.commit.checkout': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      if (node) await run('Checking out…', (ops) => ops.checkout(node.sha));
    },
    'slothgit.commit.createBranch': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      if (!node) return;
      const name = await runner.askBranchName(`New branch at ${node.shortSha}`);
      if (name) await run('Creating branch…', (ops) => ops.createBranch(name, node.sha, true));
    },
    'slothgit.commit.createTag': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      if (node) await createTag(runner, node.sha, node.shortSha);
    },
    'slothgit.commit.markBase': (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      if (!node) return;
      setBase(commits.baseSha === node.sha ? undefined : node.sha);
    },
    'slothgit.commit.clearBase': () => setBase(undefined),
    'slothgit.commit.revert': async (arg?: Node, all?: Node[]) => {
      const nodes = logTargets(arg, all);
      if (!nodes.length) return;
      const merge = nodes.find((n) => n.parents.length > 1);
      if (merge) {
        if (nodes.length > 1) {
          void vscode.window.showInformationMessage('Revert a merge commit on its own.');
          return;
        }
        const mainline = await pickMainline(merge);
        if (mainline) await runOn([], 'top', 'Reverting…', (ops) => ops.revert([merge.sha], mainline));
        return;
      }
      if (!(await confirm(`Revert ${plural(nodes, 'commit')}? This creates new commits that undo them.`, 'Revert'))) return;
      await runOn([], 'top', 'Reverting…', (ops) => ops.revert(shas(nodes)));
    },
    'slothgit.commit.copySha': async (arg?: Node, all?: Node[]) => {
      const nodes = commitTargets(arg, all);
      if (nodes.length) await vscode.env.clipboard.writeText(shas(nodes).join('\n'));
    },
    'slothgit.commit.copyMessage': async (arg?: Node, all?: Node[]) => {
      const node = single(commitTargets(arg, all));
      const git = repos.git();
      if (node && git) await vscode.env.clipboard.writeText(await getCommitMessage(git, node.sha));
    },
    'slothgit.commit.openChanges': async (arg?: Node) => {
      const node = single(commitTargets(arg));
      const git = repos.git();
      const root = repos.repository?.rootUri;
      if (!node || !git || !root) return;
      // Any commit node (log, reflog or under a branch) loads its files the same way.
      const files = (await commits.getChildren(node)) as FileNode[];
      await openMultiDiff(repos, `${node.shortSha}: ${node.subject}`, files);
    },
    'slothgit.openFileDiff': async (node: FileNode) => {
      const root = repos.repository?.rootUri;
      if (!root) return;
      const { left, right } = diffUris(repos, node);
      rememberCommitDiff(node, left, right);
      const title = `${path.posix.basename(node.pair.change.path)} (${node.label})`;
      await loadDocuments(`Loading ${path.posix.basename(node.pair.change.path)}…`, existingSides({ left, right }, node));
      await vscode.commands.executeCommand('vscode.diff', left, right, title, { preview: true, viewColumn: editorColumn() });
    },

    'slothgit.operation.continue': () => withOperation('Continuing…', (ops, op) => ops.continueOperation(op)),
    'slothgit.operation.abort': () => withOperation('Aborting…', (ops, op) => ops.abortOperation(op)),
    'slothgit.operation.skip': () => withOperation('Skipping…', (ops, op) => ops.skipOperation(op)),
    'slothgit.operation.menu': async () => {
      const picked = await vscode.window.showQuickPick(
        [
          { label: '$(debug-continue) Continue', command: 'slothgit.operation.continue' },
          { label: '$(debug-step-over) Skip Current Commit', command: 'slothgit.operation.skip' },
          { label: '$(debug-stop) Abort', command: 'slothgit.operation.abort' },
          { label: '$(source-control) Open Source Control', command: 'workbench.view.scm' },
        ],
        { placeHolder: 'Operation in progress' },
      );
      if (picked) await vscode.commands.executeCommand(picked.command);
    },

    'slothgit.stash.push': () => stashPush('all'),
    'slothgit.stash.pushStaged': () => stashPush('staged'),
    'slothgit.stash.pushKeepIndex': () => stashPush('keepIndex'),
    'slothgit.stash.apply': async (arg?: Node) => {
      const node = stashTarget(arg);
      if (node) await run('Applying stash…', (ops) => ops.stashApply(node.stash.ref, false));
    },
    'slothgit.stash.pop': async (arg?: Node) => {
      const node = stashTarget(arg);
      if (node) await run('Popping stash…', (ops) => ops.stashApply(node.stash.ref, true));
    },
    'slothgit.stash.drop': async (arg?: Node) => {
      const node = stashTarget(arg);
      if (!node || !(await confirm(`Drop ${node.stash.ref}: "${node.stash.message}"?`, 'Drop'))) return;
      await run('Dropping stash…', (ops) => ops.stashDrop(node.stash.ref));
    },
    'slothgit.stash.rename': async (arg?: Node) => {
      const node = stashTarget(arg);
      if (!node) return;
      const message = await vscode.window.showInputBox({ title: `Rename ${node.stash.ref}`, value: node.stash.message });
      if (message === undefined || message === node.stash.message) return;
      await run('Renaming stash…', (ops) => ops.stashRename(node.stash.ref, node.stash.sha, message));
    },
    'slothgit.stash.branch': async (arg?: Node) => {
      const node = stashTarget(arg);
      if (!node) return;
      const name = await runner.askBranchName(`New branch from ${node.stash.ref}`);
      if (name) await run('Creating branch from stash…', (ops) => ops.stashBranch(name, node.stash.ref));
    },
  };

  async function stashPush(mode: 'all' | 'staged' | 'keepIndex'): Promise<void> {
    const message = await vscode.window.showInputBox({ title: 'Stash message', placeHolder: 'Optional' });
    if (message === undefined) return;
    await run('Stashing…', (ops) => ops.stashPush(mode, message));
  }

  async function withOperation(title: string, fn: (ops: GitOps, op: Operation) => Promise<void>): Promise<void> {
    const git = repos.git();
    const operation = git && (await getOperation(git));
    if (!operation) {
      void vscode.window.showInformationMessage('No rebase, merge, cherry-pick or revert is in progress.');
      return;
    }
    await run(title, (ops) => fn(ops, operation));
  }

  /** For reverting a merge: the parent whose side is kept (git revert -m). */
  async function pickMainline(merge: CommitNode): Promise<number | undefined> {
    const git = repos.git();
    if (!git) return undefined;
    const subjects = (await git.run(['log', '--no-walk=unsorted', '--format=%s', ...merge.parents])).split(/\r?\n/);
    const items = merge.parents.map((sha, i) => ({
      label: `Keep parent ${i + 1}: ${subjects[i] ?? ''}`,
      description: `${sha.slice(0, 7)}${i === 0 ? ' · the branch merged into (usual)' : ' · a merged-in branch'}`,
      mainline: i + 1,
    }));
    const picked = await vscode.window.showQuickPick(items, {
      title: `Revert merge ${merge.shortSha}: which side to keep? The changes from the other side are undone.`,
    });
    return picked?.mainline;
  }

  /** Remove the selected files' changes from their commit, discarding them or moving them to the index. */
  async function removeFiles(keep: boolean, arg?: Node, all?: Node[]): Promise<void> {
    const files = resolve<FileNode>('file', commitsView, arg, all).filter((f) => f.commit?.source === 'log');
    const commit = files[0]?.commit;
    if (!commit) return;
    if (files.some((f) => f.commit?.sha !== commit.sha)) {
      void vscode.window.showInformationMessage('Select files from a single commit.');
      return;
    }
    if (!(await canRewrite([commit]))) return;
    const names = files.length === 1 ? files[0].pair.change.path : `${files.length} files`;
    const question = keep
      ? `Take the changes to ${names} out of ${commit.shortSha} and stage them in your working tree?`
      : `Discard the changes to ${names} from ${commit.shortSha}? Later commits touching them may conflict.`;
    if (!(await confirm(question, keep ? 'Move to Index' : 'Remove'))) return;
    if (!(await confirmRewrite([commit]))) return;
    await run(keep ? 'Moving changes out of commit…' : 'Removing changes from commit…', (ops) =>
      ops.removeFilesFromCommit(
        commit.sha,
        files.map((f) => f.pair.change),
        keep,
      ),
    );
  }

  /**
   * The commit file diff in the active editor and the lines selected in it. An empty selection is the change block
   * under the cursor.
   */
  function activeCommitDiff() {
    const editor = vscode.window.activeTextEditor;
    const diff = editor && commitDiffs.get(editor.document.uri.toString());
    if (!editor || !diff?.file.commit) {
      void vscode.window.showInformationMessage(
        'Open a file diff from the Commits view, select the changed lines to take out of the commit, and try again.',
      );
      return undefined;
    }
    const ranges: LineRange[] = editor.selections.map((s) => {
      if (s.isEmpty) return { start: s.active.line + 1, end: s.active.line + 1, whole: true };
      // A selection that ends at the start of a line doesn't include that line.
      const end = s.end.character === 0 && s.end.line > s.start.line ? s.end.line : s.end.line + 1;
      return { start: s.start.line + 1, end };
    });
    return { diff, commit: diff.file.commit, ranges };
  }

  /**
   * Line-level version of removeFiles: takes the changes selected in a commit's file diff out of the commit.
   * An empty selection takes the change block under the cursor.
   */
  async function patchSelection(target: 'discard' | 'index' | 'newCommit'): Promise<void> {
    const active = activeCommitDiff();
    const ops = repos.ops();
    if (!active || !ops) return;
    const { diff, commit, ranges } = active;
    if (!(await canRewrite([commit]))) return;
    let patch: string | undefined;
    try {
      patch = removalPatch(await ops.commitFileDiff(commit.sha, diff.file.pair.change), diff.side, ranges);
    } catch (error) {
      void vscode.window.showWarningMessage(error instanceof Error ? error.message : String(error));
      return;
    }
    if (!patch) {
      void vscode.window.showInformationMessage(
        `The selection has no changes from ${commit.shortSha}. Select changed lines, or put the cursor on a change.`,
      );
      return;
    }
    const name = path.posix.basename(diff.file.pair.change.path);
    let newCommit: string | undefined;
    if (target === 'newCommit') {
      newCommit = await vscode.window.showInputBox({
        title: `New commit after ${commit.shortSha} with the selected changes to ${name}`,
        value: commit.subject,
        validateInput: (v) => (v.trim() ? undefined : 'The message cannot be empty.'),
      });
      if (newCommit === undefined) return;
    } else {
      const question =
        target === 'index'
          ? `Take the selected changes to ${name} out of ${commit.shortSha} and stage them in your working tree?`
          : `Discard the selected changes to ${name} from ${commit.shortSha}? Later commits touching those lines may conflict.`;
      if (!(await confirm(question, target === 'index' ? 'Move to Index' : 'Remove'))) return;
    }
    if (!(await confirmRewrite([commit]))) return;
    const where = newCommit !== undefined ? { newCommit } : (target as 'discard' | 'index');
    const title = { discard: 'Removing changes from commit…', index: 'Moving changes out of commit…', newCommit: 'Moving changes to a new commit…' }[target];
    await run(title, (ops) => ops.removePatchFromCommit(commit.sha, patch!, where));
  }

  // --- custom patch (lazygit's patch building) --------------------------------------------

  async function addToPatch(file: FileNode, side?: Side, ranges?: LineRange[]): Promise<void> {
    const current = patch.commit;
    const commit = file.commit!;
    if (current && current.sha !== commit.sha) {
      const question = `The custom patch has changes from ${current.shortSha}. Discard them and start a new patch from ${commit.shortSha}?`;
      if (!(await confirm(question, 'Start New Patch'))) return;
    }
    patch.add(file, side, ranges);
  }

  /** The custom patch as a removal patch for its commit, or undefined after saying why there is none. */
  async function builtPatch(ops: GitOps, forRewrite: boolean): Promise<{ commit: CommitNode; removal: string } | undefined> {
    const commit = patch.commit;
    if (!commit || !patch.entries.length) {
      void vscode.window.showInformationMessage(
        'The custom patch is empty. Add changes with "Add Selected Changes to Custom Patch" in a commit\'s file diff, or "Add to Custom Patch" on a file of a commit.',
      );
      return undefined;
    }
    if (forRewrite) {
      const onBranch = await ops.git.exec(['merge-base', '--is-ancestor', commit.sha, 'HEAD'], { allowedExitCodes: [1, 128] });
      if (onBranch.exitCode !== 0) {
        patch.clear();
        void vscode.window.showWarningMessage(
          `${commit.shortSha} is no longer on the current branch (it was rewritten, or another branch is checked out), so the custom patch was discarded.`,
        );
        return undefined;
      }
      if (!(await canRewrite([commit]))) return undefined;
    }
    try {
      const parts = await Promise.all(
        patch.entries.map(async (e) => selectionPatch(await ops.commitFileDiff(commit.sha, e.file.pair.change), e.selection)),
      );
      const removal = parts.filter(Boolean).join('');
      if (removal) return { commit, removal };
      void vscode.window.showInformationMessage(`The custom patch has no changes from ${commit.shortSha}.`);
    } catch (error) {
      void vscode.window.showWarningMessage(error instanceof Error ? error.message : String(error));
    }
    return undefined;
  }

  async function patchMenu(): Promise<void> {
    const commit = patch.commit;
    if (!commit) return void (await builtPatch(repos.ops()!, false));
    const items: (vscode.QuickPickItem & { run: () => Promise<unknown> })[] = [
      { label: '$(trash) Remove from Commit', description: `Discard the changes from ${commit.shortSha}`, run: () => rewritePatch('discard') },
      { label: '$(arrow-down) Move out to Index', description: 'Take them out of the commit and stage them', run: () => rewritePatch('index') },
      { label: '$(git-commit) Move to New Commit', description: `Split them into a new commit after ${commit.shortSha}`, run: () => rewritePatch('newCommit') },
      { label: '$(arrow-swap) Move into Another Commit…', description: 'Older or newer', run: () => pickPatchTarget() },
      { label: '$(check) Apply to Working Tree', description: 'History is unchanged', run: () => applyPatch(false) },
      { label: '$(discard) Apply in Reverse to Working Tree', description: 'History is unchanged', run: () => applyPatch(true) },
      { label: '$(eye) Show Patch', run: () => showPatch() },
      { label: '$(close) Reset Patch', run: async () => patch.clear() },
    ];
    const files = patch.entries.map((e) => e.file.pair.change.path).join(', ');
    const picked = await vscode.window.showQuickPick(items, { title: `Custom patch from ${commit.shortSha}: ${files}` });
    await picked?.run();
  }

  async function rewritePatch(target: 'discard' | 'index' | 'newCommit'): Promise<void> {
    const ops = repos.ops();
    const built = ops && (await builtPatch(ops, true));
    if (!built) return;
    const { commit, removal } = built;
    let where: 'discard' | 'index' | { newCommit: string } = target as 'discard' | 'index';
    if (target === 'newCommit') {
      const message = await vscode.window.showInputBox({
        title: `New commit after ${commit.shortSha} with the custom patch`,
        value: commit.subject,
        validateInput: (v) => (v.trim() ? undefined : 'The message cannot be empty.'),
      });
      if (message === undefined) return;
      where = { newCommit: message };
    } else if (target === 'discard' && !(await confirm(`Discard the custom patch's changes from ${commit.shortSha}? Later commits touching those lines may conflict.`, 'Remove'))) {
      return;
    }
    if (!(await confirmRewrite([commit]))) return;
    const title = { discard: 'Removing patch from commit…', index: 'Moving patch out of commit…', newCommit: 'Moving patch to a new commit…' }[target];
    if (await run(title, (ops) => ops.removePatchFromCommit(commit.sha, removal, where))) patch.clear();
  }

  async function pickPatchTarget(): Promise<void> {
    const source = patch.commit;
    const items = commits.commits
      .filter((c) => c.source === 'log' && c.onHead !== false && c.sha !== source?.sha)
      .map((c) => ({ label: `${c.shortSha}  ${c.subject}`, description: c.relDate, commit: c }));
    const picked = await vscode.window.showQuickPick(items, { title: 'Move the custom patch into which commit?', matchOnDescription: true });
    if (picked) await movePatch(picked.commit);
  }

  async function movePatch(target: CommitNode): Promise<void> {
    const ops = repos.ops();
    if (!ops || !(await canRewrite([target]))) return;
    const built = await builtPatch(ops, true);
    if (!built) return;
    const { commit, removal } = built;
    if (target.sha === commit.sha) {
      void vscode.window.showInformationMessage('The custom patch is already from this commit. Pick a different commit to move it into.');
      return;
    }
    if (!(await confirmRewrite([commit, target]))) return;
    if (await run('Moving patch into commit…', (ops) => ops.movePatchToCommit(commit.sha, removal, target.sha))) patch.clear();
  }

  async function applyPatch(reverse: boolean): Promise<void> {
    const ops = repos.ops();
    const built = ops && (await builtPatch(ops, false));
    if (!built) return;
    await run(reverse ? 'Applying patch in reverse…' : 'Applying patch…', (ops) => ops.applyPatch(built.commit.sha, built.removal, reverse));
  }

  async function showPatch(): Promise<void> {
    const ops = repos.ops();
    const built = ops && (await builtPatch(ops, false));
    if (!built) return;
    const content = await ops.patchContent(built.commit.sha, built.removal);
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument({ language: 'diff', content }), { preview: true, viewColumn: editorColumn() });
  }

  function setBase(sha: string | undefined): void {
    commits.baseSha = sha;
    setContext('slothgit.hasRebaseBase', !!sha);
    commits.redraw();
    if (sha) {
      void vscode.window.showInformationMessage(
        'Base commit marked. "Rebase Current Branch onto This" in the Branches view will now move only the commits above it.',
      );
    }
  }

  // fromWebview: the tab's and the graph's context menus pass a webview context instead of a node.
  return [
    patchContext,
    ...Object.entries(commands).map(([id, handler]) => vscode.commands.registerCommand(id, (...args: unknown[]) => handler(...fromWebview(args)))),
  ];
}

/**
 * The sides of commit file diffs SlothGit opened, by document URI, so the line-level patch commands know which
 * commit and side an editor shows. Only for the Commits view, whose history can be rewritten.
 */
const commitDiffs = new Map<string, { file: FileNode; side: Side }>();
const MAX_COMMIT_DIFFS = 500;

function rememberCommitDiff(file: FileNode, left: vscode.Uri, right: vscode.Uri): void {
  if (file.commit?.source !== 'log') return;
  for (const [uri, side] of [[left, 'old'], [right, 'new']] as const) {
    const key = uri.toString();
    commitDiffs.delete(key);
    commitDiffs.set(key, { file, side });
  }
  while (commitDiffs.size > MAX_COMMIT_DIFFS) commitDiffs.delete(commitDiffs.keys().next().value!);
}

function updatePatchContext(editor: vscode.TextEditor | undefined): void {
  const known = !!editor && commitDiffs.has(editor.document.uri.toString());
  setContext('slothgit.inCommitDiff', known);
}

export function diffUris(repos: RepoManager, node: FileNode) {
  const root = repos.repository!.rootUri;
  const { change, leftRef, rightRef } = node.pair;
  const uri = vscode.Uri.joinPath(root, change.path);
  const leftUri = vscode.Uri.joinPath(root, change.oldPath ?? change.path);
  return {
    uri,
    left: repos.api.toGitUri(leftUri, leftRef),
    right: repos.api.toGitUri(uri, rightRef),
  };
}

export async function openMultiDiff(repos: RepoManager, title: string, files: FileNode[]): Promise<void> {
  if (!files.length) {
    void vscode.window.showInformationMessage('No changes.');
    return;
  }
  const resources = files.map((f) => {
    const { left, right, uri } = diffUris(repos, f);
    rememberCommitDiff(f, left, right);
    // Like the Git extension: no original for added files, no modified for deleted ones.
    const { status } = f.pair.change;
    return [uri, status === 'A' ? undefined : left, status === 'D' ? undefined : right];
  });
  // The multi-diff editor renders lazily, so loading the first screenful is enough.
  const firstScreen = files.slice(0, PRELOAD_FILES).flatMap((f) => existingSides(diffUris(repos, f), f));
  await loadDocuments(`Loading changes (${files.length} ${files.length === 1 ? 'file' : 'files'})…`, firstScreen);
  await leaveTab();
  await vscode.commands.executeCommand('vscode.changes', title, resources);
}

/** The sides of a diff that have content: an added file has no left side, a deleted one no right side. */
function existingSides({ left, right }: { left: vscode.Uri; right: vscode.Uri }, file: FileNode): vscode.Uri[] {
  const { status } = file.pair.change;
  return status === 'A' ? [right] : status === 'D' ? [left] : [left, right];
}

const PRELOAD_FILES = 15;
const NOTIFY_AFTER_MS = 400;

/**
 * Loads git: documents before an editor shows them. Their content comes from `git show`, which can take a
 * moment, and an editor opened before that shows up empty. A progress notification appears if loading
 * takes noticeably long.
 */
export async function loadDocuments(title: string, uris: vscode.Uri[]): Promise<void> {
  const queue = [...uris];
  let done = 0;
  let progress: vscode.Progress<{ message?: string }> | undefined;
  const report = () => uris.length > 2 && progress?.report({ message: `${Math.round((done / uris.length) * 100)}%` });
  const worker = async () => {
    for (let uri = queue.shift(); uri; uri = queue.shift()) {
      try {
        await vscode.workspace.openTextDocument(uri);
      } catch {
        // Binary files and missing sides (added/deleted files): the editor explains those itself.
      }
      done++;
      report();
    }
  };
  const work = Promise.all(Array.from({ length: 4 }, worker));
  const timer = setTimeout(() => {
    void vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title }, (p) => {
      progress = p;
      report();
      return work;
    });
  }, NOTIFY_AFTER_MS);
  try {
    await work;
  } finally {
    clearTimeout(timer);
  }
}

export async function createTag(runner: Runner, rev: string, label: string): Promise<void> {
  const name = await vscode.window.showInputBox({
    title: `New tag at ${label}`,
    placeHolder: 'Tag name',
    validateInput: (v) => (/^\S+$/.test(v) ? undefined : 'Enter a tag name without spaces.'),
  });
  if (!name) return;
  const message = await vscode.window.showInputBox({
    title: `Message for tag ${name}`,
    placeHolder: 'Optional: leave empty for a lightweight tag',
  });
  if (message === undefined) return;
  await runner.run('Creating tag…', (ops) => ops.createTag(name, rev, message));
}

/** The web repository for the preferred remote (the given one, else origin, else the first). */
export async function webRepo(git: Git, preferredRemote?: string): Promise<WebRepo | undefined> {
  const remotes = await getRemotes(git);
  const remote =
    remotes.find((r) => r.name === preferredRemote) ?? remotes.find((r) => r.name === 'origin') ?? remotes[0];
  return remote && toWebRepo(remote.url);
}

/**
 * Opens `initial` in an editor tab and resolves with the saved text once the tab is closed, or undefined when
 * the text is unchanged or empty. Lines starting with '#' are dropped, like git's own message editing.
 */
export async function editInEditor(file: string, initial: string, title: string): Promise<string | undefined> {
  const help = [
    '',
    `# ${title}: save and close this tab to apply.`,
    '# Lines starting with # are ignored. Close without changes, or leave it empty, to cancel.',
  ];
  fs.writeFileSync(file, [initial, ...help].join('\n') + '\n');
  const uri = vscode.Uri.file(file);
  const doc = await vscode.languages.setTextDocumentLanguage(await vscode.workspace.openTextDocument(uri), 'git-commit');
  await vscode.window.showTextDocument(doc, { preview: false, viewColumn: editorColumn() });

  await new Promise<void>((resolve) => {
    const isOurs = (tab: vscode.Tab) => tab.input instanceof vscode.TabInputText && tab.input.uri.fsPath === uri.fsPath;
    const listener = vscode.window.tabGroups.onDidChangeTabs((e) => {
      const stillOpen = vscode.window.tabGroups.all.some((g) => g.tabs.some(isOurs));
      if (e.closed.some(isOurs) && !stillOpen) {
        listener.dispose();
        resolve();
      }
    });
  });

  const text = fs
    .readFileSync(file, 'utf8')
    .split(/\r?\n/)
    .filter((line) => !line.startsWith('#'))
    .join('\n')
    .trim();
  fs.rmSync(file, { force: true });
  return text ? text : undefined;
}
