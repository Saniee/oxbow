# Undo almost anything

Made a mistake? Press **Undo** (↶) on the Commits panel's title. It takes back your last Git action, one step at a time:

- a commit, or a change to a commit (reword, squash, drop, amend…)
- a whole rebase, as one step
- a merge, a cherry-pick, a reset
- switching branches

**Redo** (↷) puts it back. Your uncommitted changes are kept either way.

Undo works even for things you did outside SlothGit, such as in the terminal, because it reads Git's own record of where your branch has been: the **reflog**.

## The Reflog

The **Reflog** tab of the Commits panel lists that record directly: every commit your branch pointed to, newest first. If a commit seems lost (you dropped it, or reset past it), find it there, right-click it and choose **Create Branch Here** or **Reset Current Branch to Here**.

Git keeps these entries for about 90 days, so very little is ever truly gone.
