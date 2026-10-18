// The buttons the SlothGit tab draws itself: on a row when the mouse is over it, and in each list's header. Their
// `when` clauses use VS Code's syntax (evaluated by tab/when.ts) over SlothGit's context keys plus:
//   view: the kind of list a row comes from (files, branches, commits, reflog, stashes)
//   item: the row's contextValue (as the old tree items had)
// Titles and icons come from the commands in package.json. Right-click menus are VS Code's own (webview/context).

export type ListId = 'files' | 'local' | 'remotes' | 'tags' | 'worktrees' | 'commits' | 'reflog' | 'stash';

export interface ActionEntry {
  command: string;
  when?: string;
}

export interface HeaderEntry extends ActionEntry {
  /** Shown as an icon button; the others are in the ⋯ menu. */
  primary?: boolean;
}

const SIMPLE = 'config.slothgit.simpleMode';

/** Buttons on a row while the mouse is over it, in order. */
export const ROW_ACTIONS: ActionEntry[] = [
  // Files panel
  { command: 'slothgit.file.openMergeEditor', when: 'view == files && item =~ /:conflict/' },
  { command: 'slothgit.file.open', when: 'view == files && item =~ /^change(?!.*:deleted)/' },
  { command: 'slothgit.file.discard', when: 'view == files && item =~ /^change(?!.*:conflict)/' },
  { command: 'slothgit.file.stage', when: 'view == files && item =~ /:(unstaged|conflict)/' },
  { command: 'slothgit.file.unstage', when: 'view == files && item =~ /:staged/' },
  // Commits, reflog, branch logs
  { command: 'slothgit.commit.openChanges', when: 'item =~ /^(commit|reflog|refcommit)/' },
  { command: 'slothgit.commit.moveUp', when: `view == commits && item =~ /^commit(?!.*:(head|other))/ && !${SIMPLE}` },
  { command: 'slothgit.commit.moveDown', when: `view == commits && item =~ /^commit(?!.*:(root|other))/ && !${SIMPLE}` },
  { command: 'slothgit.todo.moveUp', when: 'view == commits && item =~ /^todo:(pick|reword|edit|squash|fixup|drop)/' },
  { command: 'slothgit.todo.moveDown', when: 'view == commits && item =~ /^todo:(pick|reword|edit|squash|fixup|drop)/' },
  // Stashes
  { command: 'slothgit.stash.apply', when: 'item == stash' },
  { command: 'slothgit.stash.pop', when: 'item == stash' },
  // Branches, remotes, tags, worktrees
  { command: 'slothgit.branch.checkout', when: 'view == branches && item =~ /^(branch:local(?!.*:current)|branch:remote|tag$)/' },
  { command: 'slothgit.branch.fastForward', when: 'view == branches && item =~ /^branch:local.*:upstream/' },
  { command: 'slothgit.branch.push', when: 'view == branches && item =~ /^branch:local/' },
  { command: 'slothgit.remote.fetch', when: 'view == branches && item == remote' },
  { command: 'slothgit.worktree.open', when: 'view == branches && item =~ /^worktree(?!.*:current)/' },
  // Files of a commit or stash
  { command: 'slothgit.file.openAtRevision', when: 'item =~ /^file:/' },
];

/** Each list's header buttons, in order. */
export const LIST_ACTIONS: Record<ListId, HeaderEntry[]> = {
  files: [
    { command: 'slothgit.files.stageAll', primary: true },
    { command: 'slothgit.files.unstageAll', primary: true },
    { command: 'slothgit.stash.push', primary: true },
    { command: 'slothgit.files.discardAll' },
    { command: 'slothgit.stash.pushStaged' },
    { command: 'slothgit.stash.pushKeepIndex', when: `!${SIMPLE}` },
  ],
  local: [
    { command: 'slothgit.commit.clearCherryPick', primary: true, when: 'slothgit.hasCherryPicks' },
    { command: 'slothgit.branch.create', primary: true },
    { command: 'slothgit.fetchAll', primary: true },
    { command: 'slothgit.branch.checkoutPrevious' },
    { command: 'slothgit.branch.deleteMerged' },
    { command: 'slothgit.branch.sort' },
  ],
  remotes: [
    { command: 'slothgit.remote.add', primary: true },
    { command: 'slothgit.fetchAll', primary: true },
  ],
  tags: [{ command: 'slothgit.tag.create', primary: true }],
  worktrees: [
    { command: 'slothgit.worktree.add', primary: true, when: `!${SIMPLE}` },
    { command: 'slothgit.worktree.prune', when: `!${SIMPLE}` },
  ],
  commits: [
    { command: 'slothgit.commit.pasteCherryPick', primary: true, when: 'slothgit.hasCherryPicks' },
    { command: 'slothgit.commit.clearCherryPick', primary: true, when: 'slothgit.hasCherryPicks' },
    { command: 'slothgit.patch.menu', primary: true, when: 'slothgit.hasPatch' },
    { command: 'slothgit.commit.clearBase', primary: true, when: 'slothgit.hasRebaseBase' },
    { command: 'slothgit.bisect.reset', primary: true, when: 'slothgit.bisecting' },
    { command: 'slothgit.undo', primary: true },
    { command: 'slothgit.redo', primary: true },
    { command: 'slothgit.commits.search', primary: true, when: '!slothgit.commitsSearching' },
    { command: 'slothgit.commits.clearSearch', primary: true, when: 'slothgit.commitsSearching' },
    { command: 'slothgit.commits.clearFilter', primary: true, when: 'slothgit.commitsFiltered' },
    { command: 'slothgit.commits.showCurrentBranch', primary: true, when: 'slothgit.commitsAllBranches' },
    { command: 'slothgit.graph.open', primary: true },
    { command: 'slothgit.commits.filterByFile', when: '!slothgit.commitsFiltered' },
    { command: 'slothgit.commits.showAllBranches', when: '!slothgit.commitsAllBranches' },
    { command: 'slothgit.compare.clear', when: 'slothgit.hasCompareBase' },
  ],
  reflog: [
    { command: 'slothgit.undo', primary: true },
    { command: 'slothgit.redo', primary: true },
  ],
  stash: [
    { command: 'slothgit.stash.push', primary: true },
    { command: 'slothgit.stash.pushStaged' },
    { command: 'slothgit.stash.pushKeepIndex', when: `!${SIMPLE}` },
  ],
};

/** The Status panel's header: things about SlothGit as a whole. */
export const STATUS_ACTIONS: HeaderEntry[] = [
  { command: 'slothgit.refresh', primary: true },
  { command: 'slothgit.switchRepository', primary: true, when: 'slothgit.multipleRepos' },
  { command: 'slothgit.learn' },
  { command: 'slothgit.simpleMode.on', when: `!${SIMPLE}` },
  { command: 'slothgit.simpleMode.off', when: SIMPLE },
];

/** The Status panel's own buttons: Fetch, Pull, Push, and Continue / Skip / Abort or Stop Bisecting when needed. */
export const STATUS_BUTTONS = [
  'slothgit.sync.fetch',
  'slothgit.sync.pull',
  'slothgit.sync.push',
  'slothgit.operation.continue',
  'slothgit.operation.skip',
  'slothgit.operation.abort',
  'slothgit.bisect.reset',
];

/** Every command a button may run; the tab ignores anything else a page posts. */
export const BUTTON_COMMANDS = new Set([
  ...ROW_ACTIONS.map((a) => a.command),
  ...Object.values(LIST_ACTIONS).flatMap((list) => list.map((a) => a.command)),
  ...STATUS_ACTIONS.map((a) => a.command),
  ...STATUS_BUTTONS,
]);
