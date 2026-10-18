// The Status panel's Pull and Push dropdowns. The ⌄ buttons show them as a quick pick (tab/tabPanel.ts); the same
// entries are the native right-click menu (package.json webview/context, kept in step by test/contributions.test.ts).
export type SyncMenu = 'pull' | 'push';

export interface SyncMenuItem {
  command: string;
  /** Second line in the quick pick. The label is the command's title from package.json. */
  detail?: string;
  /** Hidden in Simple Mode. */
  advanced?: boolean;
}

export const SYNC_MENUS: Record<SyncMenu, SyncMenuItem[]> = {
  pull: [
    { command: 'slothgit.sync.pull', detail: "Bring the upstream branch's new commits into this branch" },
    { command: 'slothgit.sync.pullRebase', detail: 'Put your new commits on top of the upstream branch’s, without a merge commit' },
    { command: 'slothgit.sync.fetch', detail: 'Download what is new on the remote without changing your branch' },
    { command: 'slothgit.fetchAll', detail: 'Fetch every remote' },
  ],
  push: [
    { command: 'slothgit.sync.push', detail: 'Upload your new commits to the upstream branch' },
    {
      command: 'slothgit.sync.forcePush',
      detail: 'Replace the remote branch with yours, after a rewrite. Refuses if someone else pushed since your last fetch',
    },
    { command: 'slothgit.sync.pushTo', detail: 'Push to a branch with another name, or on another remote' },
    { command: 'slothgit.sync.setUpstream', detail: 'Choose which remote branch this one pulls from and pushes to', advanced: true },
  ],
};
