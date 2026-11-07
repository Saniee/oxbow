//! Undo and redo from the HEAD reflog, modelled on lazygit's.
//!
//! The reflog is replayed oldest to newest as a stack of actions. Oxbow's own undo and redo steps are recorded with
//! GIT_REFLOG_ACTION set to the markers below, so a later replay knows which actions are already undone. A new
//! action after an undo clears the redo stack.

use crate::git::{Exec, Git, GitError, Result};
use crate::queries::{self, ReflogEntry};

pub const UNDO_MARKER: &str = "[oxbow undo]";
pub const REDO_MARKER: &str = "[oxbow redo]";
/// Written by SlothGit and CodeGit, Oxbow's predecessors, and still understood.
const OLD_UNDO_MARKERS: [&str; 2] = ["[slothgit undo]", "[codegit undo]"];
const OLD_REDO_MARKERS: [&str; 2] = ["[slothgit redo]", "[codegit redo]"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoKind {
    /// Switching between branches or commits.
    Checkout,
    /// A commit or amend, undone with a soft reset so its changes come back staged.
    Commit,
    /// Anything else that moved the current branch (reset, rebase, merge, pull…).
    Reset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoAction {
    pub kind: UndoKind,
    /// State before the action: a branch name or commit for checkouts, a commit for resets.
    pub from: String,
    /// State after the action.
    pub to: String,
    /// The reflog message of the action.
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoPlan {
    pub undo: Option<UndoAction>,
    pub redo: Option<UndoAction>,
    /// The newest entries belong to a rebase that hasn't finished.
    pub rebase_in_progress: bool,
}

/// "checkout: moving from <a> to <b>"
fn checkout(subject: &str) -> Option<(&str, &str)> {
    let rest = subject.strip_prefix("checkout: moving from ")?;
    let (from, to) = rest.split_once(" to ")?;
    (!from.contains(' ') && !to.contains(' ')).then_some((from, to))
}

/// The part before the first colon mentions a rebase: "rebase (start)", "rebase -i (finish)", "pull --rebase (start)"…
fn rebase_phase(subject: &str) -> Option<&str> {
    let head = subject.split(':').next().unwrap_or("");
    if !head.split_whitespace().any(|w| w == "rebase" || w == "--rebase") {
        return None;
    }
    ["(start)", "(finish)", "(abort)"].into_iter().find(|p| head.contains(p))
}

/// `entries` is the HEAD reflog, newest first.
pub fn plan_undo(entries: &[ReflogEntry]) -> UndoPlan {
    let mut done: Vec<UndoAction> = Vec::new();
    let mut redo: Vec<UndoAction> = Vec::new();
    let mut rebase_from: Option<String> = None;
    let mut in_rebase = false;
    let mut previous: Option<&str> = None;

    for entry in entries.iter().rev() {
        let subject = entry.subject.as_str();
        let record = |done: &mut Vec<UndoAction>, redo: &mut Vec<UndoAction>, action: UndoAction| {
            if action.from != action.to {
                done.push(action);
                redo.clear();
            }
        };
        if subject.starts_with(UNDO_MARKER) || OLD_UNDO_MARKERS.iter().any(|m| subject.starts_with(m)) {
            if let Some(a) = done.pop() {
                redo.push(a);
            }
        } else if subject.starts_with(REDO_MARKER) || OLD_REDO_MARKERS.iter().any(|m| subject.starts_with(m)) {
            if let Some(a) = redo.pop() {
                done.push(a);
            }
        } else if in_rebase {
            // Everything between start and finish is one action.
            if matches!(rebase_phase(subject), Some("(finish)" | "(abort)")) {
                in_rebase = false;
                if let Some(from) = rebase_from.take() {
                    record(&mut done, &mut redo, UndoAction { kind: UndoKind::Reset, from, to: entry.sha.clone(), description: "rebase".into() });
                }
            }
        } else if rebase_phase(subject) == Some("(start)") {
            in_rebase = true;
            rebase_from = previous.map(str::to_owned);
        } else if let Some((from, to)) = checkout(subject) {
            record(&mut done, &mut redo, UndoAction { kind: UndoKind::Checkout, from: from.into(), to: to.into(), description: subject.into() });
        } else if let Some(prev) = previous {
            let kind = if subject.starts_with("commit") { UndoKind::Commit } else { UndoKind::Reset };
            record(&mut done, &mut redo, UndoAction { kind, from: prev.into(), to: entry.sha.clone(), description: subject.into() });
        }
        previous = Some(&entry.sha);
    }
    UndoPlan { undo: done.pop(), redo: redo.pop(), rebase_in_progress: in_rebase }
}

pub fn load_plan(git: &Git) -> Result<UndoPlan> {
    Ok(plan_undo(&queries::reflog(git, 1000)?))
}

/// What undoing (or redoing) `action` will do, for its confirmation.
pub fn describe(action: &UndoAction, redo: bool, branch: Option<&str>) -> String {
    let target = if redo { &action.to } else { &action.from };
    let short = if target.len() == 40 && target.bytes().all(|b| b.is_ascii_hexdigit()) { &target[..7] } else { target.as_str() };
    let branch = branch.unwrap_or("HEAD");
    match (action.kind, redo) {
        (UndoKind::Checkout, _) => format!("This checks out {short}."),
        (UndoKind::Commit, false) => format!("This moves {branch} back to {short} and leaves the commit's changes staged."),
        (UndoKind::Commit, true) => format!("This moves {branch} to {short} again. Your files are not touched."),
        (UndoKind::Reset, _) => format!("This resets {branch} to {short}. Uncommitted changes are kept."),
    }
}

/// Reverts (undo) or re-applies (redo) `action`. Uncommitted changes are carried over.
pub fn apply(git: &Git, action: &UndoAction, redo: bool) -> Result<()> {
    if let Some(op) = queries::operation(git)? {
        return Err(GitError::user(format!("Finish or abort the {} in progress before undoing.", op.name())));
    }
    let marker = if redo { REDO_MARKER } else { UNDO_MARKER };
    let env = [("GIT_REFLOG_ACTION", marker)];
    let exec = Exec { env: &env, ..Default::default() };
    let target = if redo { &action.to } else { &action.from };
    match action.kind {
        UndoKind::Checkout => {
            git.exec(["checkout", target, "--"], exec)?;
        }
        UndoKind::Commit => {
            // Moves the branch only: undoing leaves the commit's changes staged, redoing commits them again.
            git.exec(["reset", "--soft", target], exec)?;
        }
        UndoKind::Reset => {
            let dirty = !git.run(["status", "--porcelain"])?.trim().is_empty();
            if dirty {
                git.run(["stash", "push", "--include-untracked", "-m", "oxbow: autostash for undo"])?;
            }
            let reset = git.exec(["reset", "--hard", target], exec);
            if dirty && git.exec(["stash", "pop", "--index"], Exec { allowed: &[1], ..Default::default() })?.code != 0 {
                return Err(GitError::user("Your uncommitted changes conflict with this. They were kept in the stash (stash@{0})."));
            }
            reset?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(items: &[(&str, &str)]) -> Vec<ReflogEntry> {
        // Given oldest first, returned newest first like the reflog.
        items
            .iter()
            .rev()
            .map(|(sha, subject)| ReflogEntry { sha: (*sha).into(), short_sha: String::new(), selector: String::new(), rel_date: String::new(), subject: (*subject).into() })
            .collect()
    }

    #[test]
    fn undoes_and_redoes_in_order() {
        let log = [("a", "commit (initial): one"), ("b", "commit: two"), ("c", "checkout: moving from main to dev"), ("d", "reset: moving to HEAD~1")];
        let plan = plan_undo(&entries(&log));
        assert_eq!(plan.undo.as_ref().unwrap().kind, UndoKind::Reset);
        assert_eq!(plan.undo.as_ref().unwrap().from, "c");
        assert_eq!(plan.redo, None);

        let mut undone = log.to_vec();
        undone.push(("c", "[oxbow undo]: updating HEAD"));
        let plan = plan_undo(&entries(&undone));
        assert_eq!(plan.undo.as_ref().unwrap().kind, UndoKind::Checkout);
        assert_eq!(plan.redo.as_ref().unwrap().to, "d");

        // A new action clears the redo stack.
        undone.push(("e", "commit: three"));
        assert_eq!(plan_undo(&entries(&undone)).redo, None);
    }

    #[test]
    fn a_rebase_is_one_action() {
        let log = [
            ("a", "commit: one"),
            ("b", "commit: two"),
            ("a", "rebase (start): checkout HEAD~1"),
            ("x", "rebase (pick): two"),
            ("x", "rebase (finish): returning to refs/heads/main"),
        ];
        let plan = plan_undo(&entries(&log));
        let undo = plan.undo.unwrap();
        assert_eq!((undo.kind, undo.from.as_str(), undo.to.as_str()), (UndoKind::Reset, "b", "x"));
        assert!(plan_undo(&entries(&log[..4])).rebase_in_progress);
    }
}
