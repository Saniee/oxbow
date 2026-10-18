import * as fs from 'node:fs';
import * as path from 'node:path';
import {
  currentBranch,
  EMPTY_TREE,
  getBisectState,
  getCommitMessage,
  getOperation,
  getReflog,
  rebaseTodoPath,
  hasStagedChanges,
  headSha,
  isDirty,
  parsePerson,
  type FileChange,
  type Operation,
  type Person,
} from './queries';
import { Git } from './runner';
import { REWORD_MESSAGES_FILE, transformTodo, type RebasePlan, type TodoAction } from './todo';
import { parseFileDiff, type FileDiff } from './patch';
import { planUndo, REDO_MARKER, UNDO_MARKER, type UndoAction, type UndoPlan } from './undo';
import { discard, hasHead, stage, unstage, type StatusEntry } from './workingTree';

const ROOT = '--root';

/**
 * Builds the shell command git runs as its (sequence) editor. Git executes editors through `sh`
 * (bundled with Git for Windows too), so POSIX quoting and forward slashes work everywhere.
 * ELECTRON_RUN_AS_NODE lets VS Code's own executable act as node; plain node ignores it.
 */
export function buildEditorCommand(runtime: string, script: string): string {
  const quote = (p: string) => {
    const normalized = process.platform === 'win32' ? p.replace(/\\/g, '/') : p;
    return `'${normalized.replace(/'/g, `'\\''`)}'`;
  };
  return `ELECTRON_RUN_AS_NODE=1 ${quote(runtime)} ${quote(script)}`;
}

export class UserError extends Error {}

export type ResetMode = 'soft' | 'mixed' | 'hard';

export interface OpsOptions {
  /**
   * Whether rebases also move branches stacked on the rewritten commits (`--update-refs`). 'gitConfig' leaves it
   * to git's rebase.updateRefs setting.
   */
  updateStackedBranches?: 'gitConfig' | 'always' | 'never';
}

/** History-rewriting and housekeeping operations. Callers are expected to refresh UI afterwards. */
export class GitOps {
  constructor(
    readonly git: Git,
    private readonly editorCommand: string,
    private readonly options: OpsOptions = {},
  ) {}

  private updateRefsFlags(): string[] {
    const mode = this.options.updateStackedBranches ?? 'gitConfig';
    return mode === 'always' ? ['--update-refs'] : mode === 'never' ? ['--no-update-refs'] : [];
  }

  // --- interactive rebase -------------------------------------------------

  /** `sha~depth`, or ROOT when that commit does not exist (rebase from the root commit). */
  private async ancestor(sha: string, depth: number): Promise<string> {
    const result = await this.git.exec(['rev-parse', '--verify', '-q', `${sha}~${depth}^{commit}`], {
      allowedExitCodes: [1],
    });
    return result.exitCode === 0 ? result.stdout.trim() : ROOT;
  }

  private async hasParent(sha: string): Promise<boolean> {
    return (await this.ancestor(sha, 1)) !== ROOT;
  }

  /** For commits on a linear history, the oldest of them. */
  private async oldest(shas: string[]): Promise<string> {
    if (shas.length === 1) return shas[0];
    return (await this.git.run(['merge-base', '--octopus', ...shas])).trim();
  }

  private async hasMerges(base: string): Promise<boolean> {
    const range = base === ROOT ? 'HEAD' : `${base}..HEAD`;
    return parseInt(await this.git.run(['rev-list', '--merges', '--count', range]), 10) > 0;
  }

  /** Merge commits can be reworded, edited and dropped, but not squashed, fixed up or moved. */
  private async assertNotMerges(shas: string[]): Promise<void> {
    if (!shas.length) return;
    const merges = (await this.git.run(['rev-list', '--no-walk', '--merges', ...shas])).trim();
    if (merges) {
      throw new UserError("A merge commit can't be squashed, fixed up, moved or amended with staged changes. It can be reworded, edited or dropped.");
    }
  }

  /**
   * Runs `git rebase --interactive` with `plan` applied to the todo list. When the commits to rewrite include
   * merges, --rebase-merges recreates them; merges whose parents changed are merged again, so conflicts
   * resolved in them come back.
   */
  private async rebase(
    base: string,
    plan: RebasePlan,
    options: { message?: string; autosquash?: boolean; onto?: string } = {},
  ): Promise<void> {
    const [, hasMerges] = await Promise.all([
      this.assertNotMerges([
        ...Object.entries(plan.actions ?? {}).flatMap(([sha, action]) => (action === 'squash' || action === 'fixup' ? [sha] : [])),
        ...(plan.move ? [plan.move.sha] : []),
        ...(plan.fixupInto ? [plan.fixupInto.target] : []),
      ]),
      this.hasMerges(base),
    ]);
    const merges = hasMerges ? ['--rebase-merges'] : [];
    const env: Record<string, string> = {
      GIT_SEQUENCE_EDITOR: this.editorCommand,
      GIT_EDITOR: this.editorCommand,
      SLOTHGIT_REBASE_PLAN: JSON.stringify(plan),
    };
    if (options.message !== undefined) env.SLOTHGIT_COMMIT_MESSAGE = options.message;
    const autosquash = options.autosquash ? '--autosquash' : '--no-autosquash';
    const range = options.onto ? ['--onto', options.onto, base] : [base];
    await this.git.exec(['rebase', '--interactive', '--autostash', autosquash, ...merges, ...this.updateRefsFlags(), ...range], { env });
  }

  /**
   * Stops an interactive rebase at `sha`, runs `fn` (which typically amends HEAD), then continues.
   * If `fn` fails the rebase is aborted, so history is left untouched.
   */
  private async atCommit(sha: string, fn: () => Promise<void>): Promise<void> {
    await this.rebase(await this.ancestor(sha, 1), { actions: { [sha]: 'edit' } });
    if ((await getOperation(this.git)) !== 'rebase' || (await headSha(this.git)) !== sha) {
      throw new Error(`The rebase did not stop at ${sha.slice(0, 10)} as expected.`);
    }
    try {
      await fn();
    } catch (error) {
      await this.git.exec(['rebase', '--abort']).catch(() => undefined);
      throw error;
    }
    await this.continueOperation('rebase');
  }

  /** Amends only the commit metadata of `sha` (never staged changes). */
  private async amendMetadata(sha: string, args: string[]): Promise<void> {
    const amend = ['commit', '--amend', '--only', '--no-edit', '--allow-empty', ...args];
    if (sha === (await headSha(this.git)) && !(await getOperation(this.git))) {
      await this.git.exec(amend);
      return;
    }
    await this.atCommit(sha, () => this.git.exec(amend).then(() => undefined));
  }

  private static actions(shas: string[], action: TodoAction): Record<string, TodoAction> {
    return Object.fromEntries(shas.map((sha) => [sha, action]));
  }

  async drop(shas: string[]): Promise<void> {
    const base = await this.ancestor(await this.oldest(shas), 1);
    await this.rebase(base, { actions: GitOps.actions(shas, 'drop') });
  }

  /** Squash or fixup each commit into the commit before it. */
  async squash(shas: string[], action: 'squash' | 'fixup'): Promise<void> {
    const oldest = await this.oldest(shas);
    if (!(await this.hasParent(oldest))) throw new UserError('The first commit has no previous commit to squash into.');
    const [parentMerges, base] = await Promise.all([
      this.git.run(['rev-list', '--no-walk', '--merges', ...shas.map((sha) => `${sha}^`)]),
      this.ancestor(oldest, 2),
    ]);
    if (parentMerges.trim()) throw new UserError("The commit before it is a merge commit, which can't be squashed into.");
    await this.rebase(base, { actions: GitOps.actions(shas, action) });
  }

  async reword(sha: string, message: string): Promise<void> {
    if (sha === (await headSha(this.git))) {
      await this.git.exec(['commit', '--amend', '--only', '--allow-empty', '-m', message]);
      return;
    }
    await this.rebase(await this.ancestor(sha, 1), { actions: { [sha]: 'reword' } }, { message });
  }

  async edit(sha: string): Promise<void> {
    await this.rebase(await this.ancestor(sha, 1), { actions: { [sha]: 'edit' } });
  }

  async move(sha: string, direction: 'up' | 'down'): Promise<void> {
    if (direction === 'up') {
      if (sha === (await headSha(this.git))) throw new UserError('This is already the newest commit.');
      await this.rebase(await this.ancestor(sha, 1), { move: { sha, direction } });
    } else {
      if (!(await this.hasParent(sha))) throw new UserError('This is already the oldest commit.');
      await this.rebase(await this.ancestor(sha, 2), { move: { sha, direction } });
    }
  }

  /** Fold the staged changes into `sha` (lazygit's "amend commit with staged changes"). */
  async amendStaged(sha: string): Promise<void> {
    if (!(await hasStagedChanges(this.git))) throw new UserError('There are no staged changes to amend.');
    if (sha === (await headSha(this.git))) {
      await this.git.exec(['commit', '--amend', '--no-edit']);
      return;
    }
    const base = await this.ancestor(sha, 1);
    await this.assertNotMerges([sha]);
    // Temporary commit; it is folded into the target and its message discarded, so skip hooks.
    await this.git.exec(['commit', '--no-verify', '-m', `slothgit: amend ${sha}`]);
    const temp = await headSha(this.git);
    try {
      await this.rebase(base, { fixupInto: { sha: temp, target: sha } });
    } catch (error) {
      // If the rebase never started, put the changes back in the index.
      if (!(await getOperation(this.git))) await this.git.exec(['reset', '--soft', 'HEAD~1']);
      throw error;
    }
  }

  /** Creates a "fixup! <subject>" commit for `sha` from the staged changes (lazygit's F). */
  async createFixupCommit(sha: string): Promise<void> {
    if (!(await hasStagedChanges(this.git))) throw new UserError('Stage the changes for the fixup commit first.');
    await this.git.exec(['commit', `--fixup=${sha}`]);
  }

  /** Squashes every "fixup!"/"squash!" commit above `sha` into its target (lazygit's S). */
  async autosquash(sha: string): Promise<void> {
    await this.rebase(await this.ancestor(sha, 1), {}, { autosquash: true });
  }

  async resetAuthor(sha: string): Promise<void> {
    await this.amendMetadata(sha, ['--reset-author']);
  }

  /** `author` in the form "Name <email>". */
  async setAuthor(sha: string, author: string): Promise<void> {
    await this.amendMetadata(sha, [`--author=${author}`]);
  }

  async addCoAuthor(sha: string, coAuthor: string): Promise<void> {
    await this.amendMetadata(sha, ['--trailer', `Co-authored-by: ${coAuthor}`]);
  }

  /** Removes the Co-authored-by trailers naming any of `people` from the message of `sha`. */
  async removeCoAuthors(sha: string, people: Person[]): Promise<void> {
    const named = (value: string) => {
      const p = parsePerson(value);
      return people.some((q) => q.name === p.name && q.email === p.email);
    };
    const lines = (await getCommitMessage(this.git, sha)).split('\n');
    const kept = lines.filter((line) => {
      const m = /^co-authored-by:\s*(.*)$/i.exec(line.trim());
      return !(m && named(m[1]));
    });
    if (kept.length === lines.length) throw new UserError('The commit has no such co-author.');
    await this.reword(sha, kept.join('\n').replace(/\s+$/, ''));
  }

  /**
   * Removes the changes to `changes` from commit `sha`. With `keep`, the removed changes are then
   * applied to the index and working tree instead of being discarded (lazygit's "move patch out into index").
   */
  async removeFilesFromCommit(sha: string, changes: FileChange[], keep: boolean): Promise<string | void> {
    let patchFile: string | undefined;
    if (keep) {
      const parent = (await this.hasParent(sha)) ? `${sha}^` : EMPTY_TREE;
      const paths = changes.flatMap((c) => (c.oldPath ? [c.oldPath, c.path] : [c.path]));
      const patch = await this.git.run(['diff', '--binary', parent, sha, '--', ...paths]);
      patchFile = await this.git.gitDirPath('slothgit-uncommit.patch');
      fs.writeFileSync(patchFile, patch);
    }

    await this.atCommit(sha, async () => {
      for (const change of changes) {
        if (change.status === 'A' || change.status === 'C' || change.status === 'R') {
          await this.git.exec(['rm', '-q', '--', change.path]);
        }
        const restore = change.status === 'R' ? change.oldPath : change.status === 'A' || change.status === 'C' ? undefined : change.path;
        if (restore) await this.git.exec(['checkout', 'HEAD^', '--', restore]);
      }
      await this.git.exec(['commit', '--amend', '--no-edit', '--allow-empty', '--no-verify']);
    });

    if (patchFile) return this.applyToWorkingTree(patchFile);
  }

  /** Applies changes taken out of a commit to the index and working tree; if that fails, says where they are kept. */
  private async applyToWorkingTree(patchFile: string): Promise<string | void> {
    try {
      await this.git.exec(['apply', '--3way', patchFile]);
      fs.rmSync(patchFile, { force: true });
    } catch (error) {
      const reason = error instanceof Error ? error.message.split('\n')[0] : String(error);
      return `The changes were removed from the commit but could not be applied to your working tree (${reason}). They are saved in ${patchFile}.`;
    }
  }

  /**
   * The changes `removal` (from removalPatch, possibly for several files) takes out of commit `sha`, as a forward
   * patch with blob ids, so it can be applied elsewhere with a 3-way merge.
   */
  async patchContent(sha: string, removal: string): Promise<string> {
    const index = await this.git.gitDirPath('slothgit-patch-index');
    const removalFile = await this.git.gitDirPath('slothgit-patch.patch');
    fs.writeFileSync(removalFile, removal);
    // A scratch index, so the real one and the working tree are untouched.
    const env = { GIT_INDEX_FILE: index };
    try {
      await this.git.exec(['read-tree', sha], { env });
      await this.git.exec(['apply', '--cached', '--recount', removalFile], { env });
      const without = (await this.git.run(['write-tree'], { env })).trim();
      return await this.git.run(['diff', '--binary', without, sha]);
    } finally {
      fs.rmSync(index, { force: true });
      fs.rmSync(removalFile, { force: true });
    }
  }

  /**
   * Moves the changes `removal` takes out of commit `sha` into commit `target`, older or newer (lazygit's "move
   * patch into selected commit"). If they don't apply to `target`, history is put back as it was.
   */
  async movePatchToCommit(sha: string, removal: string, target: string): Promise<void> {
    if (sha === target) throw new UserError('Pick a different commit to move the changes into.');
    // Found again by its distance from HEAD after the first rewrite, which changes it if it is newer than `sha`.
    const distance = parseInt(await this.git.run(['rev-list', '--first-parent', '--count', `${target}..HEAD`]), 10);
    const resolved = await this.git.exec(['rev-parse', '--verify', '-q', `HEAD~${distance}`], { allowedExitCodes: [1] });
    if (resolved.stdout.trim() !== target) throw new UserError('The target commit must be on the current branch, not on a merged-in branch.');
    await this.assertNotMerges([target]);
    const forward = await this.patchContent(sha, removal);
    const original = await headSha(this.git);
    await this.removePatchFromCommit(sha, removal, 'discard');

    const patchFile = await this.git.gitDirPath('slothgit-move.patch');
    fs.writeFileSync(patchFile, forward);
    try {
      const newTarget = (await this.git.run(['rev-parse', `HEAD~${distance}`])).trim();
      await this.atCommit(newTarget, async () => {
        await this.git.exec(['apply', '--index', '--3way', patchFile]);
        await this.git.exec(['commit', '--amend', '--no-edit', '--allow-empty', '--no-verify']);
      });
    } catch (error) {
      // Undo the first step; --keep leaves uncommitted changes alone.
      const back = await this.git.exec(['reset', '--keep', original], { allowedExitCodes: [128] });
      const reason = error instanceof Error ? error.message.split('\n')[0] : String(error);
      throw new UserError(
        back.exitCode === 0
          ? `The changes don't apply to ${target.slice(0, 7)}, so nothing was changed (${reason}).`
          : `The changes don't apply to ${target.slice(0, 7)} (${reason}). They were taken out of ${sha.slice(0, 7)}; the original history is ${original.slice(0, 10)}.`,
      );
    } finally {
      fs.rmSync(patchFile, { force: true });
    }
  }

  /** Applies the changes `removal` takes out of commit `sha` to the working tree, or reverts them there with `reverse`. */
  async applyPatch(sha: string, removal: string, reverse: boolean): Promise<void> {
    const patchFile = await this.git.gitDirPath('slothgit-apply.patch');
    fs.writeFileSync(patchFile, await this.patchContent(sha, removal));
    const flags = reverse ? ['--reverse'] : [];
    try {
      // Unstaged when it applies as is; otherwise a 3-way merge, which stages what it merges and marks conflicts.
      const plain = await this.git.exec(['apply', ...flags, patchFile], { allowedExitCodes: [1] });
      if (plain.exitCode !== 0) await this.git.exec(['apply', '--3way', ...flags, patchFile]);
    } finally {
      fs.rmSync(patchFile, { force: true });
    }
  }

  /** `git diff` of one file of commit `sha`, parsed for building partial patches (see patch.ts). */
  async commitFileDiff(sha: string, change: FileChange): Promise<FileDiff> {
    const parent = (await this.hasParent(sha)) ? `${sha}^` : EMPTY_TREE;
    const paths = change.oldPath ? [change.oldPath, change.path] : [change.path];
    const diff = await this.git.run([
      'diff', '--no-color', '--no-ext-diff', '-M', '-U3', '--src-prefix=a/', '--dst-prefix=b/', parent, sha, '--', ...paths,
    ]);
    return parseFileDiff(diff, change.path);
  }

  /**
   * Takes the changes in `patch` (from removalPatch) out of commit `sha` (lazygit's custom patch, at line level).
   * 'index' then applies them to the index and working tree; 'newCommit' commits them right after `sha`.
   */
  async removePatchFromCommit(
    sha: string,
    patch: string,
    target: 'discard' | 'index' | { newCommit: string },
  ): Promise<string | void> {
    const patchFile = await this.git.gitDirPath('slothgit-patch.patch');
    const restoreFile = await this.git.gitDirPath('slothgit-uncommit.patch');
    fs.writeFileSync(patchFile, patch);
    try {
      await this.atCommit(sha, async () => {
        await this.git.exec(['apply', '--index', '--recount', patchFile]);
        await this.git.exec(['commit', '--amend', '--no-edit', '--allow-empty', '--no-verify']);
        if (target === 'discard') return;
        // Exactly what was taken out, with blob ids, so it can be applied with a 3-way merge later.
        fs.writeFileSync(restoreFile, await this.git.run(['diff', '--binary', 'HEAD', sha]));
        if (target === 'index') return;
        await this.git.exec(['apply', '--index', restoreFile]);
        fs.rmSync(restoreFile, { force: true });
        await this.git.exec(['commit', '-m', target.newCommit]);
      });
    } finally {
      fs.rmSync(patchFile, { force: true });
    }
    if (target === 'index') return this.applyToWorkingTree(restoreFile);
  }

  /** Overwrites a file in the working tree and index with its version from `sha`. */
  async checkoutFileFrom(sha: string, file: string): Promise<void> {
    await this.git.exec(['checkout', sha, '--', file]);
  }

  // --- other commit operations ---------------------------------------------

  /** `shas` in the order they should be applied (oldest first). */
  /** Applies oldest first, whatever order the commits were copied in; newest first would conflict with itself. */
  async cherryPick(shas: string[]): Promise<void> {
    const ordered = shas.length > 1 ? (await this.git.run(['rev-list', '--no-walk=sorted', '--reverse', ...shas])).split(/\r?\n/).filter(Boolean) : shas;
    await this.git.exec(['cherry-pick', ...ordered]);
  }

  /** `mainline` is the parent (1-based) a merge commit is reverted against, the side whose changes are kept. */
  async revert(shas: string[], mainline?: number): Promise<void> {
    await this.git.exec(['revert', '--no-edit', ...(mainline ? ['-m', `${mainline}`] : []), ...shas]);
  }

  async reset(sha: string, mode: ResetMode): Promise<void> {
    await this.git.exec(['reset', `--${mode}`, sha]);
  }

  async checkout(sha: string): Promise<void> {
    await this.git.exec(['checkout', '--detach', sha]);
  }

  async createBranch(name: string, sha: string, checkout: boolean): Promise<void> {
    await this.git.exec(checkout ? ['checkout', '-b', name, sha] : ['branch', name, sha]);
  }

  // --- branches ---------------------------------------------------------------

  /**
   * For commits made on the wrong branch (lazygit's "move commits to new branch"): checks out a new branch `name`
   * with the current branch's unpushed commits, and puts the current branch back to its upstream. Uncommitted
   * changes come along. Returns the number of commits moved.
   */
  async moveCommitsToNewBranch(name: string): Promise<number> {
    const current = await currentBranch(this.git);
    if (!current) throw new UserError('HEAD is detached. Check out the branch the commits were made on.');
    const upstream = await this.git.exec(['rev-parse', '--verify', '-q', '@{upstream}'], { allowedExitCodes: [1, 128] });
    if (upstream.exitCode !== 0) {
      throw new UserError(`${current} has no upstream, so there is nothing to put it back to. Create a branch here and reset ${current} instead.`);
    }
    const count = parseInt(await this.git.run(['rev-list', '--count', '@{upstream}..HEAD']), 10);
    if (count === 0) throw new UserError(`${current} has no commits that are not on its upstream.`);
    await this.git.exec(['switch', '-c', name]);
    await this.git.exec(['branch', '-f', current, upstream.stdout.trim()]);
    return count;
  }

  async switchBranch(name: string): Promise<void> {
    await this.git.exec(['switch', name]);
  }

  async switchPrevious(): Promise<void> {
    await this.git.exec(['switch', '-']);
  }

  /** Checks out a local branch for `remoteBranch`, creating a tracking branch when none exists. Returns the local name. */
  async checkoutRemoteBranch(remoteBranch: string, localName: string): Promise<'switched' | 'created'> {
    const exists = await this.git.exec(['show-ref', '--verify', '--quiet', `refs/heads/${localName}`], {
      allowedExitCodes: [1],
    });
    if (exists.exitCode === 0) {
      await this.switchBranch(localName);
      return 'switched';
    }
    await this.git.exec(['switch', '-c', localName, '--track', remoteBranch]);
    return 'created';
  }

  async deleteBranch(name: string, force: boolean): Promise<void> {
    await this.git.exec(['branch', force ? '-D' : '-d', name]);
  }

  async renameBranch(oldName: string, newName: string): Promise<void> {
    await this.git.exec(['branch', '-m', oldName, newName]);
  }

  async setUpstream(branch: string, upstream: string | undefined): Promise<void> {
    await this.git.exec(
      upstream ? ['branch', `--set-upstream-to=${upstream}`, branch] : ['branch', '--unset-upstream', branch],
    );
  }

  async merge(ref: string, mode: 'default' | 'noFastForward' | 'squash'): Promise<void> {
    const flags = { default: ['--no-edit'], noFastForward: ['--no-ff', '--no-edit'], squash: ['--squash'] }[mode];
    await this.git.exec(['merge', ...flags, ref], { env: { GIT_EDITOR: ':' } });
  }

  /**
   * Rebase the current branch onto `target`; with `base`, only the commits after `base` are moved (`--onto`).
   * `interactive` stops before the first commit is applied, so the todo list can be changed first.
   */
  async rebaseOnto(target: string, base?: string, interactive = false): Promise<void> {
    if (interactive) {
      const count = parseInt(await this.git.run(['rev-list', '--count', `${base ?? target}..HEAD`]), 10);
      if (count === 0) throw new UserError('There are no commits to rebase: the current branch has no commits that are not on the target.');
      await this.rebase(base ?? target, { pauseFirst: true }, base ? { onto: target } : {});
      return;
    }
    const args = base ? ['--onto', target, base] : [target];
    await this.git.exec(['rebase', '--autostash', ...this.updateRefsFlags(), ...args], { env: { GIT_EDITOR: ':' } });
  }

  /**
   * Number of commits a rebase would rewrite that are already on the current branch's upstream,
   * i.e. commits in `exclude..HEAD` that are also reachable from @{upstream}.
   */
  async countPushedCommitsAfter(exclude: string): Promise<number> {
    const mergeBase = await this.git.exec(['merge-base', 'HEAD', '@{upstream}'], { allowedExitCodes: [1, 128] });
    if (mergeBase.exitCode !== 0) return 0;
    const count = await this.git.run(['rev-list', '--count', `${exclude}..${mergeBase.stdout.trim()}`]);
    return parseInt(count, 10);
  }

  /** Fast-forward the current branch to its upstream, without fetching. */
  async fastForwardCurrent(): Promise<void> {
    await this.git.exec(['merge', '--ff-only', '@{upstream}']);
  }

  // --- remotes -------------------------------------------------------------------

  async addRemote(name: string, url: string): Promise<void> {
    await this.git.exec(['remote', 'add', name, url]);
  }

  async removeRemote(name: string): Promise<void> {
    await this.git.exec(['remote', 'remove', name]);
  }

  async renameRemote(oldName: string, newName: string): Promise<void> {
    await this.git.exec(['remote', 'rename', oldName, newName]);
  }

  async setRemoteUrl(name: string, url: string): Promise<void> {
    await this.git.exec(['remote', 'set-url', name, url]);
  }

  // --- tags ------------------------------------------------------------------------

  /** Creates an annotated tag when `message` is non-empty, otherwise a lightweight one. */
  async createTag(name: string, ref: string, message: string): Promise<void> {
    await this.git.exec(message ? ['tag', '-a', name, '-m', message, ref] : ['tag', name, ref]);
  }

  async deleteTag(name: string): Promise<void> {
    await this.git.exec(['tag', '-d', name]);
  }

  // --- undo / redo ---------------------------------------------------------------

  async planUndo(): Promise<UndoPlan> {
    const entries = await getReflog(this.git, 1000);
    return planUndo(entries.map((e) => ({ sha: e.sha, subject: e.subject })));
  }

  /** Reverts (undo) or re-applies (redo) a reflog action. Uncommitted changes are carried over. */
  async applyUndo(action: UndoAction, direction: 'undo' | 'redo'): Promise<void> {
    const env = { GIT_REFLOG_ACTION: direction === 'undo' ? UNDO_MARKER : REDO_MARKER };
    const target = direction === 'undo' ? action.from : action.to;
    if (action.kind === 'checkout') {
      await this.git.exec(['checkout', target, '--'], { env });
      return;
    }
    if (action.kind === 'commit') {
      // Moves the branch only: undoing leaves the commit's changes staged, redoing commits them again.
      await this.git.exec(['reset', '--soft', target], { env });
      return;
    }
    const stash = await isDirty(this.git);
    if (stash) await this.git.exec(['stash', 'push', '--include-untracked', '-m', `slothgit: autostash for ${direction}`]);
    try {
      await this.git.exec(['reset', '--hard', target], { env });
    } finally {
      if (stash) {
        const pop = await this.git.exec(['stash', 'pop', '--index'], { allowedExitCodes: [1] });
        if (pop.exitCode !== 0) {
          throw new UserError(
            `Your uncommitted changes conflict with the ${direction}. They were kept in the stash (stash@{0}); resolve the conflicts in Source Control.`,
          );
        }
      }
    }
  }

  // --- bisect -------------------------------------------------------------------------

  /** Marks `sha` as bad/good/skip, starting a bisect first if needed. Returns git's summary line. */
  async bisectMark(sha: string, mark: 'bad' | 'good' | 'skip'): Promise<string> {
    const state = await getBisectState(this.git);
    if (!state) await this.git.exec(['bisect', 'start']);
    const term = mark === 'skip' ? 'skip' : state ? state.terms[mark] : mark;
    const output = await this.git.run(['bisect', term, sha]);
    return output
      .split(/\r?\n/)
      .find((line) => /Bisecting:|is the first|There are only|bisect run|needs testing|Some good revs/.test(line))
      ?.trim() ?? output.split(/\r?\n/)[0];
  }

  async bisectReset(): Promise<void> {
    await this.git.exec(['bisect', 'reset']);
  }

  // --- worktrees ----------------------------------------------------------------------

  /** Adds a worktree at `dir`, for `branch`, or for a new branch `newBranch` starting at `start`. */
  async addWorktree(dir: string, options: { branch: string } | { newBranch: string; start: string }): Promise<void> {
    const args =
      'branch' in options ? [dir, options.branch] : ['-b', options.newBranch, dir, options.start];
    await this.git.exec(['worktree', 'add', ...args]);
  }

  async removeWorktree(dir: string, force: boolean): Promise<void> {
    await this.git.exec(['worktree', 'remove', ...(force ? ['--force'] : []), dir]);
  }

  async pruneWorktrees(): Promise<void> {
    await this.git.exec(['worktree', 'prune']);
  }

  /** Local branches fully merged into HEAD, excluding the current one. */
  async mergedBranches(): Promise<string[]> {
    const output = await this.git.run(['branch', '--merged', 'HEAD', '--format=%(HEAD) %(refname:short)']);
    return output
      .split(/\r?\n/)
      .filter((line) => line.startsWith('  '))
      .map((line) => line.slice(2).trim())
      .filter(Boolean);
  }

  // --- in-progress operations ----------------------------------------------

  /**
   * Sets a step of the paused rebase to reword. The message is saved next to the todo list and written by
   * rebaseEditor.js when git reaches the step.
   */
  async rewordTodo(sha: string, message: string): Promise<void> {
    const full = (await this.git.run(['rev-parse', '--verify', `${sha}^{commit}`])).trim();
    const todoFile = await rebaseTodoPath(this.git);
    if (!fs.existsSync(todoFile)) throw new UserError('No interactive rebase is paused.');
    fs.writeFileSync(todoFile, transformTodo(fs.readFileSync(todoFile, 'utf8'), { actions: { [full]: 'reword' } }));
    const savedFile = path.join(path.dirname(todoFile), REWORD_MESSAGES_FILE);
    const saved = fs.existsSync(savedFile) ? (JSON.parse(fs.readFileSync(savedFile, 'utf8')) as Record<string, string>) : {};
    saved[full] = message;
    fs.writeFileSync(savedFile, JSON.stringify(saved));
  }

  /**
   * Rebases run rebaseEditor.js, which applies saved reword messages and otherwise keeps git's prepared message.
   * For the others ':' tells git not to launch an editor and keep the prepared message.
   */
  private continueEnv(operation: Operation): Record<string, string> {
    return { GIT_EDITOR: operation === 'rebase' ? this.editorCommand : ':' };
  }

  async continueOperation(operation: Operation): Promise<void> {
    await this.git.exec([operation, '--continue'], { env: this.continueEnv(operation) });
  }

  async abortOperation(operation: Operation): Promise<void> {
    await this.git.exec([operation, '--abort']);
  }

  async skipOperation(operation: Operation): Promise<void> {
    await this.git.exec([operation, '--skip'], { env: this.continueEnv(operation) });
  }

  // --- stashes ---------------------------------------------------------------

  async stashPush(mode: 'all' | 'staged' | 'keepIndex', message: string): Promise<void> {
    const flags = { all: ['--include-untracked'], staged: ['--staged'], keepIndex: ['--keep-index', '--include-untracked'] }[mode];
    await this.git.exec(['stash', 'push', ...flags, ...(message ? ['-m', message] : [])]);
  }

  async stashApply(ref: string, pop: boolean): Promise<void> {
    await this.git.exec(['stash', pop ? 'pop' : 'apply', ref]);
  }

  async stashDrop(ref: string): Promise<void> {
    await this.git.exec(['stash', 'drop', ref]);
  }

  async stashRename(ref: string, sha: string, message: string): Promise<void> {
    await this.git.exec(['stash', 'drop', ref]);
    await this.git.exec(['stash', 'store', '-m', message, sha]);
  }

  async stashBranch(name: string, ref: string): Promise<void> {
    await this.git.exec(['stash', 'branch', name, ref]);
  }

  // --- working tree (the Files panel) -------------------------------------------

  stage(entries: StatusEntry[]): Promise<void> {
    return stage(this.git, entries);
  }

  unstage(entries: StatusEntry[]): Promise<void> {
    return unstage(this.git, entries);
  }

  discard(entries: StatusEntry[], unstagedOnly = false): Promise<void> {
    return discard(this.git, entries, unstagedOnly);
  }

  async stageAll(): Promise<void> {
    await this.git.exec(['add', '-A']);
  }

  async unstageAll(): Promise<void> {
    if (await hasHead(this.git)) await this.git.exec(['reset', '-q']);
    else await this.git.exec(['rm', '-q', '-r', '--cached', '.']);
  }

  /** Every uncommitted change, untracked files included, back to HEAD (lazygit's "discard all"). */
  async discardAll(): Promise<void> {
    if (await hasHead(this.git)) await this.git.exec(['reset', '-q', '--hard']);
    else await this.git.exec(['rm', '-q', '-r', '-f', '--cached', '.'], { allowedExitCodes: [128] });
    await this.git.exec(['clean', '-f', '-d', '-q']);
  }
}
