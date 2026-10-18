# Find the commit that broke something: bisect

Something worked last week and is broken now, and there are dozens of commits in between. **Bisect** finds the commit that introduced the problem by halving the search each time, so even 100 commits take only about 7 checks.

1. In the **Commits** panel, right-click a commit where the problem exists (usually the newest) → **Bisect → Mark as Bad (Has the Problem)**.
2. Right-click an older commit where it still worked → **Bisect → Mark as Good (Works)**.
3. SlothGit checks out a commit halfway between. Test it: run the app, run the test, whatever shows the problem.
4. Right-click the checked-out commit (marked *testing*) → **Mark as Good** or **Mark as Bad**. Use **Skip** if you can't test that commit.
5. Repeat until Git names the first bad commit. SlothGit tells you which one and offers to **Stop Bisecting**, which takes you back to your branch.

Your branch isn't changed by bisecting; SlothGit only checks out commits temporarily. Commit or stash your work before starting.
