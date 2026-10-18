// Runs inside a real VS Code (see scripts/smoke.mjs). Opens the scratch repository the launcher created,
// walks every SlothGit list in several repository states, and checks every declared command is registered.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import type { SlothGitTestApi } from '../../src/extension';
import type { Node } from '../../src/views/nodes';

function log(message: string): void {
  // Ends up in the launcher's console.
  console.log(`[smoke] ${message}`);
}

async function walk(provider: vscode.TreeDataProvider<Node>, depth: number): Promise<Map<string, number>> {
  const counts = new Map<string, number>();
  const visit = async (node: Node | undefined, level: number): Promise<void> => {
    const children = ((await provider.getChildren(node)) ?? []) as Node[];
    for (const child of children) {
      const item = await provider.getTreeItem(child);
      assert.ok(item.label !== undefined || item.resourceUri, `tree item without a label: ${JSON.stringify(child).slice(0, 200)}`);
      const key = item.contextValue ?? child.kind;
      counts.set(key, (counts.get(key) ?? 0) + 1);
      const expandable = item.collapsibleState !== vscode.TreeItemCollapsibleState.None;
      if (expandable && level < depth) await visit(child, level + 1);
    }
  };
  await visit(undefined, 0);
  return counts;
}

async function snapshot(api: SlothGitTestApi, label: string): Promise<Record<string, Map<string, number>>> {
  await api.refreshAll(true);
  const result: Record<string, Map<string, number>> = {};
  for (const [name, provider] of Object.entries(api.providers)) {
    // Commits/branches: expand down to files. Others: one level of children.
    result[name] = await walk(provider, name === 'branches' ? 3 : 1);
  }
  const summary = Object.entries(result)
    .map(([name, counts]) => `${name}: ${[...counts].map(([k, v]) => `${k}=${v}`).join(' ')}`)
    .join('\n    ');
  log(`${label}\n    ${summary}`);
  return result;
}

const has = (counts: Map<string, number>, key: string) => [...counts.keys()].some((k) => k === key || k.startsWith(`${key}:`));

export async function run(): Promise<void> {
  const repo = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  assert.ok(repo, 'the launcher should open the scratch repository');
  const sh = (...args: string[]) => execFileSync('git', args, { cwd: repo, encoding: 'utf8' }).trim();

  const ext = vscode.extensions.all.find((e) => e.packageJSON.name === 'slothgit');
  assert.ok(ext, 'SlothGit extension not found');
  const api = (await ext.activate()) as SlothGitTestApi | undefined;
  assert.ok(api, 'activate() returned nothing; is the built-in Git extension enabled?');

  // Every command in package.json is registered.
  const registered = new Set(await vscode.commands.getCommands(true));
  const declared: string[] = ext.packageJSON.contributes.commands.map((c: { command: string }) => c.command);
  const missing = declared.filter((id) => !registered.has(id));
  assert.deepEqual(missing, [], `declared but not registered: ${missing.join(', ')}`);
  log(`${declared.length} commands registered`);

  // Wait for the Git extension to discover the repository, then open the tab: the lists load while it is visible.
  const gitApi = (await vscode.extensions.getExtension('vscode.git')!.activate()).getAPI(1);
  for (let i = 0; i < 100 && gitApi.repositories.length === 0; i++) await new Promise((r) => setTimeout(r, 100));
  assert.ok(gitApi.repositories.length > 0, 'the Git extension found the scratch repository');
  await vscode.commands.executeCommand('slothgit.open');
  for (let i = 0; i < 100 && !api.tab.renderedLists.commits; i++) await new Promise((r) => setTimeout(r, 100));
  assert.ok(api.tab.renderedLists.commits, 'the tab drew the Commits list');

  // 1. Normal state.
  let s = await snapshot(api, 'normal');
  assert.ok(has(s.commits, 'commit'), 'commits view lists commits');
  assert.ok(has(s.commits, 'file'), 'commit files are listed');
  for (const kind of ['section:local', 'section:remotes', 'section:tags', 'section:worktrees', 'branch:local', 'branch:remote', 'tag', 'remote', 'worktree']) {
    assert.ok(has(s.branches, kind), `branches view has ${kind}`);
  }
  assert.ok(has(s.stashes, 'stash') && has(s.stashes, 'file:stash'), 'stash view lists stashes and their files');
  assert.ok(has(s.reflog, 'reflog'), 'reflog view lists entries');
  log(`tab drew ${JSON.stringify(api.tab.renderedLists)}`);

  // 2. File filter via the command the Explorer menu uses.
  // f2.txt is only touched by "commit 2".
  await vscode.commands.executeCommand('slothgit.commits.showFileHistory', vscode.Uri.file(path.join(repo, 'f2.txt')));
  s = await snapshot(api, 'filtered by f2.txt');
  const shown = [...s.commits].filter(([k]) => k.startsWith('commit')).reduce((sum, [, v]) => sum + v, 0);
  assert.equal(shown, 1, 'the filter limits the log to commits touching the file');
  await vscode.commands.executeCommand('slothgit.commits.clearFilter');

  // 3. Paused interactive rebase: the todo list shows up in the Commits view.
  const second = sh('rev-list', '--reverse', 'HEAD').split('\n')[1];
  sh('-c', 'sequence.editor=sed -i -e "1s/^pick/edit/"', 'rebase', '-i', `${second}~1`);
  s = await snapshot(api, 'rebase paused');
  assert.ok(has(s.commits, 'todo'), 'pending rebase steps are listed');
  sh('rebase', '--abort');

  // 4. Bisect: marks appear on commits.
  const shas = sh('rev-list', 'HEAD').split('\n');
  sh('bisect', 'start');
  sh('bisect', 'bad', shas[0]);
  sh('bisect', 'good', shas[shas.length - 1]);
  s = await snapshot(api, 'bisecting');
  assert.ok((s.commits.get('commit') ?? 0) + (s.commits.get('commit:root') ?? 0) >= 2, 'bisect keeps showing the branch');
  sh('bisect', 'reset');

  // 5. Non-interactive commands run without errors.
  await vscode.commands.executeCommand('slothgit.refresh');
  await vscode.commands.executeCommand('slothgit.compare.clear');
  await vscode.commands.executeCommand('slothgit.commit.clearCherryPick');
  log('commands executed');

  // Opening diffs: the contents are loaded before the editor appears, so it is never blank.
  const [head] = (await api.providers.commits.getChildren()) as Node[];
  const files = (await api.providers.commits.getChildren(head)) as Node[];
  await vscode.commands.executeCommand('slothgit.openFileDiff', files[0]);
  const tab = vscode.window.tabGroups.activeTabGroup.activeTab;
  assert.ok(tab?.input instanceof vscode.TabInputTextDiff, 'a diff editor opened');
  const loaded = vscode.workspace.textDocuments.find((d) => d.uri.toString() === (tab.input as vscode.TabInputTextDiff).modified.toString());
  assert.ok(loaded && loaded.getText().length > 0, 'the diff content was loaded');
  await vscode.commands.executeCommand('slothgit.commit.openChanges', head);
  log(`diffs open with content (${tab.label})`);
  await vscode.commands.executeCommand('workbench.action.closeAllEditors');

  // 6. The Sync view's Fetch and Pull buttons.
  await vscode.commands.executeCommand('slothgit.sync.fetch');
  await vscode.commands.executeCommand('slothgit.sync.pull');
  assert.equal(sh('log', '-1', '--format=%s'), 'upstream change', 'Pull fast-forwarded main');
  log('fetch / pull buttons work');

  fs.writeFileSync(path.join(repo, '.smoke-ok'), 'ok');
}
