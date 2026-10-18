# Testing

**Status for 1.0.0: everything in the checklist below is tested and working.** Every item is covered by `npm run e2e`, which passes in full (126 scenarios), alongside the unit and integration tests (`npm test`). The e2e run opens a separate VS Code window on a fresh playground, runs each item with scripted answers to SlothGit's prompts, and checks the resulting git state. The window's status bar and the terminal show progress; the repository visibly resets between scenarios. Manual testing is still useful for what automation can't judge, such as layout, wording and how the workflow feels.

## Get a playground

```bash
npm run playground
```

This creates `../slothgit-playground/repo` (pass a folder to put it elsewhere: `npm run playground -- D:\tmp\pg`). Open that `repo` folder in VS Code with SlothGit installed. The remotes `origin` and `upstream` are bare repositories next to it, so pushing, fetching and deleting remote branches are safe and need no login.

Re-run the command whenever you want to start over. It replaces the previous playground (close VS Code windows on it first).

**Undo** (↶ on the Commits panel) is a quick way back after most experiments, and it is worth testing in its own right.

## What's in it

| Ref | State | Try |
| --- | --- | --- |
| `main` | 3 unpushed commits on top of pushed history; staged + unstaged changes; 2 stashes | Commit colors, every rewrite (dirty tree must survive), push |
| `main` history | "Refactor parser" puts `BUG` in `app.txt` | Bisect: bad on HEAD, good on "Add config", and see whether git finds "Refactor parser". Try it once with the dirty tree (git refuses; the error should be readable), then stash and retry |
| "Fix typo in docs" | Different author | Amend author → reset / set / co-author |
| "Add profile page" | Multi-line message | Reword in Editor; Copy… → message |
| "Add settings page" | Touches 2 files | Remove one file from the commit; move it to the index |
| "Rename logout to signout" | Rename | File list and diff of a rename |
| `feature/login` | Pushed, ahead 1 | Push, force push, create pull request (opens a URL; the remote is local, so expect nothing useful). Move Unpushed Commits to New Branch: the new branch gets "Login: error messages", `feature/login` matches `origin/feature/login` again |
| `feature/login-ui` | Stacked on `feature/login` | Mark "Login: error messages" as base, then rebase onto `main` with the marked base, so only the 2 UI commits move. With `slothgit.updateStackedBranches` set to `always`, reword a `feature/login` commit from here: `feature/login` moves too, and an interactive rebase lists "Update branch feature/login" |
| `conflict/status-line` | Edits the same line as `main` | Merge into `main`, or rebase it onto `main`, to get a conflict; Continue / Abort from the status bar |
| `feature/search` | Has a `fixup!` commit | Squash In Fixup Commits Above on "Add search"; Create Fixup Commit |
| `feature/with-merge` | Contains a merge commit | Reword or drop "Side work" below the merge: the merge is recreated. Reword, drop and revert the merge itself (revert asks which parent to keep). Squashing or moving the merge, or squashing "Merged-in work", should refuse with a clear message |
| `chore/gone-upstream` | Upstream deleted on remote | Shows "upstream gone"; Unset Upstream; Delete |
| `origin/remote-only` | No local branch | Checkout creates a tracking branch |
| `release/1.x` | Behind its upstream by 1 | Fast-forward without checking it out |
| `merged/old-experiment` | Already in `main` | Delete Merged Branches |
| `hotfix/crash` | Checked out in `../hotfix-worktree` | Checkout offers to open the worktree; worktree open / remove / prune |
| `v1.0`, `v0.9` | Annotated and lightweight tags | Expand, checkout, push, delete (also on remote) |
| `upstream` remote | Second remote | Fetch one remote, rename, edit URL, remove |

## Checklist

Ticked items are tested and working in 1.0.0. If you find something odd, such as a wrong label, a missing refresh, a confusing prompt or an error in **Output → SlothGit**, please open an issue.

### Branches panel
- [x] Sort order: recency, date, name
- [x] Checkout, checkout previous, new branch, rename, delete (local / remote / both; unmerged asks to force)
- [x] Merge: regular, `--no-ff`, squash
- [x] Rebase onto, plain and with a marked base; onto a branch already contained says there is nothing to rebase
- [x] Interactive rebase onto, plain and with a marked base: pauses first, change / reorder steps, Continue or Abort
- [x] Fast-forward, set / unset upstream, set remote branch as upstream
- [x] Push, force push with lease, fetch, fetch all
- [x] Compare with current, reset current to this, copy name
- [x] Remote: add, rename, edit URL, copy URL, remove
- [x] Tags: create (lightweight / annotated), push, delete
- [x] Worktrees: add (existing / new branch), open in new or current window, copy path, remove (dirty asks), prune

