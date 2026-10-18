import * as path from 'node:path';
import * as vscode from 'vscode';
import type { Git } from '../git/runner';
import { getStatus, isConflict, isStaged, isUnstaged, isUntracked, type StatusEntry } from '../git/workingTree';
import { BaseProvider } from './baseProvider';
import type { ChangeNode, Node } from './nodes';

const LETTERS: Record<string, string> = {
  M: 'modified',
  A: 'added',
  D: 'deleted',
  R: 'renamed',
  C: 'copied',
  T: 'type changed',
  U: 'unmerged',
  '?': 'untracked',
};

/** What a status means, for tooltips: "staged: modified, unstaged: deleted". */
export function describeStatus(e: StatusEntry): string {
  if (isConflict(e)) return `conflict (${e.x}${e.y}): both sides changed it`;
  if (isUntracked(e)) return 'untracked: new file, not staged yet';
  const parts = [];
  if (e.x !== ' ') parts.push(`staged: ${LETTERS[e.x] ?? e.x}`);
  if (e.y !== ' ') parts.push(`unstaged: ${LETTERS[e.y] ?? e.y}`);
  return parts.join(', ');
}

/** lazygit's Files panel: every changed path once, with git's two status letters. */
export class FilesProvider extends BaseProvider<ChangeNode> {
  get entries(): StatusEntry[] {
    return this.roots.map((n) => n.entry);
  }

  protected async load(git: Git): Promise<ChangeNode[]> {
    return (await getStatus(git)).map((entry) => ({ kind: 'change', entry }));
  }

  protected keyOf(roots: ChangeNode[]): string {
    return roots.map((n) => `${n.entry.x}${n.entry.y} ${n.entry.path} ${n.entry.oldPath ?? ''}`).join('\n');
  }

  protected description(): string | undefined {
    const conflicts = this.entries.filter(isConflict).length;
    const staged = this.entries.filter(isStaged).length;
    return [conflicts && `${conflicts} conflicts`, staged && `${staged} staged`].filter(Boolean).join(' · ') || undefined;
  }

  protected item(n: Node): vscode.TreeItem {
    const { entry } = n as ChangeNode;
    const dir = path.posix.dirname(entry.path);
    const item = new vscode.TreeItem(path.posix.basename(entry.path), vscode.TreeItemCollapsibleState.None);
    item.id = `change:${entry.path}`;
    item.description = [dir === '.' ? '' : dir, entry.oldPath && `← ${entry.oldPath}`].filter(Boolean).join(' ');
    item.tooltip = `${entry.path}\n${describeStatus(entry)}`;
    const flags = [
      isStaged(entry) && 'staged',
      isUnstaged(entry) && 'unstaged',
      isUntracked(entry) && 'untracked',
      isConflict(entry) && 'conflict',
      (entry.x === 'D' || entry.y === 'D') && 'deleted',
    ].filter(Boolean);
    item.contextValue = ['change', ...flags].join(':');
    return item;
  }

  protected async children(): Promise<Node[]> {
    return [];
  }
}
