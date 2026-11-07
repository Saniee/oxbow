//! Notices changes made outside Oxbow (an editor saving, git in a terminal) so the panels can refresh on their own.
//! Changes are batched until things go quiet for a moment, and ignored files (build output, say) and git's
//! internals that don't change what is shown (objects, lock files) are left out.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher as _};

use crate::git::{Exec, Git};

/// How long things must stay quiet before a batch of changes counts, and the longest a batch waits.
const QUIET: Duration = Duration::from_millis(300);
const MAX_WAIT: Duration = Duration::from_millis(1500);

/// What changed in a batch.
#[derive(Clone, Copy, Debug)]
pub struct Change {
    /// Something in the git directory (a branch moved, a commit was made); otherwise only what the status shows
    /// (files in the working tree, the index).
    pub repository: bool,
    /// When the last change of the batch happened: a refresh started after that has seen it already.
    pub last: Instant,
}

/// Watches until dropped.
pub struct Watcher {
    inner: Option<notify::RecommendedWatcher>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Dropping the notify watcher closes the channel, which ends the thread.
        self.inner = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Inside the git directory, the files whose change means something shown may have changed.
fn git_dir_change_matters(rel: &str) -> bool {
    if rel.ends_with(".lock") || rel.starts_with("objects/") || rel.contains("oxbow-message-") {
        return false;
    }
    let top = rel.split('/').next().unwrap_or(rel);
    matches!(top, "HEAD" | "index" | "ORIG_HEAD" | "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD" | "BISECT_LOG" | "packed-refs" | "refs" | "rebase-merge" | "rebase-apply" | "sequencer" | "worktrees")
        || rel == "logs/HEAD"
}

/// Calls `on_change` after files in the repository change.
pub fn watch(git: &Git, on_change: impl Fn(Change) + Send + 'static) -> Result<Watcher, String> {
    let root = git.root().to_path_buf();
    let git_dir = git.git_dir().map_err(|e| e.to_string())?.to_path_buf();
    let git_dir = git_dir.canonicalize().unwrap_or(git_dir);
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
    // None: changes were lost (the system's buffer overflowed, or watching failed for a moment), so anything may
    // have changed.
    let (tx, rx) = mpsc::channel::<Option<PathBuf>>();
    let mut inner = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
        Ok(event) if event.need_rescan() => {
            let _ = tx.send(None);
        }
        Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
            for path in event.paths {
                let _ = tx.send(Some(path));
            }
        }
        Ok(_) => {}
        Err(_) => {
            let _ = tx.send(None);
        }
    })
    .map_err(|e| e.to_string())?;
    inner.watch(&root, RecursiveMode::Recursive).map_err(|e| e.to_string())?;
    if !git_dir.starts_with(&canonical_root) {
        // A linked worktree: its git directory lives elsewhere.
        let _ = inner.watch(&git_dir, RecursiveMode::Recursive);
    }

    let probe = git.clone();
    let thread = std::thread::Builder::new()
        .name("oxbow-watch".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut paths = vec![first];
                let start = Instant::now();
                let mut last = start;
                loop {
                    match rx.recv_timeout(QUIET) {
                        Ok(p) => {
                            paths.push(p);
                            last = Instant::now();
                            if start.elapsed() > MAX_WAIT {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                let repository = if paths.iter().any(Option::is_none) {
                    Some(true)
                } else {
                    let paths: Vec<PathBuf> = paths.into_iter().flatten().collect();
                    matters(&probe, &paths, &canonical_root, &root, &git_dir)
                };
                if let Some(repository) = repository {
                    on_change(Change { repository, last });
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Watcher { inner: Some(inner), thread: Some(thread) })
}

fn relative<'a>(path: &'a Path, bases: &[&Path]) -> Option<&'a Path> {
    bases.iter().find_map(|b| path.strip_prefix(b).ok())
}

/// Whether any of the changed paths can change what Oxbow shows: Some(true) when something in the git directory
/// did, Some(false) when only files in the working tree did.
fn matters(git: &Git, paths: &[PathBuf], canonical_root: &Path, root: &Path, git_dir: &Path) -> Option<bool> {
    let mut work_tree = Vec::new();
    // Changes only the status would show: the index (staging, or a status saving file times), a submodule.
    let mut status_only = false;
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !seen.insert(path) {
            continue;
        }
        let path = if path.starts_with(root) && !path.starts_with(root.join(".git")) {
            path.clone()
        } else {
            path.canonicalize().unwrap_or_else(|_| path.clone())
        };
        if let Some(rel) = relative(&path, &[git_dir]) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel == "index" {
                status_only = true;
            } else if git_dir_change_matters(&rel) {
                return Some(true);
            }
            // A submodule's repository lives in modules/: a commit or checkout in it changes the submodule's status.
            if let Some(inner) = rel.strip_prefix("modules/")
                && inner.split('/').any(|part| matches!(part, "HEAD" | "index" | "refs" | "packed-refs"))
                && !rel.ends_with(".lock")
            {
                status_only = true;
            }
        } else if let Some(rel) = relative(&path, &[canonical_root, root]) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            // A .git directory below the root is the git directory itself, already handled, or a nested repo's.
            if !rel.is_empty() && !rel.split('/').any(|part| part == ".git") {
                work_tree.push(rel);
            }
        }
    }
    if status_only {
        return Some(false);
    }
    if work_tree.is_empty() {
        return None;
    }
    work_tree.sort();
    work_tree.dedup();
    // Ignored files (build output, say) change nothing Oxbow shows. check-ignore lists the ignored ones.
    let input = work_tree.join("\n") + "\n";
    let any = match git.exec(["check-ignore", "--stdin"], Exec { input: Some(&input), allowed: &[1], ..Default::default() }) {
        Ok(out) => out.stdout.lines().count() < work_tree.len(),
        Err(_) => true,
    };
    any.then_some(false)
}

#[cfg(test)]
mod tests {
    use super::git_dir_change_matters;

    #[test]
    fn git_internals_that_matter() {
        for rel in ["HEAD", "index", "refs/heads/main", "packed-refs", "rebase-merge/git-rebase-todo", "logs/HEAD", "MERGE_HEAD"] {
            assert!(git_dir_change_matters(rel), "{rel}");
        }
        for rel in ["index.lock", "objects/ab/cdef", "refs/heads/main.lock", "logs/refs/heads/main", "COMMIT_EDITMSG", "FETCH_HEAD", "rebase-merge/oxbow-message-abc"] {
            assert!(!git_dir_change_matters(rel), "{rel}");
        }
    }
}
