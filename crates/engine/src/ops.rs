//! Changing the repository: commits, branches, tags, stashes, sync and stopped operations. History rewriting
//! (interactive rebase) lives in its own module.

use crate::git::{Exec, Git, GitError, Result};
use crate::queries::{self, LocalBranch, Operation};

// --- commits ---------------------------------------------------------------------------

/// Commits the index with `message`. With `amend`, replaces HEAD instead (keeping its message when `message` is empty).
/// `git init` in `dir`, which is created if needed; returns the new repository.
pub fn init(dir: &std::path::Path) -> Result<Git> {
    std::fs::create_dir_all(dir).map_err(|e| crate::git::GitError::user(format!("Could not create {}: {e}", dir.display())))?;
    Git::new(dir).run(["init", "-q"])?;
    Git::open(dir)
}

pub fn commit(git: &Git, message: &str, amend: bool) -> Result<()> {
    let message = message.trim();
    let mut args = vec!["commit"];
    if amend {
        args.push("--amend");
        if message.is_empty() {
            args.push("--no-edit");
            git.run(&args)?;
            return Ok(());
        }
    } else if message.is_empty() {
        return Err(GitError::user("Write a commit message first."));
    }
    args.extend(["--cleanup=strip", "-F", "-"]);
    git.exec(&args, Exec { input: Some(&format!("{message}\n")), ..Default::default() })?;
    Ok(())
}

pub fn cherry_pick(git: &Git, shas: &[String]) -> Result<()> {
    let mut args = vec!["cherry-pick".to_owned(), "--allow-empty".to_owned()];
    args.extend(shas.iter().cloned());
    git.run(&args)?;
    Ok(())
}

pub fn revert(git: &Git, sha: &str, is_merge: bool) -> Result<()> {
    if is_merge {
        git.run(["revert", "--no-edit", "-m", "1", sha])?;
    } else {
        git.run(["revert", "--no-edit", sha])?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

pub fn reset(git: &Git, rev: &str, mode: ResetMode) -> Result<()> {
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    git.run(["reset", flag, rev])?;
    Ok(())
}

/// Checks out a commit, detaching HEAD.
pub fn checkout_detached(git: &Git, rev: &str) -> Result<()> {
    git.run(["checkout", "--detach", rev])?;
    Ok(())
}

// --- branches --------------------------------------------------------------------------

pub fn checkout(git: &Git, branch: &str) -> Result<()> {
    git.run(["checkout", branch, "--"])?;
    Ok(())
}

/// Checks out a remote branch: its local branch when there is one, otherwise a new tracking branch.
pub fn checkout_remote(git: &Git, remote_branch: &str, local_name: &str) -> Result<()> {
    let exists = git
        .exec(["rev-parse", "-q", "--verify", &format!("refs/heads/{local_name}")], Exec { allowed: &[1, 128], ..Default::default() })?
        .code
        == 0;
    if exists {
        checkout(git, local_name)
    } else {
        git.run(["checkout", "-b", local_name, "--track", remote_branch])?;
        Ok(())
    }
}

pub fn create_branch(git: &Git, name: &str, start: &str, checkout: bool) -> Result<()> {
    if checkout {
        git.run(["checkout", "-b", name, start])?;
    } else {
        git.run(["branch", name, start])?;
    }
    Ok(())
}

pub fn rename_branch(git: &Git, old: &str, new: &str) -> Result<()> {
    git.run(["branch", "-m", old, new])?;
    Ok(())
}

/// Deletes a local branch; without `force`, git refuses when it isn't merged.
pub fn delete_branch(git: &Git, name: &str, force: bool) -> Result<()> {
    git.run(["branch", if force { "-D" } else { "-d" }, name])?;
    Ok(())
}

pub fn delete_remote_branch(git: &Git, remote: &str, branch: &str) -> Result<()> {
    git.run(["push", remote, "--delete", branch])?;
    Ok(())
}

pub fn merge(git: &Git, rev: &str) -> Result<()> {
    git.run(["merge", "--no-edit", rev])?;
    Ok(())
}

pub fn rebase_onto(git: &Git, rev: &str) -> Result<()> {
    git.run(["rebase", "--autostash", rev])?;
    Ok(())
}

/// Moves a branch that isn't checked out to its upstream, if that is a fast-forward.
pub fn fast_forward(git: &Git, branch: &LocalBranch) -> Result<()> {
    let upstream = branch.upstream.as_ref().ok_or_else(|| GitError::user(format!("{} has no upstream.", branch.name)))?;
    if branch.current {
        git.run(["merge", "--ff-only", &upstream.name])?;
    } else {
        git.run(["fetch", ".", &format!("refs/remotes/{}:refs/heads/{}", upstream.name, branch.name)])?;
    }
    Ok(())
}

pub fn set_upstream(git: &Git, branch: &str, upstream: &str) -> Result<()> {
    git.run(["branch", &format!("--set-upstream-to={upstream}"), branch])?;
    Ok(())
}

// --- tags ------------------------------------------------------------------------------

pub fn create_tag(git: &Git, name: &str, rev: &str, message: Option<&str>) -> Result<()> {
    match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => git.exec(["tag", "-a", name, "-F", "-", rev], Exec { input: Some(&format!("{m}\n")), ..Default::default() })?,
        None => git.exec(["tag", name, rev], Exec::default())?,
    };
    Ok(())
}

pub fn delete_tag(git: &Git, name: &str) -> Result<()> {
    git.run(["tag", "-d", name])?;
    Ok(())
}

pub fn push_tag(git: &Git, remote: &str, name: &str) -> Result<()> {
    git.run(["push", remote, &format!("refs/tags/{name}")])?;
    Ok(())
}

// --- sync ------------------------------------------------------------------------------

pub fn fetch(git: &Git, all: bool) -> Result<()> {
    if all {
        git.run(["fetch", "--all", "--prune"])?;
    } else {
        git.run(["fetch", "--prune"])?;
    }
    Ok(())
}

pub fn pull(git: &Git, rebase: bool) -> Result<()> {
    if rebase {
        git.run(["pull", "--rebase", "--autostash"])?;
    } else {
        git.run(["pull", "--no-rebase"])?;
    }
    Ok(())
}

/// Pushes the current branch. Without an upstream, pushes to `default_remote` and sets it.
pub fn push(git: &Git, force: bool, default_remote: Option<&str>) -> Result<()> {
    let branch = queries::current_branch(git)?.ok_or_else(|| GitError::user("HEAD is detached: check out a branch to push it."))?;
    let has_upstream = git.exec(["rev-parse", "--abbrev-ref", "@{upstream}"], Exec { allowed: &[128], ..Default::default() })?.code == 0;
    let mut args = vec!["push".to_owned()];
    if force {
        args.push("--force-with-lease".to_owned());
    }
    if !has_upstream {
        let remote = default_remote.ok_or_else(|| GitError::user("This repository has no remote to push to."))?;
        args.extend(["-u".to_owned(), remote.to_owned(), format!("HEAD:refs/heads/{branch}")]);
    }
    git.run(&args)?;
    Ok(())
}

// --- stash -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashKind {
    /// Tracked changes, staged and unstaged.
    Tracked,
    /// Untracked files too.
    IncludeUntracked,
    /// Only what is staged.
    Staged,
}

