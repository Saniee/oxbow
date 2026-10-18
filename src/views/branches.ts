import * as vscode from 'vscode';
import * as path from 'node:path';
import {
  getLocalBranches,
  getLog,
  getRemoteBranches,
  getRemotes,
  getTags,
  getWorktrees,
  type BranchSort,
  type LocalBranch,
} from '../git/queries';
import type { Git } from '../git/runner';
import type { RepoManager } from '../repoManager';
import { BaseProvider } from './baseProvider';
import type { CherryPickClipboard } from './cherryPickClipboard';
import type { CommitNode, Node, RefNode, RemoteNode, SectionNode, WorktreeNode } from './nodes';
import { coAuthorLines, commitFiles, commitLabel, config } from './providers';

const SECTIONS: Record<SectionNode['section'], { label: string; icon: string; expanded: boolean }> = {
  local: { label: 'Local Branches', icon: 'git-branch', expanded: true },
  remotes: { label: 'Remotes', icon: 'cloud', expanded: false },
  tags: { label: 'Tags', icon: 'tag', expanded: false },
  worktrees: { label: 'Worktrees', icon: 'repo', expanded: false },
};

/** lazygit's branches panel: local branches, remotes with their branches, and tags. */
export class BranchesProvider extends BaseProvider<SectionNode> {
  constructor(
    repos: RepoManager,
    output: vscode.LogOutputChannel,
    private readonly clipboard: CherryPickClipboard,
  ) {
    super(repos, output);
  }

  /** Local branches from the last load, for pickers and name lookups. */
  get localBranches(): LocalBranch[] {
    const local = this.roots.find((s) => s.section === 'local');
    return (local?.children ?? []).flatMap((n) => (n.kind === 'ref' && n.type === 'local' ? [n.branch] : []));
  }

  /** Remote branches from the last load. */
  get remoteBranchNames(): string[] {
    const remotes = this.roots.find((s) => s.section === 'remotes');
    return (remotes?.children ?? []).flatMap((r) => (r.kind === 'remote' ? r.branches.map((b) => b.name) : []));
  }

  get remoteNames(): string[] {
    const remotes = this.roots.find((s) => s.section === 'remotes');
    return (remotes?.children ?? []).flatMap((r) => (r.kind === 'remote' ? [r.remote.name] : []));
  }

  protected async load(git: Git): Promise<SectionNode[]> {
    const remoteList = getRemotes(git);
    const [local, remotes, remoteBranches, tags, worktrees] = await Promise.all([
      getLocalBranches(git, config<BranchSort>('branchSort', 'recency')),
      remoteList,
      getRemoteBranches(git, remoteList),
      getTags(git),
      getWorktrees(git),
    ]);

    const localNodes: RefNode[] = local.map((branch) => ({
      kind: 'ref',
      type: 'local',
      name: branch.name,
      rev: branch.name,
      sha: branch.sha,
      branch,
    }));
    const remoteNodes: RemoteNode[] = remotes.map((remote) => ({
      kind: 'remote',
      remote,
      branches: remoteBranches
        .filter((b) => b.remote === remote.name)
        .map((branch) => ({ kind: 'ref', type: 'remote', name: branch.name, rev: branch.name, sha: branch.sha, branch })),
    }));
    const tagNodes: RefNode[] = tags.map((tag) => ({
      kind: 'ref',
      type: 'tag',
      name: tag.name,
      rev: `refs/tags/${tag.name}`,
      sha: tag.sha,
      tag,
    }));

    return [
      { kind: 'section', section: 'local', children: localNodes },
      { kind: 'section', section: 'remotes', children: remoteNodes },
      { kind: 'section', section: 'tags', children: tagNodes },
      // Worktrees are an advanced feature: Simple Mode leaves the section out.
      ...(config('simpleMode', false)
        ? []
        : [
            {
              kind: 'section' as const,
              section: 'worktrees' as const,
              children: worktrees.map((worktree): WorktreeNode => ({ kind: 'worktree', worktree })),
            },
          ]),
    ];
  }

  protected keyOf(roots: SectionNode[]): string {
    return JSON.stringify(roots);
  }

  protected item(node: Node): vscode.TreeItem {
    switch (node.kind) {
      case 'section':
        return sectionItem(node);
      case 'remote':
        return remoteItem(node);
      case 'ref':
        return refItem(node);
      case 'commit':
        return commitItem(node, this.clipboard.has(node.sha));
      case 'worktree':
        return worktreeItem(node);
      default:
        throw new Error(`unexpected node ${node.kind}`);
    }
  }

  protected async children(git: Git, node: Node): Promise<Node[]> {
    switch (node.kind) {
      case 'section':
        return node.children;
      case 'remote':
        return node.branches;
      case 'ref': {
        const commits = await getLog(git, config('commitLimit', 300), node.rev);
        return commits.map(
          (c, index): CommitNode => ({ kind: 'commit', source: 'ref', parentRev: node.rev, index, ...c }),
        );
      }
      case 'commit':
        return commitFiles(git, node);
      default:
        return [];
    }
  }
}

function sectionItem(node: SectionNode): vscode.TreeItem {
  const { label, icon, expanded } = SECTIONS[node.section];
  const state = expanded ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.Collapsed;
  const item = new vscode.TreeItem(label, state);
  item.id = `section:${node.section}`;
  item.description = `${node.children.length}`;
  item.iconPath = new vscode.ThemeIcon(icon);
  item.contextValue = `section:${node.section}`;
  return item;
}

