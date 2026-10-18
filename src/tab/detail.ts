// What the SlothGit tab's detail pane shows for the selected row: lazygit's main panel. A commit's message, files
// and patch; a branch's log; a stash's patch; an uncommitted file's staged and unstaged changes.
import * as path from 'node:path';
import { limitDiff, parseDiff } from '../git/diff';
import { parseRefs } from '../git/graph';
import { getCommitMessage, getLog, getStashes } from '../git/queries';
import type { Git } from '../git/runner';
import { isConflict, isStaged, isUnstaged, workingDiff } from '../git/workingTree';
import type { BaseProvider } from '../views/baseProvider';
import { describeStatus } from '../views/files';
import type { ChangeNode, CommitNode, Node, RefNode, RemoteNode, StashNode, TodoNode, WorktreeNode } from '../views/nodes';
import type { DetailBlock, TabDetail, TabRow } from '../webview/messages';

/** About this many diff lines are drawn at first; "Show the whole diff" draws the rest. */
const DIFF_LINES = 3000;
const US = '\x1f';

/** A node with the provider that produced it and the kind of list it came from. */
export interface Source {
  node: Node;
  provider: BaseProvider<Node>;
  view: string;
}

export interface DetailContext {
  git: Git;
  /** A row for the detail pane, registered so menus and clicks find its node. */
  row(source: Source): TabRow;
  remotes: string[];
  /** Commits for a rebase step: the Commits list's provider. */
  commits: BaseProvider<Node>;
}

/** Stable key of what a node shows, so a detail can be refreshed after a reload. */
export function detailKey(node: Node): string {
  switch (node.kind) {
    case 'commit':
      return `commit:${node.source}:${node.sha}`;
    case 'todo':
      return `todo:${node.sha}:${node.ref ?? ''}`;
    case 'stash':
      return `stash:${node.stash.sha}`;
    case 'ref':
      return `ref:${node.type}:${node.name}`;
    case 'remote':
      return `remote:${node.remote.name}`;
    case 'worktree':
      return `worktree:${node.worktree.path}`;
    case 'change':
      return `change:${node.entry.path}`;
    case 'file':
      return `file:${node.label}:${node.pair.change.path}`;
    case 'section':
      return `section:${node.section}`;
  }
}

export async function buildDetail(source: Source, context: DetailContext, full: boolean): Promise<TabDetail> {
  const { node } = source;
  const key = detailKey(node);
  switch (node.kind) {
    case 'commit':
      return commitDetail(source as Source & { node: CommitNode }, context, full, key);
    case 'todo':
      return todoDetail(node, context, full, key);
    case 'stash':
      return stashDetail(source as Source & { node: StashNode }, context, full, key);
    case 'ref':
      return refDetail(source as Source & { node: RefNode }, context, key);
    case 'remote':
      return remoteDetail(source as Source & { node: RemoteNode }, context, key);
    case 'worktree':
      return worktreeDetail(node, key);
    case 'change':
      return changeDetail(node, context, full, key);
    default:
      return { title: '', key, blocks: [] };
  }
}

function diffBlock(text: string, full: boolean, title?: string): DetailBlock {
  const files = parseDiff(text);
  const limited = full ? { files, omitted: 0 } : limitDiff(files, DIFF_LINES);
  return { type: 'diff', title, files: limited.files, omitted: limited.omitted, full };
}

const DIFF_ARGS = ['-c', 'core.quotePath=false'];

/** The commit's patch against its first parent, as its file list shows it. */
function commitPatch(git: Git, sha: string, parent: string | undefined): Promise<string> {
  return parent
    ? git.run([...DIFF_ARGS, 'diff', '--no-color', '--no-ext-diff', '-M', parent, sha, '--'])
    : git.run([...DIFF_ARGS, 'diff-tree', '-p', '-r', '--root', '--no-commit-id', '--no-color', '--no-ext-diff', '-M', sha, '--']);
}