pub fn stash_push(git: &Git, message: &str, kind: StashKind) -> Result<()> {
    let mut args = vec!["stash", "push"];
    match kind {
        StashKind::Tracked => {}
        StashKind::IncludeUntracked => args.push("--include-untracked"),
        StashKind::Staged => args.push("--staged"),
    }
    let message = message.trim();
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    git.run(&args)?;
    Ok(())
}

pub fn stash_apply(git: &Git, selector: &str) -> Result<()> {
    git.run(["stash", "apply", "--index", selector])?;
    Ok(())
}

pub fn stash_pop(git: &Git, selector: &str) -> Result<()> {
    git.run(["stash", "pop", "--index", selector])?;
    Ok(())
}

pub fn stash_drop(git: &Git, selector: &str) -> Result<()> {
    git.run(["stash", "drop", selector])?;
    Ok(())
}

pub fn stash_branch(git: &Git, name: &str, selector: &str) -> Result<()> {
    git.run(["stash", "branch", name, selector])?;
    Ok(())
}

// --- stopped operations ----------------------------------------------------------------

pub fn continue_operation(git: &Git, op: Operation) -> Result<()> {
    match op {
        Operation::Merge => git.run(["commit", "--no-edit"])?,
        other => git.run([other.name(), "--continue"])?,
    };
    Ok(())
}

pub fn skip_operation(git: &Git, op: Operation) -> Result<()> {
    if op == Operation::Merge {
        return Err(GitError::user("A merge can't be skipped; abort it instead."));
    }
    git.run([op.name(), "--skip"])?;
    Ok(())
}

pub fn abort_operation(git: &Git, op: Operation) -> Result<()> {
    git.run([op.name(), "--abort"])?;
    Ok(())
}

// --- Bisect ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectMark {
    Bad,
    Good,
    Skip,
}

/// Marks `sha`, starting a bisect first if none is going. git then checks out the next commit to test.
pub fn bisect_mark(git: &Git, sha: &str, mark: BisectMark) -> Result<()> {
    let state = crate::queries::bisect_state(git)?;
    if state.is_none() {
        git.run(["bisect", "start"])?;
    }
    let (bad, good) = state.map(|s| s.terms).unwrap_or_else(|| ("bad".into(), "good".into()));
    let term = match mark {
        BisectMark::Bad => bad,
        BisectMark::Good => good,
        BisectMark::Skip => "skip".into(),
    };
    git.run(["bisect", term.as_str(), sha])?;
    Ok(())
}

/// Ends the bisect and checks out what was checked out before it.
pub fn bisect_reset(git: &Git) -> Result<()> {
    git.run(["bisect", "reset"])?;
    Ok(())
}

// --- Worktrees -------------------------------------------------------------------------

/// Adds a worktree at `dir` with `branch` checked out; with `new_from`, `branch` is created there first.
pub fn worktree_add(git: &Git, dir: &str, branch: &str, new_from: Option<&str>) -> Result<()> {
    match new_from {
        Some(start) => git.run(["worktree", "add", "-b", branch, dir, start])?,
        None => git.run(["worktree", "add", dir, branch])?,
    };
    Ok(())
}

pub fn worktree_remove(git: &Git, dir: &str, force: bool) -> Result<()> {
    if force {
        git.run(["worktree", "remove", "--force", dir])?;
    } else {
        git.run(["worktree", "remove", dir])?;
    }
    Ok(())
}

/// Forgets worktrees whose folder is gone.
pub fn worktree_prune(git: &Git) -> Result<()> {
    git.run(["worktree", "prune"])?;
    Ok(())
}