function remoteItem(node: RemoteNode): vscode.TreeItem {
  const item = new vscode.TreeItem(node.remote.name, vscode.TreeItemCollapsibleState.Collapsed);
  item.id = `remote:${node.remote.name}`;
  item.description = node.remote.url;
  item.tooltip = `${node.remote.name}\n${node.remote.url}\n${node.branches.length} branches`;
  item.iconPath = new vscode.ThemeIcon('cloud');
  item.contextValue = 'remote';
  return item;
}

function refItem(node: RefNode): vscode.TreeItem {
  const item = new vscode.TreeItem(node.name, vscode.TreeItemCollapsibleState.Collapsed);
  item.id = `${node.type}:${node.name}`;
  const tooltip = new vscode.MarkdownString(undefined, true);

  if (node.type === 'local') {
    const b = node.branch;
    const status: string[] = [];
    if (b.gone) status.push('upstream gone');
    else if (b.upstream) {
      if (b.ahead) status.push(`↑${b.ahead}`);
      if (b.behind) status.push(`↓${b.behind}`);
      if (!b.ahead && !b.behind) status.push('✓');
    }
    item.description = [status.join(' '), b.upstream?.name, b.relDate].filter(Boolean).join(' · ');
    if (b.worktreePath) item.description = `worktree · ${item.description}`;
    item.iconPath = b.current
      ? new vscode.ThemeIcon('check', new vscode.ThemeColor('slothgit.currentBranchIcon'))
      : b.worktreePath
        ? new vscode.ThemeIcon('repo', new vscode.ThemeColor('slothgit.worktreeBranchIcon'))
        : new vscode.ThemeIcon('git-branch', b.gone ? new vscode.ThemeColor('slothgit.goneIcon') : undefined);
    tooltip.appendMarkdown(`**${b.name}**${b.current ? ' (current)' : ''}\n\n`);
    tooltip.appendMarkdown(
      b.upstream
        ? `Tracking \`${b.upstream.name}\`${b.gone ? ' (deleted on remote)' : `: ${b.ahead} ahead, ${b.behind} behind`}\n\n`
        : 'No upstream\n\n',
    );
    if (b.worktreePath) tooltip.appendMarkdown(`Checked out in the worktree \`${b.worktreePath}\`\n\n`);
    tooltip.appendText(`${b.sha.slice(0, 8)} ${b.subject} (${b.relDate})`);
    const flags = [
      b.current && 'current',
      b.upstream && !b.gone && 'upstream',
      b.upstream && 'tracking',
      b.worktreePath && 'worktree',
    ].filter(Boolean);
    item.contextValue = ['branch:local', ...flags].join(':');
  } else if (node.type === 'remote') {
    const b = node.branch;
    item.label = b.branch;
    item.id = `remote-branch:${b.name}`;
    item.description = b.relDate;
    item.iconPath = new vscode.ThemeIcon('git-branch');
    tooltip.appendMarkdown(`**${b.name}**\n\n`);
    tooltip.appendText(`${b.sha.slice(0, 8)} ${b.subject} (${b.relDate})`);
    item.contextValue = 'branch:remote';
  } else {
    const t = node.tag;
    item.description = `${t.sha.slice(0, 7)} · ${t.relDate}`;
    item.iconPath = new vscode.ThemeIcon('tag');
    tooltip.appendMarkdown(`**${t.name}**${t.annotated ? '' : ' (lightweight)'}\n\n`);
    tooltip.appendText(`${t.sha.slice(0, 8)} ${t.subject} (${t.relDate})`);
    item.contextValue = 'tag';
  }
  item.tooltip = tooltip;
  return item;
}

function commitItem(node: CommitNode, copied: boolean): vscode.TreeItem {
  const item = new vscode.TreeItem(commitLabel(node), vscode.TreeItemCollapsibleState.Collapsed);
  item.id = `ref:${node.parentRev}:${node.sha}`;
  item.description = node.relDate;
  item.tooltip = [node.sha, `${node.author}, ${node.relDate}`, ...coAuthorLines(node), '', node.subject].join('\n');
  // Copied for cherry-pick: marked as in the Commits view, since the copy is usually made from another branch.
  item.iconPath = copied
    ? new vscode.ThemeIcon('clippy', new vscode.ThemeColor('slothgit.cherryPickedIcon'))
    : new vscode.ThemeIcon('git-commit');
  if (copied) {
    item.tooltip += '\n\nCopied for cherry-pick';
    item.description = `copied · ${item.description}`;
  }
  item.contextValue = copied ? 'refcommit:copied' : 'refcommit';
  return item;
}

function worktreeItem(node: WorktreeNode): vscode.TreeItem {
  const w = node.worktree;
  const item = new vscode.TreeItem(path.basename(w.path), vscode.TreeItemCollapsibleState.None);
  item.id = `worktree:${w.path}`;
  const what = w.branch ?? (w.sha ? `detached at ${w.sha.slice(0, 7)}` : '');
  item.description = [what, w.main && 'main', w.current && 'current', w.locked && 'locked', w.prunable && 'missing']
    .filter(Boolean)
    .join(' · ');
  item.tooltip = `${w.path}\n${what}`;
  item.iconPath = w.current
    ? new vscode.ThemeIcon('check', new vscode.ThemeColor('slothgit.currentBranchIcon'))
    : new vscode.ThemeIcon('repo', w.prunable ? new vscode.ThemeColor('slothgit.goneIcon') : undefined);
  const flags = [w.main && 'main', w.current && 'current', w.prunable && 'prunable'].filter(Boolean);
  item.contextValue = ['worktree', ...flags].join(':');
  return item;
}
