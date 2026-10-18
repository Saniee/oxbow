# Changelog

## 2.0.0

### Added
- **The SlothGit tab:** lazygit's layout in an editor tab, in VS Code's own look and used with the mouse. Status, Files, Branches (Local, Remotes, Tags, Worktrees), Commits (Commits, Reflog) and Stash panels on the left; the panel in use gets most of the room. On the right, whatever is selected: a commit's message, details, files and coloured patch; a branch's log (open one of its commits, then go back); a stash's patch; a file's staged and unstaged changes. Below it, the **Command Log** of each action and the git commands it ran. Open it from the SlothGit icon in the Activity Bar, the button on the Source Control title bar, or **SlothGit: Open SlothGit**.
- Right-clicking a row opens the same menus the views had. The common actions are also buttons on the row (on hover) and on each panel's title, with the rest in its **…** menu. Ctrl- and Shift-click select several rows; right-click actions then apply to all of them. Double-click opens a commit's changes, a file's diff, or checks out a branch.
- **Files panel:** every uncommitted change with git's two status letters. Stage, unstage and discard files (asking whether to keep the staged part when a file has both) or everything, open a file or its changes, resolve conflicts in the merge editor, and **commit or amend from the commit box**. Committing goes through VS Code's Git extension, so hooks, signing and its settings apply; with nothing staged it offers to stage everything.
- **Status panel:** what the Sync view showed (branch, upstream, ↑↓, Fetch, Pull and Push with their ⌄ menus), plus Continue / Skip / Abort while a rebase, merge, cherry-pick or revert is stopped, and Stop Bisecting.
- Diffs and files opened from the tab open beside it, so it stays in view.
- **Theme colours:** the tab and Commit Graph follow your colour theme, and SlothGit's own parts (commit icons, branch and tag labels, status letters, panels, graph lanes) have `slothgit.*` colour ids that themes and `workbench.colorCustomizations` can set. See the README.

### Removed
- The Sync, Branches, Commits, Reflog and Stashes views in the Source Control sidebar. Everything they did is in the SlothGit tab. SlothGit no longer hides VS Code's own **Graph** view on first start.

## 1.0.1

### Changed
- The package ships readable (unminified) code and no longer includes the internal `TODO.md`, to clear the Marketplace's content check.

## 1.0.0

### Added
- **Sync view** in the Source Control sidebar: the current branch, its upstream and ↑↓ counts, with Pull and Push buttons (Publish Branch when it has no upstream). Like the Commit button, each has a ⌄ with the other ways to do it, also on right-click: Pull (Rebase), Fetch, Fetch All Remotes; Force Push (with lease), Push to Another Branch, Set Upstream. Drag it above **Changes** to keep it next to the commit box.
- **Get Started with SlothGit** walkthrough on VS Code's Welcome page (also **Learn Git Concepts…** in each view's **…** menu): what each view shows, what the commit colours and ↑↓ counts mean, Undo and the Reflog, and plain-language guides to tidying commits, merge vs rebase (and interactive rebase), cherry-pick, stashes, worktrees and bisect.
- **Simple Mode** hides advanced actions (bisect, custom patches, worktrees, compare, resets, upstream settings, the Reflog view) from the menus. Toggle it with the eye button on the Source Control **Changes** title bar or from any SlothGit view's **…** menu; setting `slothgit.simpleMode`.
- The merge and rebase pickers explain each option in a second line, and link to the guide.

### Changed
- **Renamed from CodeGit to SlothGit** (the name was taken on the Marketplace). Commands, views and settings now start with `slothgit.`; your `codegit.*` settings are moved over the first time SlothGit starts, and Undo / Redo still understands history recorded under the old name.
- Branch name prompts check git's naming rules as you type (no spaces, `..`, characters such as `~`, `:` or `*`, a leading `-`, a trailing `.lock` and so on), instead of failing in git afterwards.
- Simple Mode also hides the ↑ / ↓ buttons on commit rows, like the Move Up / Down menu items.
- The Fetch / Pull / Push buttons moved from the **Changes** title bar to the Sync view, and the `showSyncButtons` setting is gone. Hide the Sync view (right-click its title) if you don't want it.
- Plainer menu labels: e.g. **Squash into Previous Commit (Combine Messages)**, **Fixup into Previous Commit (Discard This Message)**, **Add Staged Changes to This Commit**, **Check Out This Commit (Without a Branch)**, **Update from Remote (Fast-Forward)**, **Force Push (Safely, with Lease)**, **Pop Stash (Apply and Delete It)**, "Staged" instead of "Index", and the bisect actions say what Good and Bad mean.
- Commits copied for cherry-pick are marked in the Branches view too (blue clipboard icon, "copied"), and the Commits view title shows how many are copied. **Remove from Cherry-Pick** un-copies one; **Clear Copied Commits** is on the Branches title bar as well.
- Continue and Abort are on the Branches title bar while a rebase, merge, cherry-pick or revert is stopped.
- **Rebase Current Branch onto This** with a branch the current branch already contains now says there is nothing to rebase (and suggests merging the other way), instead of running a rebase that changes nothing.

