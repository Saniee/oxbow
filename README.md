# SlothGit

Lazygit-inspired power tools that fill the gaps in VS Code's built-in Git support, in a tab of their own. The **SlothGit** tab puts lazygit's panels side by side: Status, Files, Branches, Commits and Stash on the left, and on the right whatever you select, such as a commit's message and patch, a branch's log or a file's changes, with a log of the Git commands SlothGit ran below it. It looks like VS Code, not a terminal, and everything works with the mouse. SlothGit builds on the built-in Git extension (same repositories, same diff and merge editors) rather than replacing it.

## Getting started

Click the SlothGit icon in the Activity Bar, the SlothGit button on the Source Control title bar, or run **SlothGit: Open SlothGit**. **Get Started with SlothGit** on VS Code's Welcome page explains each panel and, in plain language, what squash, fixup, rebase, cherry-pick, stashes, worktrees and bisect are for.

**Right-click** anything in the tab to see what you can do with it. The most common actions are also buttons on a row when the mouse is over it, and on each panel's title. New to Git? **Switch to Simple Mode** in the Status panel's **…** menu hides the advanced actions until you turn it off again.

## Features

- **Status:** the repository, the checked-out branch, its upstream and ↑↓ counts, with Fetch, Pull and Push buttons. Each has a ⌄ (also on right-click) with Pull (Rebase), Fetch All Remotes, Force Push, Push to Another Branch and Set Upstream. While a rebase, merge, cherry-pick or revert is stopped, Continue, Skip and Abort are right there.
- **Files:** every changed file once, with git's two status letters (staged and unstaged). Stage, unstage or discard a file or everything, open it, open its changes, resolve conflicts in the merge editor, and commit or amend from the commit box. Selecting a file shows its staged and unstaged changes.
- **Undo / redo** (↶ / ↷ on the Commits panel) for commits, amends, resets, rebases, merges, cherry-picks and checkouts, like lazygit's `z` / `Z`. It works from the reflog, so it covers actions run outside SlothGit too.
- **Branches:** local branches (current first, then most recently used) with ahead/behind counts, remotes, tags and worktrees, each in its own tab of the panel. Selecting one shows its log. Checkout, merge, rebase (whole branch, from a marked base, or interactive), fast-forward without checking out, push / force push with lease, upstreams, pull requests, delete merged branches, and moving commits made on the wrong branch to a new one.
- **Commits:** rewrite history from the commit list: squash, fixup, drop, reword, edit, move up / down, amend staged changes into an older commit, fixup commits and autosquash, amend author and co-authors, cherry-pick, revert, reset. Ctrl- or Shift-click to act on several commits. Works across merge commits (`--rebase-merges`) and can move stacked branches along (`slothgit.updateStackedBranches`).
- **Patches:** remove or move a file's changes, or selected lines in a commit's diff, out of a commit, to the index or a new commit. Collect several into a custom patch (status bar) and move it into any other commit.
- **Paused rebases:** the remaining steps are listed at the top of the Commits panel. Right-click to change them (pick, reword, squash, fixup, edit, drop), reorder with ↑ / ↓, then Continue, Skip or Abort from the Status panel or the status bar.
- **Commit colours** as in lazygit: green when on the main branch (`slothgit.mainBranches`), yellow when pushed, red when not. Rewriting pushed commits asks first.
- **Commit Graph:** history as a graph in an editor tab, with branch lanes, labels, search, details and the same right-click actions.
- **Search, filter by file, all branches, bisect and compare** in the Commits panel.
- **Stash:** apply, pop, drop, rename, branch from a stash; stash untracked or only staged changes.
- **Reflog** (a tab of the Commits panel): recover dropped commits or undo resets from any entry.
- **Command log:** each action and the Git commands it ran, newest first.

Diffs and files opened from the tab open beside it, so the tab stays in view.

## Settings

