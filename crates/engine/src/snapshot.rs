//! Everything the panels show, read in one go (in parallel) after each change.

use std::collections::HashSet;

use crate::git::{Git, Result};
use crate::queries::{self, BranchSort, Commit, LocalBranch, LogQuery, Operation, ReflogEntry, Remote, RemoteBranch, Stash, Tag, Worktree};
use crate::status::{self, StatusEntry};
use crate::todo::TodoEntry;
use crate::undo::{self, UndoPlan};
use crate::rebase;

#[derive(Clone, Debug)]
pub struct Settings {
    pub commit_limit: usize,
    pub reflog_limit: usize,
    pub main_branches: Vec<String>,
    pub branch_sort: BranchSort,
}

impl Default for Settings {
    fn default() -> Self {
        Self { commit_limit: 300, reflog_limit: 200, main_branches: vec!["main".into(), "master".into()], branch_sort: BranchSort::Recency }
    }
}

/// Where a commit stands, for its colour: on a main branch, pushed, or only here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMark {
    Merged,
    Pushed,
    Unpushed,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub operation: Option<Operation>,
    pub status: Vec<StatusEntry>,
    pub local_branches: Vec<LocalBranch>,
    pub remotes: Vec<Remote>,
    pub remote_branches: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
    pub worktrees: Vec<Worktree>,
    pub commits: Vec<Commit>,
    pub merged: HashSet<String>,
    /// None when the branch has no upstream.
    pub unpushed: Option<HashSet<String>>,
    pub reflog: Vec<ReflogEntry>,
    pub stashes: Vec<Stash>,
    /// The remaining steps of a paused interactive rebase, next one first.
    pub todo: Vec<TodoEntry>,
    pub undo: UndoPlan,
}

fn join<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("snapshot query panicked")
}

impl Snapshot {
    pub fn current_branch(&self) -> Option<&LocalBranch> {
        self.local_branches.iter().find(|b| b.current)
    }

    pub fn mark(&self, sha: &str) -> CommitMark {
        if self.merged.contains(sha) {
            CommitMark::Merged
        } else if self.unpushed.as_ref().is_some_and(|u| !u.contains(sha)) {
            CommitMark::Pushed
        } else {
            CommitMark::Unpushed
        }
    }

    /// The remote to push a branch without upstream to: "origin", or the only remote.
    pub fn default_remote(&self) -> Option<&str> {
        self.remotes
            .iter()
            .find(|r| r.name == "origin")
            .or_else(|| (self.remotes.len() == 1).then(|| &self.remotes[0]))
            .map(|r| r.name.as_str())
    }

    pub fn load(git: &Git, settings: &Settings, search: Option<&str>) -> Result<Snapshot> {
        std::thread::scope(|s| {
            let status = s.spawn(|| status::status(git));
            let branches = s.spawn(|| queries::local_branches(git, settings.branch_sort));
            let remote_side = s.spawn(|| -> Result<_> {
                let remotes = queries::remotes(git)?;
                let branches = queries::remote_branches(git, &remotes)?;
                Ok((remotes, branches))
            });
            let tags = s.spawn(|| queries::tags(git));
            let worktrees = s.spawn(|| queries::worktrees(git));
            let commits = s.spawn(|| -> Result<_> {
                let query = LogQuery { limit: settings.commit_limit, search: search.map(str::to_owned), ..Default::default() };
                let commits = queries::log(git, &query)?;
                let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
                let main = queries::main_refs(git, &settings.main_branches)?;
                let merged = queries::reachable_from(git, &shas, &main)?;
                Ok((commits, merged, queries::unpushed(git)?))
            });
            let reflog = s.spawn(|| -> Result<_> {
                // Undo looks further back than the Reflog tab shows.
                let mut reflog = queries::reflog(git, settings.reflog_limit.max(1000))?;
                let undo = undo::plan_undo(&reflog);
                reflog.truncate(settings.reflog_limit);
                Ok((reflog, undo))
            });
            let todo = s.spawn(|| rebase::todo_steps(git));
            let stashes = s.spawn(|| queries::stashes(git));
            let head = s.spawn(|| -> Result<_> { Ok((queries::current_branch(git)?, queries::head_sha(git)?, queries::operation(git)?)) });

            let (branch, head, operation) = join(head)?;
            let (remotes, remote_branches) = join(remote_side)?;
            let (commits, merged, unpushed) = join(commits)?;
            let (reflog, undo) = join(reflog)?;
            Ok(Snapshot {
                branch,
                head,
                operation,
                status: join(status)?,
                local_branches: join(branches)?,
                remotes,
                remote_branches,
                tags: join(tags)?,
                worktrees: join(worktrees)?,
                commits,
                merged,
                unpushed,
                reflog,
                stashes: join(stashes)?,
                todo: join(todo)?,
                undo,
            })
        })
    }
}