### Fixed
- Copied commits are cherry-picked oldest first, whatever order they were copied in. Copying the newest first used to apply it first and conflict.
- A cherry-pick or revert of several commits is still recognised after the stopped commit was committed by hand (e.g. from the Commit button after resolving a conflict). Continue, Skip and Abort used to say nothing was in progress, while git still had the rest queued.
- Actions on the checked-out branch right after switching to it (before the Branches view had refreshed) could treat it as not checked out: **Fast-forward** failed with "refusing to fetch into branch", and checkout, delete, rebase and new worktrees could misjudge it. They now ask git which branch is checked out.

## 0.4.0

### Changed
- The Commits view keeps the cursor on the commit you acted on, as lazygit does: after Move Up / Down it follows the commit, after a reword, squash, drop or author change it stays in place, and after a cherry-pick, revert or new fixup commit it goes to the top.
- Shorter right-click menus. On a commit, the everyday rewrites stay at the top; Reword in Editor, Amend Author, fixup commits, Move Up / Down and Mark as Base move to **More Rewrite Actions**, and the bisect actions to a **Bisect** submenu. On a branch, Set / Unset Upstream move to an **Upstream** submenu. Copy Commit Message is gone from the menu (it is in **Copy…**). The Commit Graph's menu has the same layout.
- The Commits title bar keeps Undo, Redo, Search, Commit Graph and Refresh as buttons; **Filter Commits by File…** and **Show All Branches** move to its **…** menu. While a filter or all-branches mode is on, the button to turn it off stays visible.
- Rebase steps read like git's todo file, action first (`pick  Add search`), with a tooltip on what the action does. Right-click a step for **Pick**, **Reword…**, **Squash into Step Below**, **Fixup into Step Below**, **Edit (Stop Here)** or **Drop**, on several selected steps at once. Squash and fixup are hidden on the first step while nothing is applied, where there is nothing to meld into.
- **Skip** is on the Commits title bar while a rebase, cherry-pick or revert is stopped.
- Running Git actions show a spinner and their name in the status bar, and a notification if they take longer than a second and a half. When an action finishes without a message of its own, the status bar says so for a few seconds.

## 0.3.0

### Added
- **Stacked branches:** rewriting history can also move other local branches that point at the rewritten commits (`git rebase --update-refs`), so a branch stacked on another stays stacked. New setting `codegit.updateStackedBranches` (`gitConfig`, `always`, `never`); the default follows git's `rebase.updateRefs`. A paused rebase lists the branch updates as steps, and moving a commit across one moves it into that branch.
- **Custom patch:** collect changes from one commit, whole files (**Add to Custom Patch**) or selected lines in its diffs (**Add Selected Changes to Custom Patch**). The status bar shows the patch; click it to remove it from the commit, move it to the index, a new commit or another commit (older or newer), apply it to the working tree (also in reverse), show it or reset it. **Move Custom Patch into This Commit** on a commit does the same. If the patch doesn't apply to the target commit, history is left as it was.
- **Merge commits:** a merge can now be reworded, edited (including amending its author) or dropped. Dropping it takes the merged-in commits out of the branch's history. **Revert Commit** on a merge asks which parent's side to keep (`git revert -m`).
- **Move Unpushed Commits to New Branch…** on the current branch in the Branches view, for commits made on the wrong branch: the new branch gets them (and your uncommitted changes), and the current branch goes back to its upstream.
- **Co-authors:** `Co-authored-by` trailers are shown in commit rows (author initials such as `AL+GH`), in tooltips, and in the Commit Graph (an avatar per person, and "with …" in the details).
- **Remove Co-Author…** in **Amend Author…**, for co-authors added by mistake. It lists the commit's co-authors and keeps the rest of the message.
- **Main-branch colours:** setting `codegit.mainBranches` (default `main` and `master`) sets which branches count as the main branch.

### Changed
- Commit icon colours follow lazygit: green for commits already on a main branch, yellow for pushed, red for not pushed yet. Before, green meant pushed and yellow meant not pushed.
- Rewrites that include merge commits now refuse only squash, fixup and moving across a merge; reword, edit and drop work on the merge itself.

## 0.1.0

### Changed
- Faster view refreshes: commands that change the Commits view's filter, search or branch scope reload only that view, and only when something changed. Reload requests that arrive while one is queued join it. Saving a file no longer reloads the views unless refs, HEAD, the stash, config, worktrees or an in-progress operation changed.
- Fewer git calls: the git directory is looked up once per repository, branches load in one parallel round, and rebase checks run in parallel.
- Release builds accept version tags with or without a `v` prefix (`0.1.0` or `v0.1.0`).

## 0.0.1

First release.

- Branches, Commits, Reflog and Stashes views in the Source Control sidebar, in lazygit's order. The Commits view replaces the built-in Graph view.
- Branches view: local branches with ahead/behind counts, remotes, tags and worktrees, with checkout, merge, rebase (including onto a marked base and interactive), fast-forward, push, pull requests, delete merged branches and more.
- Commits view: squash, fixup, drop, reword, edit, amend into any commit, fixup commits and autosquash, amend author, move up/down, cherry-pick, reset, revert, bisect and compare. Rewrites also work across merge commits (`--rebase-merges`).
- A paused rebase lists its steps; reorder them or change their action before continuing.
- Line-level patches from a commit's diff: remove selected changes from the commit, move them to the index or split them into a new commit.
- Commit Graph editor, commit search, all-branches mode and file history.
- Undo / redo of Git actions from the reflog.
- Fetch / Pull / Push buttons on the Changes view.
