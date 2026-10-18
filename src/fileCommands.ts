import * as path from 'node:path';
import * as vscode from 'vscode';
import { isConflict, isStaged, isUnstaged, isUntracked, type StatusEntry } from './git/workingTree';
import type { RepoManager } from './repoManager';
import type { Runner } from './runner';
import type { FilesProvider } from './views/files';
import type { ChangeNode, Node } from './views/nodes';
import { editorColumn } from './views/editorPlacement';
import { fromWebview, type ListView } from './views/webviewNodes';

export interface FileCommandDeps {
  repos: RepoManager;
  files: FilesProvider;
  filesView: ListView;
  runner: Runner;
}

/** lazygit's Files panel: stage, unstage, discard and open uncommitted changes. */
export function registerFileCommands(deps: FileCommandDeps): vscode.Disposable[] {
  const { repos, files, filesView, runner } = deps;
  const { run, confirm } = runner;

  /** The clicked change and the selection it is part of, or the Files panel's selection. */
  function targets(arg?: Node, all?: Node[]): StatusEntry[] {
    const nodes = all?.length ? all : arg ? [arg] : [...filesView.selection];
    const entries = nodes.filter((n): n is ChangeNode => n.kind === 'change').map((n) => n.entry);
    if (!entries.length) void vscode.window.showInformationMessage('Select a changed file in the Files panel first.');
    return entries;
  }

  const names = (entries: StatusEntry[]) => (entries.length === 1 ? entries[0].path : `${entries.length} files`);

  function uriOf(file: string): vscode.Uri | undefined {
    const root = repos.repository?.rootUri;
    return root && vscode.Uri.joinPath(root, file);
  }

  /** The change as the Source Control view opens it: staged against HEAD, or the working tree against the index. */
  async function openChange(entry: StatusEntry): Promise<void> {
    const uri = uriOf(entry.path);
    if (!uri) return;
    if (isConflict(entry)) {
      await vscode.commands.executeCommand('git.openMergeEditor', uri);
      return;
    }
    const name = path.posix.basename(entry.path);
    if (isUntracked(entry) || (entry.y === ' ' && entry.x === 'A')) {
      await vscode.commands.executeCommand('vscode.open', uri, { viewColumn: editorColumn() });
      return;
    }
    const staged = isStaged(entry) && !isUnstaged(entry);
    if (staged) {
      const left = repos.api.toGitUri(uriOf(entry.oldPath ?? entry.path)!, 'HEAD');
      const right = entry.x === 'D' ? left : repos.api.toGitUri(uri, '');
      await vscode.commands.executeCommand('vscode.diff', left, right, `${name} (Index)`, { preview: true, viewColumn: editorColumn() });
    } else {
      const left = repos.api.toGitUri(uri, '~');
      await vscode.commands.executeCommand('vscode.diff', left, uri, `${name} (Working Tree)`, { preview: true, viewColumn: editorColumn() });
    }
  }

  const commands: Record<string, (...args: any[]) => unknown> = {
    'slothgit.file.stage': async (arg?: Node, all?: Node[]) => {
      const entries = targets(arg, all).filter((e) => isUnstaged(e) || isConflict(e));
      if (entries.length) await run('Staging…', (ops) => ops.stage(entries));
    },
    'slothgit.file.unstage': async (arg?: Node, all?: Node[]) => {
      const entries = targets(arg, all).filter(isStaged);
      if (entries.length) await run('Unstaging…', (ops) => ops.unstage(entries));
    },
    'slothgit.file.discard': async (arg?: Node, all?: Node[]) => {
      const entries = targets(arg, all).filter((e) => !isConflict(e));
      if (!entries.length) return;
      // With staged and unstaged changes, lazygit asks which; so does this.
      const mixed = entries.some((e) => isStaged(e) && isUnstaged(e));
      let unstagedOnly = false;
      if (mixed) {
        const picked = await vscode.window.showQuickPick(
          [
            { label: 'Discard All Changes', description: 'Staged and unstaged', unstagedOnly: false },
            { label: 'Discard Unstaged Changes', description: 'Keep what is staged', unstagedOnly: true },
          ],
          { title: `Discard changes to ${names(entries)}` },
        );
        if (!picked) return;
        unstagedOnly = picked.unstagedOnly;
      }
      const untracked = entries.some(isUntracked);
      const question = `Discard ${unstagedOnly ? 'the unstaged changes to' : 'the changes to'} ${names(entries)}?${untracked ? ' New files are deleted.' : ''} This cannot be undone.`;
      if (!(await confirm(question, 'Discard'))) return;
      await run('Discarding…', (ops) => ops.discard(entries, unstagedOnly));
    },
    'slothgit.file.open': async (arg?: Node, all?: Node[]) => {
      for (const entry of targets(arg, all)) {
        const uri = uriOf(entry.path);
        if (uri && entry.y !== 'D' && entry.x !== 'D') await vscode.commands.executeCommand('vscode.open', uri, { preview: false, viewColumn: editorColumn() });
      }
    },
    'slothgit.file.openChange': async (arg?: Node) => {
      const [entry] = targets(arg);
      if (entry) await openChange(entry);
    },
    'slothgit.file.openMergeEditor': async (arg?: Node) => {
      const [entry] = targets(arg);
      const uri = entry && uriOf(entry.path);
      if (uri) await vscode.commands.executeCommand('git.openMergeEditor', uri);
    },
    'slothgit.files.stageAll': async () => {
      if (!files.entries.some(isUnstaged)) return void vscode.window.showInformationMessage('There is nothing to stage.');
      await run('Staging all changes…', (ops) => ops.stageAll());
    },
    'slothgit.files.unstageAll': async () => {
      if (!files.entries.some(isStaged)) return void vscode.window.showInformationMessage('Nothing is staged.');
      await run('Unstaging all changes…', (ops) => ops.unstageAll());
    },
    'slothgit.files.discardAll': async () => {
      if (!files.entries.length) return void vscode.window.showInformationMessage('There are no changes to discard.');
      const question = `Discard all ${files.entries.length === 1 ? 'changes' : `changes to ${files.entries.length} files`}, staged and unstaged, and delete untracked files? This cannot be undone.`;
      if (!(await confirm(question, 'Discard All'))) return;
      await run('Discarding all changes…', (ops) => ops.discardAll());
    },
  };

  // fromWebview: the tab's context menus pass a webview context instead of a node.
  return Object.entries(commands).map(([id, handler]) => vscode.commands.registerCommand(id, (...args: unknown[]) => handler(...fromWebview(args))));
}
