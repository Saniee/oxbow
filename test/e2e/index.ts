// Runs inside VS Code (see scripts/e2e.mjs): every scenario in scenarios.ts against the playground, restoring
// the playground between scenarios, and writes a JSON report for the launcher.
import * as fs from 'node:fs';
import * as vscode from 'vscode';
import type { SlothGitTestApi } from '../../src/extension';
import { Context } from './harness';
import scenarios from './scenarios';

const TIMEOUT_MS = 60_000;

function timeout(ms: number): Promise<never> {
  return new Promise((_, reject) => setTimeout(() => reject(new Error(`timed out after ${ms} ms (waiting for a prompt?)`)), ms));
}

export async function run(): Promise<void> {
  const playground = process.env.SLOTHGIT_E2E_PLAYGROUND!;
  const report = process.env.SLOTHGIT_E2E_REPORT!;
  const filters = (process.env.SLOTHGIT_E2E_FILTER ?? '').split(/\s+/).filter(Boolean);
  const results: { name: string; ok: boolean; ms: number; error?: string }[] = [];

  const ext = vscode.extensions.all.find((e) => e.packageJSON.name === 'slothgit')!;
  const api = (await ext.activate()) as SlothGitTestApi;
  const gitApi = (await vscode.extensions.getExtension('vscode.git')!.activate()).getAPI(1);
  for (let i = 0; i < 100 && gitApi.repositories.length === 0; i++) await new Promise((r) => setTimeout(r, 100));
  // The lists only load while the SlothGit tab is visible.
  await vscode.commands.executeCommand('slothgit.open');
  for (let i = 0; i < 100 && !api.tab.renderedLists.commits; i++) await new Promise((r) => setTimeout(r, 100));

  const t = new Context(api, playground, gitApi.repositories[0]);
  t.takeSnapshot();
  await t.restore();

  const selected = scenarios.filter((s) => !filters.length || filters.some((f) => s.name.includes(f)));
  // Shows what is going on in the test window, which otherwise just flickers as the playground is reset.
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 1000);
  status.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
  status.show();
  let failures = 0;
  for (const [i, s] of selected.entries()) {
    const progress = `${i + 1}/${selected.length}`;
    status.text = `$(beaker) SlothGit e2e ${progress}${failures ? ` · ${failures} failed` : ''}: ${s.name}`;
    const start = Date.now();
    try {
      await Promise.race([s.run(t), timeout(TIMEOUT_MS)]);
      results.push({ name: s.name, ok: true, ms: Date.now() - start });
      console.log(`[e2e] ok    ${progress} ${s.name}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      results.push({ name: s.name, ok: false, ms: Date.now() - start, error: message });
      failures++;
      console.log(`[e2e] FAIL  ${progress} ${s.name}`);
      for (const line of message.split('\n')) console.log(`[e2e]         ${line}`);
    }
    try {
      await t.restore();
    } catch (error) {
      console.log(`[e2e] Restoring the playground failed, stopping: ${error}`);
      break;
    }
    fs.writeFileSync(report, JSON.stringify({ results }, null, 2));
  }
  fs.writeFileSync(report, JSON.stringify({ results }, null, 2));
}
