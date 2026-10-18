// The working tree and index: lazygit's Files panel. Pure git, no vscode, so it can be unit tested.
import * as fs from 'node:fs';
import * as path from 'node:path';
import type { Git } from './runner';

/** One path from `git status`, with git's two status letters: X for the index, Y for the working tree. */
export interface StatusEntry {
  path: string;
  /** Where a staged rename or copy came from. */
  oldPath?: string;
  x: string;
  y: string;
}

/** Unmerged combinations, as `git status` documents them. */
const CONFLICTS = new Set(['DD', 'AU', 'UD', 'UA', 'DU', 'AA', 'UU']);

export const isConflict = (e: StatusEntry) => CONFLICTS.has(e.x + e.y);
export const isUntracked = (e: StatusEntry) => e.x === '?';
/** Has changes in the index (staged). */
export const isStaged = (e: StatusEntry) => !isConflict(e) && !isUntracked(e) && e.x !== ' ';
/** Has changes in the working tree that are not staged, untracked files included. */
export const isUnstaged = (e: StatusEntry) => !isConflict(e) && (isUntracked(e) || e.y !== ' ');

/** Parses `git status --porcelain=v1 -z`. A rename or copy is "XY new\0old\0". */
export function parseStatus(output: string): StatusEntry[] {
  const tokens = output.split('\0');
  const entries: StatusEntry[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (token.length < 4) continue;
    const entry: StatusEntry = { x: token[0], y: token[1], path: token.slice(3) };
    if (entry.x === 'R' || entry.x === 'C') entry.oldPath = tokens[++i];
    entries.push(entry);
  }
  // Conflicts first, as they block everything else; then by path.
  return entries.sort((a, b) => Number(isConflict(b)) - Number(isConflict(a)) || a.path.localeCompare(b.path));
}

export async function getStatus(git: Git): Promise<StatusEntry[]> {
  return parseStatus(await git.run(['status', '--porcelain=v1', '-z', '--untracked-files=all']));
}

export async function hasHead(git: Git): Promise<boolean> {
  return (await git.exec(['rev-parse', '-q', '--verify', 'HEAD'], { allowedExitCodes: [1, 128] })).exitCode === 0;
}

const paths = (entries: StatusEntry[]) => [...new Set(entries.flatMap((e) => (e.oldPath ? [e.oldPath, e.path] : [e.path])))];

export async function stage(git: Git, entries: StatusEntry[]): Promise<void> {
  if (entries.length) await git.exec(['add', '-A', '--', ...paths(entries)]);
}

export async function unstage(git: Git, entries: StatusEntry[]): Promise<void> {
  if (!entries.length) return;
  // Before the first commit there is no HEAD to restore the index from.
  if (await hasHead(git)) await git.exec(['restore', '--staged', '--', ...paths(entries)]);
  else await git.exec(['rm', '-q', '-r', '--cached', '--', ...paths(entries)]);
}

/**
 * Throws away a file's changes, staged and unstaged: tracked files go back to HEAD, files new since HEAD are
 * deleted. With `unstagedOnly`, only the working tree goes back to the index and untracked files are deleted.
 */
export async function discard(git: Git, entries: StatusEntry[], unstagedOnly = false): Promise<void> {
  const untracked = entries.filter(isUntracked);
  const tracked = entries.filter((e) => !isUntracked(e));
  if (untracked.length) await git.exec(['clean', '-f', '-q', '--', ...untracked.map((e) => e.path)]);
  if (!tracked.length) return;
  if (unstagedOnly) {
    const changed = tracked.filter((e) => e.y !== ' ');
    if (changed.length) await git.exec(['restore', '--worktree', '--', ...changed.map((e) => e.path)]);
    return;
  }
  // Added in the index (or the new side of a rename): not in HEAD, so removing it is how it goes back.
  const added = tracked.filter((e) => e.x === 'A' || e.x === 'R' || e.x === 'C');
  const inHead = tracked.filter((e) => !added.includes(e));
  const restore = [...inHead.map((e) => e.path), ...added.flatMap((e) => (e.x === 'R' && e.oldPath ? [e.oldPath] : []))];
  if (added.length) await git.exec(['rm', '-q', '-f', '-r', '--', ...added.map((e) => e.path)]);
  if (restore.length) {
    if (await hasHead(git)) await git.exec(['restore', '--source=HEAD', '--staged', '--worktree', '--', ...restore]);
    else await git.exec(['rm', '-q', '-f', '-r', '--', ...restore]);
  }
}

/** The diff of a file's changes: `staged` against HEAD, otherwise the working tree against the index. */
export async function workingDiff(git: Git, entry: StatusEntry, staged: boolean): Promise<string> {
  if (isUntracked(entry)) return untrackedDiff(git, entry.path);
  const args = ['-c', 'core.quotePath=false', 'diff', '--no-color', '--no-ext-diff', '-M'];
  if (staged) args.push('--cached');
  args.push('--', ...(staged && entry.oldPath ? [entry.oldPath, entry.path] : [entry.path]));
  return git.run(args);
}

/** An untracked file as an all-added diff, without running git. */
function untrackedDiff(git: Git, file: string): string {
  const header = `diff --git a/${file} b/${file}\nnew file mode 100644\n--- /dev/null\n+++ b/${file}\n`;
  let content: Buffer;
  try {
    content = fs.readFileSync(path.join(git.cwd, file));
  } catch {
    return header;
  }
  if (content.includes(0)) return `diff --git a/${file} b/${file}\nnew file mode 100644\nBinary files /dev/null and b/${file} differ\n`;
  const lines = content.toString('utf8').split(/\r?\n/);
  const missingNewline = lines[lines.length - 1] !== '';
  if (!missingNewline) lines.pop();
  if (!lines.length) return header;
  return `${header}@@ -0,0 +1,${lines.length} @@\n${lines.map((l) => `+${l}`).join('\n')}\n${missingNewline ? '\\ No newline at end of file\n' : ''}`;
}
