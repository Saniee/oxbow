import * as vscode from 'vscode';
import type { CommandLog } from './git/commandLog';
import { GitOps, UserError, type ResetMode } from './git/operations';
import { getOperation, type Operation } from './git/queries';
import { branchNameError } from './git/refName';
import { GitError } from './git/runner';
import type { RepoManager } from './repoManager';
import type { Repository } from './types/git';

const OPERATION_NAMES: Record<Operation, string> = {
  rebase: 'Rebase',
  'cherry-pick': 'Cherry-pick',
  revert: 'Revert',
  merge: 'Merge',
};

export interface Runner {
  /** Runs a git operation with progress, error reporting and a refresh afterwards. Resolves true on success. */
  run(title: string, fn: (ops: GitOps, repo: Repository) => Promise<string | void>): Promise<boolean>;
  confirm(message: string, action: string): Promise<boolean>;
  /** Asks for soft/mixed/hard and resets the current branch to `rev`. */
  resetTo(rev: string, label: string): Promise<void>;
  /** Input box for a new branch name. */
  askBranchName(title: string, value?: string): Promise<string | undefined>;
}

export function createRunner(
  repos: RepoManager,
  output: vscode.LogOutputChannel,
  refreshAll: (force: boolean) => Promise<void>,
  log?: CommandLog,
): Runner {
  let busy = false;

  async function run(title: string, fn: (ops: GitOps, repo: Repository) => Promise<string | void>): Promise<boolean> {
    const ops = repos.ops();
    const repo = repos.repository;
    if (!ops || !repo) {
      void vscode.window.showWarningMessage('SlothGit: no Git repository is open.');
      return false;
    }
    if (busy) {
      void vscode.window.showInformationMessage('SlothGit: another operation is still running.');
      return false;
    }
    busy = true;
    const entry = log?.start(title);
    let failure: { error: unknown } | undefined;
    let info: string | void = undefined;
    try {
      info = await withFeedback(title, () => fn(ops, repo));
    } catch (error) {
      failure = { error };
    } finally {
      busy = false;
      if (entry) log!.finish(entry, !failure);
      await repo.status().catch(() => undefined);
      await refreshAll(true);
    }
    if (!failure) {
      if (info) void vscode.window.showInformationMessage(info);
      else vscode.window.setStatusBarMessage(`$(check) ${doneMessage(title)}`, DONE_MESSAGE_MS);
    }
    // Reported only now that this operation no longer counts as running: the conflict prompt's
    // Continue/Abort buttons start a new one.
    if (failure) await reportError(ops, failure.error);
    return !failure;
  }

  async function reportError(ops: GitOps, error: unknown): Promise<void> {
    if (error instanceof UserError) {
      void vscode.window.showWarningMessage(error.message);
      return;
    }
    const message = errorMessage(error);
    output.error(error instanceof GitError ? `git ${error.args.join(' ')}\n${message}` : message);

    const operation = await getOperation(ops.git).catch(() => undefined);
    if (operation) {
      const choice = await vscode.window.showWarningMessage(
        `${OPERATION_NAMES[operation]} stopped. Resolve the conflicts in Source Control, stage them, then continue.`,
        'Continue',
        'Abort',
        'Open Source Control',
      );
      if (choice === 'Continue') await vscode.commands.executeCommand('slothgit.operation.continue');
      else if (choice === 'Abort') await vscode.commands.executeCommand('slothgit.operation.abort');
      else if (choice === 'Open Source Control') await vscode.commands.executeCommand('workbench.view.scm');
      return;
    }
    const choice = await vscode.window.showErrorMessage(`SlothGit: ${firstLines(message)}`, 'Show Log');
    if (choice) output.show();
  }

  async function confirm(message: string, action: string): Promise<boolean> {
    return (await vscode.window.showWarningMessage(message, { modal: true }, action)) === action;
  }

  async function resetTo(rev: string, label: string): Promise<void> {
    const modes: (vscode.QuickPickItem & { mode: ResetMode })[] = [
      { label: 'Mixed', description: 'Keep changes in the working tree, unstaged', mode: 'mixed' },
      { label: 'Soft', description: 'Keep changes staged', mode: 'soft' },
      { label: 'Hard', description: 'Discard all changes', mode: 'hard' },
    ];
    const picked = await vscode.window.showQuickPick(modes, { title: `Reset current branch to ${label}` });
    if (!picked) return;
    if (picked.mode === 'hard' && !(await confirm('Hard reset discards all uncommitted changes. Continue?', 'Reset'))) return;
    await run('Resetting…', (ops) => ops.reset(rev, picked.mode));
  }

  function askBranchName(title: string, value?: string): Promise<string | undefined> {
    return Promise.resolve(
      vscode.window.showInputBox({
        title,
        value,
        placeHolder: 'Branch name',
        validateInput: branchNameError,
      }),
    );
  }

  return { run, confirm, resetTo, askBranchName };
}

/** How long an operation runs before it also gets a notification, so slow pushes or rebases don't look stuck. */
const NOTIFY_AFTER_MS = 1500;
const DONE_MESSAGE_MS = 4000;

/**
 * Shows the running operation in the Source Control progress bar and as a spinner in the status bar,
 * plus a notification once it has taken longer than NOTIFY_AFTER_MS.
 */
function withFeedback<T>(title: string, fn: () => Promise<T>): Promise<T> {
  const task = Promise.resolve(vscode.window.withProgress({ location: vscode.ProgressLocation.SourceControl, title }, () =>
    vscode.window.withProgress({ location: vscode.ProgressLocation.Window, title: `SlothGit: ${title}` }, fn),
  ));
  const timer = setTimeout(() => {
    void vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: `SlothGit: ${title}` }, () =>
      task.catch(() => undefined),
    );
  }, NOTIFY_AFTER_MS);
  return task.finally(() => clearTimeout(timer));
}

/** "Pushing main…" → "Pushing main: done". */
export function doneMessage(title: string): string {
  return `SlothGit: ${title.replace(/…$/, '')}: done`;
}

/** Errors from the built-in Git extension carry git's output in `stderr`, with a generic `message`. */
export function errorMessage(error: unknown): string {
  const stderr = (error as { stderr?: unknown } | undefined)?.stderr;
  if (typeof stderr === 'string' && stderr.trim()) return stderr.trim();
  return error instanceof Error ? error.message : String(error);
}

function firstLines(message: string): string {
  const lines = message.split('\n').filter((l) => l.trim() && !l.startsWith('hint:'));
  return lines.slice(0, 3).join(' ');
}
