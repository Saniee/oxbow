# Work on two branches at once: worktrees

Normally a repository has one working folder, so you can only have one branch checked out at a time. Switching means committing or stashing what you're doing first.

A **worktree** is an extra working folder for the same repository, with a different branch checked out. Both folders share the same history, branches and stashes; only the files on disk are separate.

**When it helps:**

- You're in the middle of a feature and need to fix an urgent bug on `main`. Open `main` in a worktree in a second window, fix it there, and your feature folder stays untouched.
- You want to run or test one branch while you keep coding on another.
- You're reviewing someone's branch and don't want to disturb your own work.

**In SlothGit:** right-click a branch → **New Worktree…**, pick a folder, then **Open in New Window**. Worktrees are listed in the **Branches** panel's **Worktrees** tab. When you're done, right-click the worktree → **Remove Worktree**; the branch itself is kept.

A branch can only be checked out in one worktree at a time. If you try to check it out elsewhere, SlothGit offers to open the worktree that has it.