### Commits panel
- [x] Squash, fixup, drop (multi-select), reword, reword in editor, move up / down
- [x] The cursor stays on the commit after each of those (move up / down follows it)
- [x] Edit (pause), then change files, stage, Continue
- [x] Amend staged changes into an older commit
- [x] Create fixup commit, squash fixup commits above
- [x] Amend author (all three)
- [x] Cherry-pick copy on one branch, paste on another (applied oldest first); copied commits marked in the Commits and Branches views; remove one, clear; abort a multi-commit cherry-pick after committing the stopped one by hand
- [x] Reset soft / mixed / hard, checkout, revert, branch or tag here
- [x] Copy… (each item), open in browser
- [x] Commit files: open diff, open at revision, checkout file, remove from commit, move to index, Open All Changes
- [x] Line-level patches in a commit's diff (both sides, cursor only, several selections, Open All Changes): remove, move to index, move to new commit
- [x] Custom patch: add lines from two files of a commit (and a whole file from its context menu); from the status bar, move it into an older and a newer commit, apply it (and in reverse) to the working tree, show it, reset it. Adding from another commit asks to start over
- [x] Filter by file (toolbar, Explorer context menu, editor tab context menu), clear filter
- [x] Search by message, author and SHA; clear search
- [x] All branches: other branches' commits are dimmed and can't be rewritten
- [x] Commit Graph: lanes for `feature/with-merge` and all branches, labels, search, details pane, file diffs, double-click, right-click actions
- [x] During a paused rebase: steps read `pick  subject`; right-click actions on one and several steps (including reword); reorder; Continue / Skip / Abort on the title bar
- [x] Bisect: bad / good / skip, marks shown, status bar, reset
- [x] Compare: select, compare with selected (commits, branches, tags mixed)
- [x] Undo / redo across: commit, amend, reset, rebase, checkout, merge

### Stashes and reflog
- [x] Stash: all / include untracked / staged only / keep index; apply, pop, drop, rename, branch from stash
- [x] Reflog: checkout, reset, branch, cherry-pick copy from an entry

### General
- [x] Right-click menus on a commit, a branch and in the Commit Graph: More Rewrite Actions, Bisect and Upstream submenus hold the moved items; nothing appears where it can't run
- [x] A slow action (push to a slow remote, a long rebase) shows a status-bar spinner and, after ~1.5 s, a notification; quick actions flash "…: done" in the status bar
- [ ] Status panel: branch, upstream and ↑↓ counts follow commits, pushes and checkouts; Pull, Push and Publish Branch; the ⌄ quick picks and right-click menus: Pull (Rebase), Fetch, Fetch All, Force Push, Push to Another Branch, Set Upstream; Continue / Skip / Abort while an operation is stopped
- [x] Rewriting pushed commits asks first (setting `slothgit.confirmRewritingPushedCommits`)
- [ ] The tab refreshes after you commit or switch branches outside SlothGit, and the Files panel after you edit a file
- [x] Multi-root workspace / several repositories: Switch Repository
- [ ] Simple Mode: the Status panel's … menu toggles it; advanced actions, buttons and the Worktrees and Reflog tabs are hidden
- [x] `codegit.*` settings from before the rename are copied to `slothgit.*`

### SlothGit tab
- [ ] Opens from the Activity Bar icon, the Source Control title bar button and the Command Palette; reopens after a window reload
- [ ] Every list: right-click shows the same menu the old view had; hover buttons and panel title buttons (and their … menus) run their commands
- [ ] Ctrl- and Shift-click select several commits; Squash / Drop from the right-click menu apply to all of them
- [ ] Selecting a commit, a reflog entry, a stash, a branch, a remote, a tag, a worktree and a file shows it on the right; a branch's log opens its commits, and Back returns
- [ ] The patch: colours, line numbers, a file header collapses its file; "Open in the Diff Editor" and clicking a file row open the native diff beside the tab
- [ ] A large commit shows the start of its diff and "Show the Whole Diff"
- [ ] Files panel: stage / unstage / discard one file, several (Ctrl-click) and all; a file with staged and unstaged changes asks which to discard; conflicts open in the merge editor
- [ ] Commit box: commit, amend (with and without a new message), nothing staged offers to stage everything; the draft survives closing and reopening the tab
- [ ] Command Log: each action with the git commands it ran; failed ones in red; Clear
- [ ] Dragging the two dividers resizes the panels; the sizes are remembered
- [ ] Light, dark and high-contrast themes
