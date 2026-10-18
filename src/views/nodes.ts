import * as path from 'node:path';
import * as vscode from 'vscode';
import type { LocalBranch, Person, RefPair, Remote, RemoteBranch, Stash, Tag, Worktree } from '../git/queries';
import type { TodoEntry } from '../git/todo';
import type { StatusEntry } from '../git/workingTree';

export interface CommitNode {
  kind: 'commit';
  /** log: the Commits view; reflog: the Reflog view; ref: under a branch or tag in the Branches view. */
  source: 'log' | 'reflog' | 'ref';
  /** For source 'ref': the branch or tag this commit is listed under. */
  parentRev?: string;
  sha: string;
  shortSha: string;
  subject: string;
  parents: string[];
  relDate: string;
  /** Position in its list; 0 is the newest. */
  index: number;
  author?: string;
  authorEmail?: string;
  coAuthors?: Person[];
  /** Author date, ISO 8601. */
  date?: string;
  refs?: string[];
  selector?: string;
  /** False for commits of other branches, listed in "all branches" mode; they can't be rewritten. */
  onHead?: boolean;
}

export interface StashNode {
  kind: 'stash';
  stash: Stash;
}

export interface FileNode {
  kind: 'file';
  pair: RefPair;
  /** Shown in diff titles, e.g. a short SHA or stash ref. */
  label: string;
  /** The commit this file change belongs to, when listed under a commit. */
  commit?: CommitNode;
}

/** A remaining step of a paused interactive rebase, shown at the top of the Commits view. */
export interface TodoNode {
  kind: 'todo';
  action: TodoEntry['action'];
  /** Abbreviated SHA as written in the todo list. */
  sha: string;
  subject: string;
  /** For update-ref steps: the stacked branch's full ref name. */
  ref?: string;
}

export interface WorktreeNode {
  kind: 'worktree';
  worktree: Worktree;
}

interface RefNodeBase {
  kind: 'ref';
  /** Display name: "main", "origin/main", "v1.0". */
  name: string;
  /** What to pass to git commands taking a revision. */
  rev: string;
  sha: string;
}

export type RefNode =
  | (RefNodeBase & { type: 'local'; branch: LocalBranch })
  | (RefNodeBase & { type: 'remote'; branch: RemoteBranch })
  | (RefNodeBase & { type: 'tag'; tag: Tag });

export interface RemoteNode {
  kind: 'remote';
  remote: Remote;
  branches: RefNode[];
}

export interface SectionNode {
  kind: 'section';
  section: 'local' | 'remotes' | 'tags' | 'worktrees';
  children: (RefNode | RemoteNode | WorktreeNode)[];
}

/** An uncommitted change in the Files panel. */
export interface ChangeNode {
  kind: 'change';
  entry: StatusEntry;
}

export type Node = CommitNode | StashNode | FileNode | RefNode | RemoteNode | SectionNode | TodoNode | WorktreeNode | ChangeNode;

const STATUS_NAMES: Record<string, string> = {
  A: 'Added',
  M: 'Modified',
  D: 'Deleted',
  R: 'Renamed',
  C: 'Copied',
  T: 'Type changed',
  U: 'Untracked',
};

export function fileTreeItem(node: FileNode, root: vscode.Uri): vscode.TreeItem {
  const { change } = node.pair;
  const uri = vscode.Uri.joinPath(root, change.path);
  const item = new vscode.TreeItem(uri, vscode.TreeItemCollapsibleState.None);
  const dir = path.posix.dirname(change.path);
  item.label = path.posix.basename(change.path);
  item.description = dir === '.' ? change.status : `${dir} · ${change.status}`;
  item.tooltip = `${STATUS_NAMES[change.status] ?? change.status}: ${change.oldPath ? `${change.oldPath} → ` : ''}${change.path}`;
  // file:log = under a commit in the Commits view (history can be rewritten), file:commit = other commits, file:stash.
  item.contextValue = node.commit ? (node.commit.source === 'log' ? 'file:log' : 'file:commit') : 'file:stash';
  item.command = { command: 'slothgit.openFileDiff', title: 'Open Diff', arguments: [node] };
  return item;
}
