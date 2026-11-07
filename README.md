# Oxbow

A desktop Git workbench written in Rust with [egui](https://github.com/emilk/egui). It's the successor to the
SlothGit VS Code extension, which is still in this repository's history: its last version is commit `1877ada`
(SlothGit 2.0.0), the one before Oxbow's first, and its releases are tagged `0.0.1` to `1.0.0`.

## Features

Everything SlothGit did, ported in three phases:

- **Projects:** Oxbow opens on a list of recently opened repositories, with Open folder… and New repository…
  (`git init`). `oxbow <folder>` opens that folder directly, or offers to create a repository there.
- **Status:** branch, upstream and ahead/behind, Fetch / Pull / Push (fetch all, pull with rebase, force push with
  lease), and Continue / Skip / Abort while a merge, rebase, cherry-pick or revert is stopped.
- **Files:** git's two status letters per file; stage, unstage, discard (all or unstaged only), stage all / unstage
  all, stash (tracked, with untracked, or staged only), commit and amend. Double-click a file to stage or unstage it.
- **Branches:** local (current first, then most recently checked out) with ahead/behind, remotes, tags and
  worktrees. Check out, create, rename, delete, merge, rebase onto, fast-forward, set upstream, push and delete tags.
- **Commits:** a commit graph with lanes for branches and merges; dots coloured green on a main branch, yellow when
  pushed, red when not; search; check out, branch, tag, cherry-pick, revert and reset (soft, mixed, hard). The
  **Reflog** tab recovers from any entry.
- **Cherry-pick list:** copy commits from anywhere (another branch, another worktree), then paste them onto HEAD,
  oldest first.
- **Custom patches:** in a commit's details, **Pick lines** and click changed lines (or add whole files). Then take
  them out of the commit, move them to the working tree, into a new commit right after it, or into another commit;
  or apply or revert them in the working tree.
- **Bisect:** mark commits bad or good from their menu; Oxbow checks out each commit to test, says how many are
  left, and names the first bad one.
- **Worktrees:** check a branch out in a new worktree, open a worktree in Oxbow, remove and prune them.
- **History rewriting** (right-click a commit): reword, edit (stop there), squash or fixup into the previous
  commit, drop, move up or down, amend the staged changes into an older commit, create a fixup commit, squash fixup
  commits, reset the author. Ctrl- and Shift-click select several commits to squash, fix up, drop or cherry-pick
  together. Merges are recreated (`--rebase-merges`), and Oxbow asks first when the rewrite touches pushed commits.
- **Interactive rebase:** "Rebase onto this interactively" on a branch stops before the first step; the remaining
  steps show above the commits, and each can be picked, reworded, edited, squashed, fixed up, dropped or moved before
  you continue.
- **Undo / Redo** (Ctrl+Z, Ctrl+Shift+Z) from the reflog: commits, checkouts, resets and whole rebases, keeping
  uncommitted changes.
- **Stash:** apply, pop, drop, branch from a stash.
- **Details:** the selection's message and patch, a file's staged and unstaged changes, or a branch's log.
- **Command log:** each action and the git commands it ran.
- **Settings** (⚙ in the toolbar): the theme (the system's dark or light, Dark, Light, One Dark Pro, Gruvbox Dark
  or Gruvbox Light), how many commits and reflog entries to show, which branches count as main,
  branch order, whether rewrites move stacked branches (`--update-refs`), whether to ask before rewriting pushed
  commits, and whether to refresh when files change. They're saved in `%APPDATA%\Oxbow\settings.txt` (Windows) or
  `~/.config/oxbow/settings.txt`, next to `recent.txt`.
- Oxbow refreshes on its own when files in the repository change (ignored files don't count), and when its window
  gets focus. The sections on the left can be resized by dragging the handles between them; double-click one to
  reset them.

## Install

Oxbow runs the git that's installed, so install [Git](https://git-scm.com) first. Then, from a release:

- **Windows:** `oxbow-<version>-windows-x64-setup.exe` installs for your user without admin rights (or for everyone,
  if you choose that on its first page), and can add Oxbow to PATH so `oxbow <folder>` works in a terminal.
  Installing a newer version updates it in place. `oxbow-<version>-windows-x64.zip` is the same program to run from
  anywhere. Windows may warn that the installer is from an unknown publisher, as it isn't signed yet.
- **Linux:** `oxbow-<version>-x86_64.AppImage` runs on most distributions (`chmod +x` it first).
  `oxbow_<version>_amd64.deb` installs it on Debian, Ubuntu and their relatives, with git if it's missing
  (`sudo apt install ./oxbow_<version>_amd64.deb`). `oxbow-<version>-linux-x64.tar.gz` is the bare program, with its
  menu entry and icon.

`SHA256SUMS` lists each file's checksum.

## Layout

- `crates/engine` (`oxbow-engine`): the Git backend. It runs your own `git` binary, so hooks, config and credential helpers behave
  as they do on the command line. It has no UI code.
- `crates/app` (`oxbow`): the egui desktop app.

Rebases never wait on an editor: Oxbow sets itself as git's sequence editor (`OXBOW_EDITOR`), which only puts a
`break` before the first step. Oxbow then rewrites the todo list and continues. Binaries that rebase call
`oxbow_engine::editor::run_if_requested()` first thing in `main`; the test binaries do it from a `ctor`.

### How Oxbow runs git

- **History order:** the commit list uses `git log --date-order`, so a commit is never listed below its parent, even
  when a commit was made on a computer whose clock was off. With a commit-graph file (which `git gc` writes, and
  `git commit-graph write` makes on its own) this costs nothing; without one, a long history takes a little longer.
- **Reuse:** a refresh reuses the commit list it read last time only while HEAD hasn't moved, and never in a shallow
  clone or a repository with grafts or replace refs, where history can change without HEAD moving. Each commit's
  branch and tag labels and its "2 hours ago" are worked out again on every refresh.
- **Status** saves refreshed file times in the index, as plain `git status` does, so files that only look changed
  (rewritten with the same content, or copied) are read once rather than on every refresh. That takes
  `index.lock` for a moment. Other reading commands never write the index.
- **Language:** git runs in English whatever the system language, as Oxbow reads some of its messages.
- **Paths** are always taken literally (a file named `[ab].txt` is just that file), and long lists of them go to git
  on stdin, past Windows' command-line limit.

## Development

```bash
cargo run -p oxbow
```

```bash
cargo test
```

The app's tests (`crates/app/src/ui_tests.rs`) drive the real window headless with
[egui_kittest](https://crates.io/crates/egui_kittest), against throwaway repositories. `crates/engine/tests/exact.rs`
checks everything Oxbow works out itself instead of asking git (labels, relative dates, a reused commit list)
against git's own answer.

Dependencies are built optimised even in `cargo run` and `cargo test`, as an unoptimised egui stutters and hides
real performance problems. To time the queries on a repository of your own:

```bash
OXBOW_PERF_REPO=path/to/repo cargo test -p oxbow-engine --test perf -- --ignored --nocapture
```

Pass a folder to open it directly: `cargo run -p oxbow -- path/to/repo`.

### Packages and releases

The packages are built into `dist/` by the same scripts locally and in the release workflow:

- **Windows** (needs [Inno Setup](https://jrsoftware.org/isinfo.php) 6 or later): `pwsh packaging\windows\build.ps1` makes
  the installer (`packaging/windows/oxbow.iss`) and the `.zip`.
- **Linux** (needs Docker, on any system): build the image once, then run it on the source.

  ```bash
  docker build -t oxbow-linux packaging/linux
  docker run --rm -v "$PWD:/src:ro" -v "$PWD/dist:/out" oxbow-linux
  ```

  It builds on Ubuntu 22.04, so the result runs on distributions with an older glibc too, and makes the AppImage,
  the `.deb` (described in `crates/app/Cargo.toml` under `[package.metadata.deb]`) and the `.tar.gz`.

The window and `.exe` icon is `crates/app/assets/icon.png` and `icon.ico`; `crates/app/build.rs` puts the icon and
version details in the `.exe`.

To release, set `version` in `crates/app/Cargo.toml`, build once so `Cargo.lock` follows, commit, and push a tag
`oxbow-v<version>` (the prefix keeps them apart from the SlothGit extension's tags). `.github/workflows/release.yml`
tests and builds on Windows and Linux, and makes a GitHub Release with every package and `SHA256SUMS`; a tag with a
suffix (`oxbow-v0.2.0-beta.1`) makes a pre-release. Its notes are that
version's section of `CHANGELOG.md` (`## 0.2.0`) when there is one, otherwise GitHub's generated notes. Running the
workflow by hand only builds, keeping the packages as the run's artifacts. `.github/workflows/ci.yml` runs the tests
and clippy on every push.

## License

GPL-3.0-only
