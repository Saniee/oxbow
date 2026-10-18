# Merge or rebase?

Both bring the work from one branch into another. They differ in what the history looks like afterwards.

## Merge

**Merge** joins two branches with a new *merge commit* that has both as parents. Nothing that already exists is changed.

- ✅ Safe for shared branches: no commit is rewritten, so nobody else is affected.
- ✅ The history shows exactly when the branches came together.
- ➖ The history has forks and joins, which can be harder to read.

**Use merge** to bring a finished feature into `main`, or whenever others already have your commits.

In the **Branches** panel: check out the branch that should receive the work, then right-click the other branch → **Merge into Current Branch**.

## Rebase

**Rebase** lifts your commits off and replays them on top of the other branch, as if you had started your work from its latest commit. Your commits get new IDs.

- ✅ A straight, easy-to-read history with no merge commits.
- ⚠️ It rewrites your commits. If you had already pushed them, you'll need to **force push**, and anyone who pulled them will have trouble.

**Use rebase** to update *your own* feature branch with the latest `main` before you share it.

In the **Branches** panel: check out your feature branch, then right-click `main` → **Rebase Current Branch onto This**.

Remember: rebase moves the branch you're **on**. It never adds anything to the branch you right-clicked.

## Interactive rebase

**Interactive Rebase** is a rebase that pauses before it starts, showing every commit it's about to replay as a list of steps at the top of the **Commits** panel. Before you press **Continue**, you can:

- reorder the steps,
- **squash** or **fixup** commits together,
- **reword** messages,
- **drop** commits you don't want,
- **edit** a commit: the rebase stops there so you can change its files.

Use it to clean up a whole branch in one go, for example before opening a pull request. **Abort** at any point puts everything back as it was.

## If there's a conflict

When both sides changed the same lines, Git stops and asks you to choose. Open the conflicting files in **Changes**, pick what to keep, stage them, then press **Continue**. **Abort** cancels the whole merge or rebase.
