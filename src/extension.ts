import * as vscode from 'vscode';
import { setContext } from './contextKeys';
import { registerBranchCommands } from './branchCommands';
import { registerCommands } from './commands';
import { registerFileCommands } from './fileCommands';
import { CommandLog } from './git/commandLog';
import { getBisectState, getOperation, type Operation } from './git/queries';
import { registerHistoryCommands } from './historyCommands';
import { RepoManager } from './repoManager';
import { createRunner } from './runner';
import type { ListId } from './tab/actions';
import { SlothGitTab } from './tab/tabPanel';
import type { GitExtension } from './types/git';
import { BranchesProvider } from './views/branches';
import { CherryPickClipboard } from './views/cherryPickClipboard';
import { CustomPatch } from './views/customPatch';
import { FilesProvider } from './views/files';
import { GraphPanel, registerGraphNodes } from './views/graphPanel';
import { initLearn, learn, type LearnTopic } from './learn';
import type { Node } from './views/nodes';
import { CommitsProvider, ReflogProvider, StashProvider } from './views/providers';
import type { ListView } from './views/webviewNodes';

const OPERATION_LABELS: Record<Operation, string> = {
  rebase: 'Rebasing',
  'cherry-pick': 'Cherry-picking',
  revert: 'Reverting',
  merge: 'Merging',
};

/** Returned from activate() for the smoke and end-to-end tests; not a stable API. */
export interface SlothGitTestApi {
  providers: Record<'files' | 'commits' | 'branches' | 'stashes' | 'reflog', vscode.TreeDataProvider<Node>>;
  /** The tab's lists, with their selection, as commands see them. */
  views: Record<'files' | 'commits' | 'branches' | 'stashes' | 'reflog', ListView>;
  tab: SlothGitTab;
  log: CommandLog;
  /** Rows the open Commit Graph page last drew. */
  graphRenderedRows(): number | undefined;
  refreshAll(force: boolean): Promise<void>;
  /** Runs the CodeGit settings migration again, as on a first start. */
  migrateSettings(): Promise<void>;
}

export async function activate(context: vscode.ExtensionContext): Promise<SlothGitTestApi | undefined> {
  const output = vscode.window.createOutputChannel('SlothGit', { log: true });
  context.subscriptions.push(output);
  initLearn(context.extension.id);
  void migrateSettings(context, output);

  const extension = vscode.extensions.getExtension<GitExtension>('vscode.git');
  const gitExtension = extension && (extension.isActive ? extension.exports : await extension.activate());
  if (!gitExtension?.enabled) {
    output.warn('The built-in Git extension is disabled; SlothGit is inactive.');
    return;
  }

  const repos = new RepoManager(gitExtension.getAPI(1), context.asAbsolutePath('dist/rebaseEditor.js'));
  const clipboard = new CherryPickClipboard();
  const patch = new CustomPatch();
  const log = new CommandLog(() => repos.repository?.rootUri.fsPath);
  const files = new FilesProvider(repos, output);
  const commits = new CommitsProvider(repos, output, clipboard);
  const stashes = new StashProvider(repos, output);
  const reflog = new ReflogProvider(repos, output);
  const branches = new BranchesProvider(repos, output, clipboard);

  const statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
  statusItem.command = 'slothgit.operation.menu';
  statusItem.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');

  async function updateOperation(): Promise<void> {
    const git = repos.git();
    const [operation, bisect] = git
      ? await Promise.all([getOperation(git).catch(() => undefined), getBisectState(git).catch(() => undefined)])
      : [undefined, undefined];
    setContext('slothgit.operation', operation ?? '');
    if (operation) {
      statusItem.text = `$(git-merge) ${OPERATION_LABELS[operation]}…`;
      statusItem.tooltip = 'SlothGit: continue, skip or abort';
      statusItem.command = 'slothgit.operation.menu';
      statusItem.show();
    } else if (bisect) {
      statusItem.text = '$(bug) Bisecting…';
      statusItem.tooltip = 'SlothGit: mark the checked-out commit as good or bad in the Commits panel';
      statusItem.command = { command: 'slothgit.open', title: 'Open SlothGit', arguments: ['commits'] };
      statusItem.show();
    } else {
      statusItem.hide();
    }
  }

  async function refreshAll(force: boolean): Promise<void> {
    await repos.markLoaded();
    await Promise.all([
      files.reload(force),
      commits.reload(force),
      branches.reload(force),
      stashes.reload(force),
      reflog.reload(force),
      tab.reload(),
      updateOperation(),
    ]);
  }

  const runner = createRunner(repos, output, refreshAll, log);
  const tab = new SlothGitTab(context, repos, { files, branches, commits, reflog, stashes }, runner, log);
  const views = {
    files: tab.view('files'),
    commits: tab.view('commits'),
    branches: tab.combinedView(['local', 'remotes', 'tags', 'worktrees']),
    stashes: tab.view('stash'),
    reflog: tab.view('reflog'),
  };

  // The Activity Bar's SlothGit icon: opening its (empty) sidebar view opens the tab.
  const launcher = vscode.window.createTreeView('slothgit.launcher', { treeDataProvider: { getChildren: () => [], getTreeItem: (i: never) => i } });

  context.subscriptions.push(
    repos,
    clipboard,
    patch,
    log,
    tab,
    statusItem,
    launcher,
    launcher.onDidChangeVisibility((e) => e.visible && tab.show()),
    files.attach(tab),
    commits.attach(tab),
    branches.attach(tab),
    stashes.attach(tab),
    reflog.attach(tab),
    registerGraphNodes(),
    repos.onDidChange(() => refreshAll(false)),
    // The Files panel shows every edit; the other lists only change with refs and history (onDidChange).
    repos.onDidChangeWorkingTree(() => {
      void files.reload();
      void tab.workingTreeChanged();
    }),
    clipboard.onDidChange(() => {
      commits.redraw();
      branches.redraw();
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('slothgit')) void refreshAll(true);
    }),
    ...registerCommands({ repos, commits, commitsView: views.commits, stashView: views.stashes, clipboard, patch, runner, refreshAll }),
    ...registerBranchCommands({ repos, branches, branchesView: views.branches, commits, runner }),
    ...registerHistoryCommands({ repos, commits, commitsView: views.commits, runner, refreshAll }),
    ...registerFileCommands({ repos, files, filesView: views.files, runner }),
    vscode.commands.registerCommand('slothgit.open', (list?: ListId) => tab.show(typeof list === 'string' ? list : undefined)),
    vscode.commands.registerCommand('slothgit.graph.open', () => GraphPanel.show(context, repos)),
    vscode.commands.registerCommand('slothgit.simpleMode.on', () => setSimpleMode(true)),
    vscode.commands.registerCommand('slothgit.simpleMode.off', () => setSimpleMode(false)),
    vscode.commands.registerCommand('slothgit.learn', (topic?: LearnTopic) => learn(topic)),
  );

  void refreshAll(true);
  return {
    providers: { files, commits, branches, stashes, reflog },
    views,
    tab,
    log,
    graphRenderedRows: () => GraphPanel.renderedRows(),
    refreshAll,
    migrateSettings: async () => {
      await context.globalState.update(SETTINGS_MIGRATED_KEY, undefined);
      await context.workspaceState.update(SETTINGS_MIGRATED_KEY, undefined);
      await migrateSettings(context, output);
    },
  };
}

