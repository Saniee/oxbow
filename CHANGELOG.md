# Changelog

Oxbow's releases are tagged `oxbow-v<version>`. Each version's section here becomes its release notes. The SlothGit extension's own changelog, up to 2.0.0, is in this repository's history (commit `1877ada`).

## 0.1.0

The first release of Oxbow, a desktop Git workbench and the successor to the SlothGit VS Code extension. It's still young: try it on repositories you have a copy of, and report anything odd at https://github.com/Saniee/oxbow/issues.

Oxbow runs the git that's installed, so install [Git](https://git-scm.com) first.

### Added
- **Projects:** a start page of recent repositories, with Open folder… and New repository…. `oxbow <folder>` opens a folder directly.
- **Status:** branch, upstream and ahead/behind; Fetch, Pull and Push (pull with rebase, force push with lease); Continue, Skip and Abort while a merge, rebase, cherry-pick or revert is stopped.
- **Files:** git's two status letters per file; stage, unstage and discard; stash; commit and amend.
- **Branches:** local, remotes, tags and worktrees. Check out, create, rename, delete, merge, rebase onto, fast-forward, set upstream, push and delete tags.
- **Commits:** a commit graph with lanes, coloured by whether a commit is on a main branch, pushed or not; search; check out, branch, tag, cherry-pick, revert and reset. The **Reflog** tab recovers from any entry.
- **History rewriting:** reword, edit, squash, fixup, drop, move, amend into an older commit, fixup commits and reset the author, on one commit or several. Merges are kept, and Oxbow asks first when pushed commits would change.
- **Interactive rebase** that stops before the first step, so each remaining step can be changed before you continue.
- **Custom patches:** pick lines from a commit and take them out of it, move them to the working tree, into a new or another commit, or apply or revert them.
- **Cherry-pick list**, **bisect**, **worktrees**, **stash**, and **Undo / Redo** (Ctrl+Z, Ctrl+Shift+Z) from the reflog.
- **Details** of whatever is selected, and a **command log** of each action and the git commands it ran.
- **Settings**, including themes: the system's dark or light, Dark, Light, One Dark Pro, Gruvbox Dark and Gruvbox Light.
- Refreshes on its own when files in the repository change, and when its window gets focus.

### Downloads
- **Windows:** `oxbow-0.1.0-windows-x64-setup.exe` installs for your user without admin rights and can add Oxbow to PATH; `oxbow-0.1.0-windows-x64.zip` runs from anywhere. The installer isn't signed yet, so Windows may warn that it's from an unknown publisher.
- **Linux:** `oxbow-0.1.0-x86_64.AppImage` for most distributions, `oxbow_0.1.0_amd64.deb` for Debian, Ubuntu and their relatives, and `oxbow-0.1.0-linux-x64.tar.gz`.
- `SHA256SUMS` lists each file's checksum.

