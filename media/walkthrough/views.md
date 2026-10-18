# Where SlothGit lives

SlothGit opens in a tab of its own: click the SlothGit icon in the Activity Bar, or the SlothGit button on the **Source Control** title bar. Like lazygit, it shows everything side by side:

| Panel | What it shows | You use it to… |
| --- | --- | --- |
| **Status** | The branch you're on and how far it is ahead (↑) or behind (↓) its copy on the remote | Pull, push, fetch; the ⌄ arrows hold the other ways to do it. Continue or abort a rebase or merge that stopped |
| **Files** | Files you changed since the last commit, and whether each change is staged | Stage, unstage or discard changes, and commit them with the box below |
| **Branches** | Your branches, the remotes (such as GitHub) and their branches, tags and worktrees, one tab each | Switch branches, merge, rebase, push, pull |
| **Commits** | The history of the branch you're on, newest at the top; its **Reflog** tab lists every place your branch has pointed to recently | Change commits: reword, squash, drop, reorder, undo. Get back something you lost |
| **Stash** | Work you put aside without committing | Bring that work back later |

Select anything and the big pane on the right shows it: a commit's message, files and changes, a branch's history, a file's changes. Below it, the **Command Log** lists the Git commands SlothGit ran for you.

**Right-click** anything to see what you can do with it. The most common actions are also buttons on a row when the mouse is over it. Hover over a commit or branch for its details.

## Simple Mode

New to Git? Choose **Switch to Simple Mode** in the Status panel's **…** menu. It hides the advanced actions (bisect, custom patches, worktrees, compare, resets and the Reflog) so the menus show only everyday things. **Show All Actions** in the same menu brings them back. Everything stays available from the Command Palette.