| Setting | Default | |
| --- | --- | --- |
| `slothgit.simpleMode` | `false` | Hide advanced actions from the menus |
| `slothgit.branchSort` | `recency` | Order of local branches: `recency`, `date` or `name` |
| `slothgit.mainBranches` | `main`, `master` | Branches whose commits are shown green |
| `slothgit.confirmRewritingPushedCommits` | `true` | Ask before rewriting pushed commits |
| `slothgit.updateStackedBranches` | `gitConfig` | Move stacked branches along when rewriting (`always`, `never`, or git's `rebase.updateRefs`) |
| `slothgit.commitLimit` / `slothgit.reflogLimit` | `300` / `200` | How many entries the Commits and Reflog lists load |

SlothGit was called CodeGit before 1.0.0. `codegit.*` settings are copied over on first start.

## Theme colours

The SlothGit tab and the Commit Graph use your colour theme: panels, lists, buttons, inputs, menus and diffs take the theme's own colours. SlothGit's own parts have their own colour ids, which a theme can set in its `colors` or you in `workbench.colorCustomizations`. Each defaults to a standard colour, so themes that don't set them still look right:

| Colour | Default | |
| --- | --- | --- |
| `slothgit.panelBackground` / `slothgit.panelBorder` | `sideBar.background` / `panel.border` | The tab's panels |
| `slothgit.commitMergedIcon` / `commitPushedIcon` / `commitUnpushedIcon` | `charts.green` / `charts.yellow` / `charts.red` | Commit icons: on a main branch, pushed, not pushed |
| `slothgit.commitOtherBranchIcon` | `disabledForeground` | Other branches' commits, when showing all branches |
| `slothgit.cherryPickedIcon` / `rebaseBaseIcon` / `rebaseTodoIcon` | `charts.blue` / `charts.purple` / `charts.orange` | Copied for cherry-pick, rebase base, pending rebase step |
| `slothgit.bisectBadIcon` / `bisectGoodIcon` / `bisectSkippedIcon` / `bisectTestingIcon` | `charts.red` / `charts.green` / `disabledForeground` / `charts.yellow` | Bisect marks |
| `slothgit.currentBranchIcon` / `worktreeBranchIcon` / `goneIcon` | `charts.green` / `charts.blue` / `disabledForeground` | Branches and worktrees |
| `slothgit.headRefBackground` / `headRefForeground` | `charts.green` / `editor.background` | The checked-out branch's label on a commit |
| `slothgit.localRefForeground` / `remoteRefForeground` / `tagRefForeground` | `charts.green` / `charts.blue` / `charts.yellow` | Branch and tag labels on commits |
| `slothgit.stagedForeground` / `unstagedForeground` / `untrackedForeground` / `conflictForeground` | the `gitDecoration.*` colours | Status letters in the Files panel |
| `slothgit.graphLane1` … `graphLane8` | `charts.*`, `terminal.ansiCyan`, `terminal.ansiMagenta` | The Commit Graph's lanes |

For example:

```jsonc
"workbench.colorCustomizations": {
  "[Ion Atom]": { "slothgit.headRefBackground": "#3fa9f5", "slothgit.panelBackground": "#0d1826" }
}
```

## How it works

SlothGit calls the git CLI that the built-in Git extension uses. Every rewrite is a `git rebase --interactive --autostash`. Git runs `dist/rebaseEditor.js` as the sequence editor, and it rewrites the todo list from a plan. If anything doesn't match the plan, it exits non-zero and git aborts before touching history. Undo replays the HEAD reflog as a stack of actions.

The SlothGit tab and the Commit Graph are webview pages. The tab's lists are the providers' tree items (`src/views/`) turned into rows (`src/tab/rows.ts`); each row carries its kind and state in `data-vscode-context`, so its right-click menu is VS Code's own (`webview/context` in `package.json`), and the commands get the row's node back (`src/views/webviewNodes.ts`). Row and panel buttons are listed in `src/tab/actions.ts`. Each lives in `src/webview/<page>/` as markup (`body.html`), styles (`style.css`) and a script (`main.ts`, type-checked against the DOM by `src/webview/tsconfig.json`). esbuild bundles them into `dist/webview/`. The messages between a page and the extension are typed in `src/webview/messages.ts`, which both sides import.

## Development

```bash
npm install
npm test            # unit tests plus real-git integration tests
npm run e2e         # every TESTING.md item against a playground in VS Code (`npm run e2e -- stash` for a subset)
npm run playground  # builds ../slothgit-playground for trying things by hand (see TESTING.md)
npm run watch       # rebuild on change
```

Press `F5` to launch an Extension Development Host.

### Releases

Pushing a version tag (matching `version` in `package.json`, with or without a leading `v`) builds the extension, runs the tests and attaches the `.vsix` to a GitHub Release. The release notes are that version's section of [CHANGELOG.md](CHANGELOG.md). Tags with a suffix, such as `1.1.0-beta.1`, become pre-releases.

```bash
git tag 1.0.0
```

```bash
git push origin 1.0.0
```

To publish to [Open VSX](https://open-vsx.org) by hand, create an access token in your Open VSX settings and create the `Saniee` namespace once (`npx ovsx create-namespace Saniee -p <token>`). Then run:

```bash
OVSX_PAT=<token> npm run publish:ovsx
```

The release workflow does the same automatically when the repository has an `OVSX_PAT` secret.

## Roadmap

- Visual interactive rebase editor, like GitLens's: drag commits to reorder them, mark them to squash, reword or drop, see ahead of time which steps will conflict, and resolve every conflicted file from one panel [[feature/actually-interactive-rebase]](https://github.com/Saniee/SlothGit/tree/feature/actually-interactive-rebase)
- Submodules
- Custom commands (lazygit's `customCommands`)
- Git blame [[feature/git-blame]](https://github.com/Saniee/SlothGit/tree/feature/git-blame)
- Issues and pull requests dashboard[[feature/issues-pullrequest-dashboard]](https://github.com/Saniee/SlothGit/tree/feature/issues-pullrequest-dashboard)
