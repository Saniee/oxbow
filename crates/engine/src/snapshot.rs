//! Everything the panels show, read in one go (in parallel) after each change.

use std::collections::HashSet;

use crate::git::{Git, Result};
use crate::queries::{self, BisectState, BranchSort, Commit, LocalBranch, LogQuery, Operation, ReflogEntry, Remote, RemoteBranch, Stash, Tag, Worktree};
use crate::status::{self, StatusEntry};
use crate::todo::TodoEntry;
use crate::undo::{self, UndoPlan};
use crate::rebase;
use crate::graph;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub commit_limit: usize,
    pub reflog_limit: usize,
    pub main_branches: Vec<String>,
    pub branch_sort: BranchSort,
    pub update_refs: rebase::UpdateRefs,
}

impl Default for Settings {
    fn default() -> Self {
        Self { commit_limit: 300, reflog_limit: 200, main_branches: vec!["main".into(), "master".into()], branch_sort: BranchSort::Recency, update_refs: rebase::UpdateRefs::GitConfig }
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
    /// The graph beside `commits`, a row each; empty while searching, as found commits aren't connected.
    pub graph: Vec<graph::Row>,
    pub merged: HashSet<String>,
    /// None when the branch has no upstream.
    pub unpushed: Option<HashSet<String>>,
    pub reflog: Vec<ReflogEntry>,
    pub stashes: Vec<Stash>,
    /// The remaining steps of a paused interactive rebase, next one first.
    pub todo: Vec<TodoEntry>,
    pub undo: UndoPlan,
    pub bisect: Option<BisectState>,
}

/// What the commit list was read for, so the next refresh can tell whether it would read the same again: the log
/// is read from this exact commit, and commits never change, so the same key gives the same list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LogKey {
    head: String,
    search: Option<String>,
    limit: usize,
}

/// The last commit list read, kept between refreshes: the log of HEAD only changes when HEAD does, and reading it
/// again is the slowest part of a refresh in a long history. Only what can't change for the same key is kept; refs
/// and relative dates are worked out again on every refresh.
#[derive(Default)]
pub struct LogCache {
    key: Option<LogKey>,
    commits: Vec<Commit>,
    graph: Vec<graph::Row>,
    /// Which of them are on a main branch, and the main branch tips (ref and commit) that was worked out against.
    merged: Option<(Tips, HashSet<String>)>,
}

/// Each main ref with the commit it pointed at.
type Tips = Vec<(String, String)>;

/// Where each of the main refs points, from the listings already read; None when one isn't in them.
fn main_tips(main: &[String], local: &[LocalBranch], remote: &[RemoteBranch]) -> Option<Vec<(String, String)>> {
    main.iter()
        .map(|r| {
            let sha = if let Some(name) = r.strip_prefix("refs/heads/") {
                local.iter().find(|b| b.name == name).map(|b| b.sha.clone())
            } else {
                remote.iter().find(|b| format!("refs/remotes/{}/{}", b.remote, b.branch) == *r).map(|b| b.sha.clone())
            };
            sha.map(|sha| (r.clone(), sha))
        })
        .collect()
}