async function commitDetail(source: Source & { node: CommitNode }, context: DetailContext, full: boolean, key: string): Promise<TabDetail> {
  const { node } = source;
  const { git } = context;
  const format = ['%an <%ae>', '%aI', '%cn <%ce>', '%cI', '%P', '%D'].join(US);
  const [message, info, files, patch] = await Promise.all([
    getCommitMessage(git, node.sha),
    git.run(['log', '-1', `--format=${format}`, node.sha]),
    source.provider.getChildren(node),
    commitPatch(git, node.sha, node.parents[0]),
  ]);
  const [author, authorDate, committer, commitDate, parents, decorations] = info.trim().split(US);
  const [subject, ...body] = message.split('\n');
  const meta: { label: string; value: string; title?: string }[] = [
    { label: 'Author', value: author, title: new Date(authorDate).toLocaleString() },
    ...(node.coAuthors ?? []).map((p) => ({ label: 'Co-author', value: `${p.name}${p.email ? ` <${p.email}>` : ''}` })),
    { label: 'Date', value: `${new Date(authorDate).toLocaleString()}${node.relDate ? ` (${node.relDate})` : ''}` },
  ];
  if (committer !== author) meta.push({ label: 'Committer', value: committer, title: new Date(commitDate).toLocaleString() });
  meta.push({ label: 'Commit', value: node.sha });
  if (parents) meta.push({ label: parents.includes(' ') ? 'Parents' : 'Parent', value: parents.split(' ').map((p) => p.slice(0, 10)).join(', ') });
  if (node.source === 'reflog' && node.selector) meta.push({ label: 'Reflog', value: `${node.selector}: ${node.subject}` });
  const refs = parseRefs(decorations ? decorations.split(', ') : [], context.remotes);
  return {
    title: `${node.shortSha} ${subject}`,
    key,
    blocks: [
      { type: 'message', subject, body: body.join('\n').trim() },
      ...(refs.length ? [{ type: 'refs' as const, refs }] : []),
      { type: 'meta', items: meta },
      { type: 'rows', title: plural(files.length, 'file'), rows: files.map((f) => context.row({ node: f, provider: source.provider, view: source.view })), empty: 'No changes.' },
      diffBlock(patch, full),
    ],
  };
}

async function todoDetail(node: TodoNode, context: DetailContext, full: boolean, key: string): Promise<TabDetail> {
  if (node.action === 'update-ref' || !node.sha) {
    return { title: node.subject, key, blocks: [{ type: 'text', text: node.action === 'update-ref' ? `Pending rebase step: ${node.subject}.` : `Pending rebase step: ${node.action} ${node.subject}.` }] };
  }
  const [commit] = await getLog(context.git, 1, node.sha);
  if (!commit) return { title: node.subject, key, blocks: [{ type: 'text', text: `${node.sha} was not found.` }] };
  const detail = await commitDetail({ node: { kind: 'commit', source: 'ref', index: 0, ...commit }, provider: context.commits, view: 'commits' }, context, full, key);
  detail.blocks.unshift({ type: 'text', text: `Pending rebase step: ${node.action}. Steps run from the bottom of the list up.` });
  return detail;
}

async function stashDetail(source: Source & { node: StashNode }, context: DetailContext, full: boolean, key: string): Promise<TabDetail> {
  const { stash } = source.node;
  const { git } = context;
  const show = (untracked: boolean) =>
    git.exec([...DIFF_ARGS, 'stash', 'show', '-p', '-M', '--no-color', '--no-ext-diff', ...(untracked ? ['--include-untracked'] : []), stash.sha], {
      allowedExitCodes: [128, 129],
    });
  const [files, withUntracked] = await Promise.all([source.provider.getChildren(source.node), show(true)]);
  // --include-untracked needs git 2.32.
  const patch = withUntracked.exitCode === 0 ? withUntracked.stdout : (await show(false)).stdout;
  const index = (await getStashes(git)).findIndex((s) => s.sha === stash.sha);
  return {
    title: `${stash.ref}: ${stash.message}`,
    key,
    blocks: [
      { type: 'message', subject: stash.message, body: '' },
      { type: 'meta', items: [{ label: 'Stash', value: index >= 0 ? `stash@{${index}}` : stash.ref }, { label: 'Created', value: stash.relDate }, { label: 'Commit', value: stash.sha }] },
      { type: 'rows', title: plural(files.length, 'file'), rows: files.map((f) => context.row({ node: f, provider: source.provider, view: source.view })), empty: 'No changes.' },
      diffBlock(patch, full),
    ],
  };
}

