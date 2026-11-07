# Oxbow

A desktop Git workbench written in Rust with [egui](https://github.com/emilk/egui). It's the successor to the
SlothGit VS Code extension, whose last code is on the `comprehensive-ui-overhaul` branch.

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
- **Settings** (⚙ in the toolbar): how many commits and reflog entries to show, which branches count as main,
  branch order, whether rewrites move stacked branches (`--update-refs`), whether to ask before rewriting pushed
  commits, and whether to refresh when files change. They're saved in `%APPDATA%\Oxbow\settings.txt` (Windows) or
  `~/.config/oxbow/settings.txt`, next to `recent.txt`.
- Oxbow refreshes on its own when files in the repository change (ignored files don't count), and when its window
  gets focus. The sections on the left can be resized by dragging the handles between them; double-click one to
  reset them.

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

## License

GPL-3.0-only