/// Each commit's refs, as git's %D lists them ("HEAD -> main", "main", "origin/main", "origin/HEAD", "tag: v1"),
/// from the refs the snapshot has already read.
fn decorate(commits: &mut [Commit], snap: &Snapshot, remote_heads: &[(String, String)]) {
    let mut by_sha: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut add = |sha: &str, name: String| by_sha.entry(sha.to_owned()).or_default().push(name);
    match (&snap.branch, &snap.head) {
        (Some(b), Some(head)) => add(head, format!("HEAD -> {b}")),
        (None, Some(head)) => add(head, "HEAD".to_owned()),
        _ => {}
    }
    for b in &snap.local_branches {
        if !b.current {
            add(&b.sha, b.name.clone());
        }
    }
    for b in &snap.remote_branches {
        add(&b.sha, b.name.clone());
    }
    for (name, sha) in remote_heads {
        add(sha, name.clone());
    }
    for t in &snap.tags {
        add(&t.sha, format!("tag: {}", t.name));
    }
    for c in commits {
        c.refs = by_sha.remove(&c.sha).unwrap_or_default();
    }
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
        Self::load_cached(git, settings, search, &mut LogCache::default())
    }

    /// Everything at once. Each query is a git process, and starting many on Windows is slow, so the queries run in
    /// parallel, few of them wait on another, and nothing is read twice. The commit list comes from `cache` when HEAD
    /// hasn't moved.
    pub fn load_cached(git: &Git, settings: &Settings, search: Option<&str>, cache: &mut LogCache) -> Result<Snapshot> {
        // Fetched once here rather than raced for by the threads below.
        git.git_dir()?;
        let head_now = match queries::head_from_files(git) {
            Some(sha) => Some(sha),
            None => queries::head_sha(git)?,
        };
        let key = head_now.clone().map(|head| LogKey { head, search: search.map(str::to_owned), limit: settings.commit_limit });
        let reuse = key.is_some() && cache.key == key && !queries::history_can_change_in_place(git);
        std::thread::scope(|s| {
            let status = s.spawn(|| status::status(git));
            let branches = s.spawn(|| queries::read_local_branches(git, settings.branch_sort));
            let remotes = s.spawn(|| queries::remotes(git));
            let remote_refs = s.spawn(|| queries::read_remote_branches(git));
            let tags = s.spawn(|| queries::tags(git));
            let worktrees = s.spawn(|| queries::worktrees(git));
            let main = s.spawn(|| queries::main_refs(git, &settings.main_branches));
            let unpushed = s.spawn(|| queries::unpushed(git));
            // From the commit just read rather than "HEAD", so the list is exactly the one `key` names.
            let commits = (!reuse).then(|| {
                s.spawn(|| queries::log(git, &LogQuery { limit: settings.commit_limit, rev: head_now.clone(), search: search.map(str::to_owned), undecorated: true, ..Default::default() }))
            });
            // Undo looks further back than the Reflog tab shows, and so does the branch order.
            let reflog = s.spawn(|| queries::reflog(git, settings.reflog_limit.max(1000)));
            let todo = s.spawn(|| rebase::todo_steps(git));
            let stashes = s.spawn(|| queries::stashes(git));
            let bisect = s.spawn(|| queries::bisect_state(git));
            let operation = queries::operation(git)?;

            let (mut commits, graph) = match commits {
                Some(handle) => {
                    let commits = join(handle)?;
                    let graph = if search.is_some() { Vec::new() } else { graph::layout(&commits) };
                    (commits, graph)
                }
                None => {
                    let mut commits = std::mem::take(&mut cache.commits);
                    // As git would word them now.
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                    for c in &mut commits {
                        c.rel_date = queries::relative_date(c.author_time, now);
                    }
                    (commits, std::mem::take(&mut cache.graph))
                }
            };
            let main = join(main)?;
            let mut local_branches = join(branches)?;
            let remotes = join(remotes)?;
            let remote_refs = join(remote_refs)?;
            let remote_branches = queries::parse_remote_branches(&remote_refs, &remotes);
            let remote_heads = queries::parse_remote_heads(&remote_refs);
            // The one query that needs another's result: which listed commits are on a main branch. Known already when
            // neither the list nor any main branch tip moved. Asked from the tips just read, so the answer matches them.
            let tips = main_tips(&main, &local_branches, &remote_branches);
            let known = cache.merged.take().filter(|(was, _)| reuse && tips.as_ref() == Some(was));
            let merged = match (known, tips) {
                (Some((tips, merged)), _) => s.spawn(move || Ok((tips, merged))),
                (None, Some(tips)) => {
                    let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
                    s.spawn(move || {
                        let tip_shas: Vec<String> = tips.iter().map(|(_, sha)| sha.clone()).collect();
                        queries::reachable_from(git, &shas, &tip_shas).map(|m| (tips, m))
                    })
                }
                // A main ref the listings don't have (it moved or went in between): ask by name, and keep nothing.
                (None, None) => {
                    let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
                    let main = main.clone();
                    s.spawn(move || queries::reachable_from(git, &shas, &main).map(|m| (Vec::new(), m)))
                }
            };
            let mut reflog = join(reflog)?;
            let undo = undo::plan_undo(&reflog);
            if settings.branch_sort == BranchSort::Recency {
                queries::sort_by_checkouts(&mut local_branches, reflog.iter().map(|e| e.subject.as_str()));
            }
            reflog.truncate(settings.reflog_limit);
            let head = head_now.clone();
            // The branch is the one marked current.
            let branch = match local_branches.iter().find(|b| b.current) {
                Some(b) => Some(b.name.clone()),
                // No commits yet (the branch doesn't exist until there are), or detached.
                None if head.is_none() => queries::current_branch(git)?,
                None => None,
            };
            let (tips, merged) = join(merged)?;
            let mut snap = Snapshot {
                branch,
                head,
                operation,
                status: join(status)?,
                local_branches,
                remotes,
                remote_branches,
                tags: join(tags)?,
                worktrees: join(worktrees)?,
                graph: Vec::new(),
                merged,
                commits: Vec::new(),
                unpushed: join(unpushed)?,
                reflog,
                stashes: join(stashes)?,
                todo: join(todo)?,
                undo,
                bisect: join(bisect)?,
            };
            decorate(&mut commits, &snap, &remote_heads);
            cache.key = key;
            cache.commits = commits.clone();
            cache.graph = graph.clone();
            cache.merged = (!tips.is_empty() || main.is_empty()).then(|| (tips, snap.merged.clone()));
            snap.commits = commits;
            snap.graph = graph;
            Ok(snap)
        })
    }
}