async function refDetail(source: Source & { node: RefNode }, context: DetailContext, key: string): Promise<TabDetail> {
  const { node } = source;
  const commits = await source.provider.getChildren(node);
  const meta: { label: string; value: string }[] = [];
  if (node.type === 'local') {
    const b = node.branch;
    meta.push({ label: 'Branch', value: `${b.name}${b.current ? ' (checked out)' : ''}` });
    if (b.upstream) meta.push({ label: 'Upstream', value: b.gone ? `${b.upstream.name} (deleted on the remote)` : `${b.upstream.name}: ${b.ahead} ahead, ${b.behind} behind` });
    else meta.push({ label: 'Upstream', value: 'none' });
    if (b.worktreePath) meta.push({ label: 'Worktree', value: b.worktreePath });
  } else if (node.type === 'remote') {
    meta.push({ label: 'Remote branch', value: node.name });
  } else {
    meta.push({ label: 'Tag', value: `${node.name}${node.tag.annotated ? '' : ' (lightweight)'}` });
    if (node.tag.annotated && node.tag.subject) meta.push({ label: 'Points at', value: node.sha.slice(0, 10) });
  }
  return {
    title: `Log of ${node.name}`,
    key,
    blocks: [
      { type: 'meta', items: meta },
      { type: 'rows', title: `${plural(commits.length, 'commit')}`, rows: commits.map((c) => context.row({ node: c, provider: source.provider, view: source.view })), empty: 'No commits.' },
    ],
  };
}

async function remoteDetail(source: Source & { node: RemoteNode }, context: DetailContext, key: string): Promise<TabDetail> {
  const { remote, branches } = source.node;
  return {
    title: remote.name,
    key,
    blocks: [
      { type: 'meta', items: [{ label: 'Remote', value: remote.name }, { label: 'URL', value: remote.url }] },
      { type: 'rows', title: plural(branches.length, 'branch', 'branches'), rows: branches.map((b) => context.row({ node: b, provider: source.provider, view: source.view })), empty: 'No branches fetched yet.' },
    ],
  };
}

function worktreeDetail(node: WorktreeNode, key: string): TabDetail {
  const w = node.worktree;
  const items = [
    { label: 'Folder', value: w.path },
    { label: 'Checked out', value: w.branch ?? (w.sha ? `detached at ${w.sha.slice(0, 10)}` : 'nothing') },
  ];
  const notes = [w.main && 'the main worktree', w.current && 'this window', w.locked && 'locked', w.prunable && 'missing: its folder is gone, Prune removes it'].filter(Boolean);
  if (notes.length) items.push({ label: 'Notes', value: notes.join(', ') });
  return { title: path.basename(w.path), key, blocks: [{ type: 'meta', items }] };
}

async function changeDetail(node: ChangeNode, context: DetailContext, full: boolean, key: string): Promise<TabDetail> {
  const { entry } = node;
  const { git } = context;
  const blocks: DetailBlock[] = [{ type: 'meta', items: [{ label: 'File', value: entry.oldPath ? `${entry.oldPath} → ${entry.path}` : entry.path }, { label: 'Status', value: describeStatus(entry) }] }];
  if (isConflict(entry)) {
    const text = await git.run([...DIFF_ARGS, 'diff', '--no-color', '--no-ext-diff', '--', entry.path]);
    blocks.push({ type: 'text', text: 'Resolve the conflict in the file or the merge editor, then stage it.' });
    blocks.push(diffBlock(text, full, 'Conflict'));
  } else {
    const [staged, unstaged] = await Promise.all([
      isStaged(entry) ? workingDiff(git, entry, true) : Promise.resolve(''),
      isUnstaged(entry) ? workingDiff(git, entry, false) : Promise.resolve(''),
    ]);
    if (unstaged) blocks.push(diffBlock(unstaged, full, 'Unstaged changes'));
    if (staged) blocks.push(diffBlock(staged, full, 'Staged changes'));
  }
  return { title: entry.path, key, blocks };
}

function plural(count: number, word: string, many = `${word}s`): string {
  return `${count} ${count === 1 ? word : many}`;
}
