# Oxbow

A desktop Git workbench written in Rust with [egui](https://github.com/emilk/egui). It's the successor to the
SlothGit VS Code extension, whose last code is on the `comprehensive-ui-overhaul` branch.

## Features

Ported from SlothGit so far (phases 1 and 2 of 3):

- **Status:** branch, upstream and ahead/behind, Fetch / Pull / Push (fetch all, pull with rebase, force push with
  lease), and Continue / Skip / Abort while a merge, rebase, cherry-pick or revert is stopped.
- **Files:** git's two status letters per file; stage, unstage, discard (all or unstaged only), stage all / unstage
  all, stash (tracked, with untracked, or staged only), commit and amend. Double-click a file to stage or unstage it.
- **Branches:** local (current first, then most recently checked out) with ahead/behind, remotes, tags and
  worktrees. Check out, create, rename, delete, merge, rebase onto, fast-forward, set upstream, push and delete tags.
- **Commits:** coloured green on a main branch, yellow when pushed, red when not; search; check out, branch, tag,
  cherry-pick, revert and reset (soft, mixed, hard). The **Reflog** tab recovers from any entry.
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

Still to come: patches and custom patches, the commit graph, bisect, worktree actions, settings and Simple Mode.

## Layout

- `crates/engine` (`oxbow-engine`): the Git backend. It runs your own `git` binary, so hooks, config and credential helpers behave
  as they do on the command line. It has no UI code.
- `crates/app` (`oxbow`): the egui desktop app.

Rebases never wait on an editor: Oxbow sets itself as git's sequence editor (`OXBOW_EDITOR`), which only puts a
`break` before the first step. Oxbow then rewrites the todo list and continues. Binaries that rebase call
`oxbow_engine::editor::run_if_requested()` first thing in `main`; the test binaries do it from a `ctor`.

## Development

```bash
cargo run -p oxbow
```

```bash
cargo test
```

The app's tests (`crates/app/src/ui_tests.rs`) drive the real window headless with
[egui_kittest](https://crates.io/crates/egui_kittest), against throwaway repositories.

Run the app from inside a repository to open it directly, or use **Open repository…**.

## License

GPL-3.0-only