/** Simple Mode hides advanced actions from the menus (package.json gates them on config.slothgit.simpleMode). */
function setSimpleMode(on: boolean): Thenable<void> {
  return vscode.workspace.getConfiguration('slothgit').update('simpleMode', on, vscode.ConfigurationTarget.Global);
}

const SETTINGS_MIGRATED_KEY = 'codegitSettingsMigrated';

type SettingsLevel = 'globalValue' | 'workspaceValue' | 'workspaceFolderValue';

/**
 * The extension was called CodeGit before 1.0.0. Copies `codegit.*` settings to `slothgit.*`, unless the new one is
 * already set at that level: user settings once per install, workspace and folder settings once per workspace (each
 * workspace has its own). The old ones stay: VS Code only writes settings some extension declares, so they can't be
 * removed from here (they show up greyed out as unknown).
 *
 * Remove after 2.0.0, together with the legacy undo markers in git/undo.ts.
 */
export async function migrateSettings(context: vscode.ExtensionContext, output: vscode.LogOutputChannel): Promise<void> {
  const keys = Object.keys(context.extension.packageJSON.contributes?.configuration?.properties ?? {}).map((k) =>
    k.replace(/^slothgit\./, ''),
  );
  const copy = async (scope: vscode.WorkspaceFolder | undefined, level: SettingsLevel, target: vscode.ConfigurationTarget) => {
    const old = vscode.workspace.getConfiguration('codegit', scope);
    const current = vscode.workspace.getConfiguration('slothgit', scope);
    for (const key of keys) {
      const value = old.inspect(key)?.[level];
      if (value === undefined || current.inspect(key)?.[level] !== undefined) continue;
      try {
        await current.update(key, value, target);
        output.info(`Copied setting codegit.${key} to slothgit.${key}`);
      } catch (e) {
        output.warn(`Could not copy setting codegit.${key}: ${e}`);
      }
    }
  };
  if (!context.globalState.get(SETTINGS_MIGRATED_KEY)) {
    await copy(undefined, 'globalValue', vscode.ConfigurationTarget.Global);
    await context.globalState.update(SETTINGS_MIGRATED_KEY, true);
  }
  const folders = vscode.workspace.workspaceFolders;
  if (folders?.length && !context.workspaceState.get(SETTINGS_MIGRATED_KEY)) {
    await copy(undefined, 'workspaceValue', vscode.ConfigurationTarget.Workspace);
    for (const folder of folders) await copy(folder, 'workspaceFolderValue', vscode.ConfigurationTarget.WorkspaceFolder);
    await context.workspaceState.update(SETTINGS_MIGRATED_KEY, true);
  }
}

export function deactivate(): void {}
