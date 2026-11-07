//! End-to-end checks against real repositories in the temp directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use oxbow_engine::ops::{self, StashKind};
use oxbow_engine::queries::{self, Operation};
use oxbow_engine::{CommandLog, Git, Settings, Snapshot, status};

struct TempRepo {
    dir: PathBuf,
    git: Git,
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn repo() -> TempRepo {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oxbow-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = Git::new(&dir);
    git.run(["init", "-q", "-b", "main"]).unwrap();
    for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        git.run(["config", k, v]).unwrap();
    }
    TempRepo { dir, git }
}

impl TempRepo {
    fn write(&self, path: &str, content: &str) {
        let file = self.dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    fn commit_file(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git.run(["add", path]).unwrap();
        ops::commit(&self.git, message, false).unwrap();
    }
}

#[test]
fn empty_repository_has_no_commits() {
    let r = repo();
    let snap = Snapshot::load(&r.git, &Settings::default(), None).unwrap();
    assert!(snap.commits.is_empty());
    assert_eq!(snap.head, None);
    assert_eq!(snap.branch.as_deref(), Some("main"));
}

#[test]
fn stage_commit_and_amend() {
    let r = repo();
    r.write("a.txt", "one\n");
    let entries = status::status(&r.git).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_untracked());

    // Unstaging before the first commit has no HEAD to restore from.
    status::stage(&r.git, &entries).unwrap();
    status::unstage(&r.git, &status::status(&r.git).unwrap()).unwrap();
    assert!(status::status(&r.git).unwrap()[0].is_untracked());

    status::stage(&r.git, &entries).unwrap();
    let staged = status::status(&r.git).unwrap();
    assert!(staged[0].is_staged());
    let diff = status::working_diff(&r.git, &staged[0], true).unwrap();
    assert!(diff.contains("+one"));

    assert!(ops::commit(&r.git, "  ", false).is_err());
    ops::commit(&r.git, "First\n\nBody text", false).unwrap();
    assert!(status::status(&r.git).unwrap().is_empty());
    assert_eq!(queries::commit_message(&r.git, "HEAD").unwrap(), "First\n\nBody text");

    r.write("a.txt", "one\ntwo\n");
    status::stage(&r.git, &status::status(&r.git).unwrap()).unwrap();
    ops::commit(&r.git, "", true).unwrap();
    let snap = Snapshot::load(&r.git, &Settings::default(), None).unwrap();
    assert_eq!(snap.commits.len(), 1);
    assert_eq!(snap.commits[0].subject, "First");
}

#[test]
fn discard_restores_and_deletes() {
    let r = repo();
    r.commit_file("a.txt", "one\n", "First");
    r.write("a.txt", "changed\n");
    r.write("new.txt", "new\n");
    status::discard(&r.git, &status::status(&r.git).unwrap(), false).unwrap();
    assert!(status::status(&r.git).unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(r.dir.join("a.txt")).unwrap(), "one\n");
    assert!(!r.dir.join("new.txt").exists());
}

#[test]
fn branches_marks_and_search() {
    let r = repo();
    r.commit_file("a.txt", "1\n", "On main");
    ops::create_branch(&r.git, "feature", "HEAD", true).unwrap();
    r.commit_file("b.txt", "2\n", "Feature work");
    let snap = Snapshot::load(&r.git, &Settings::default(), None).unwrap();
    assert_eq!(snap.branch.as_deref(), Some("feature"));
    assert_eq!(snap.local_branches[0].name, "feature");
    assert!(snap.local_branches[0].current);
    assert_eq!(snap.commits.len(), 2);
    assert_eq!(snap.mark(&snap.commits[1].sha), oxbow_engine::CommitMark::Merged);
    assert_eq!(snap.mark(&snap.commits[0].sha), oxbow_engine::CommitMark::Unpushed);
    assert!(snap.commits[0].refs.iter().any(|r| r == "HEAD -> feature"));

    let found = Snapshot::load(&r.git, &Settings::default(), Some("feature w")).unwrap();
    assert_eq!(found.commits.len(), 1);

    ops::checkout(&r.git, "main").unwrap();
    ops::merge(&r.git, "feature").unwrap();
    ops::delete_branch(&r.git, "feature", false).unwrap();
    let snap = Snapshot::load(&r.git, &Settings::default(), None).unwrap();
    assert_eq!(snap.local_branches.len(), 1);
    // Checkouts are in the reflog, newest first.
    assert!(snap.reflog.iter().any(|e| e.subject.starts_with("checkout: moving from feature to main")));
}

#[test]
fn stash_round_trip() {
    let r = repo();
    r.commit_file("a.txt", "1\n", "First");
    r.write("a.txt", "2\n");
    r.write("untracked.txt", "u\n");
    ops::stash_push(&r.git, "wip", StashKind::IncludeUntracked).unwrap();
    assert!(status::status(&r.git).unwrap().is_empty());
    let stashes = queries::stashes(&r.git).unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("wip"));
    let patch = queries::stash_patch(&r.git, &stashes[0]).unwrap();
    assert!(patch.contains("+2") && patch.contains("+u"));
    ops::stash_pop(&r.git, &stashes[0].selector).unwrap();
    assert_eq!(status::status(&r.git).unwrap().len(), 2);
    assert!(queries::stashes(&r.git).unwrap().is_empty());
}

#[test]
fn conflicting_merge_is_an_operation_that_can_be_aborted() {
    let r = repo();
    r.commit_file("a.txt", "base\n", "Base");
    ops::create_branch(&r.git, "other", "HEAD", true).unwrap();
    r.commit_file("a.txt", "other\n", "Other");
    ops::checkout(&r.git, "main").unwrap();
    r.commit_file("a.txt", "main\n", "Main");
    assert!(ops::merge(&r.git, "other").is_err());
    assert_eq!(queries::operation(&r.git).unwrap(), Some(Operation::Merge));
    assert!(status::status(&r.git).unwrap()[0].is_conflict());
    ops::abort_operation(&r.git, Operation::Merge).unwrap();
    assert_eq!(queries::operation(&r.git).unwrap(), None);
}

#[test]
fn command_log_lists_only_changes() {
    let r = repo();
    let log = Arc::new(CommandLog::default());
    let git = r.git.clone().with_log(log.clone());
    let id = log.start("Committing…");
    r.write("a.txt", "1\n");
    status::stage(&git, &status::status(&git).unwrap()).unwrap();
    ops::commit(&git, "First", false).unwrap();
    log.finish(id, true);
    let entries = log.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].commands, ["git --literal-pathspecs add -A -- a.txt", "git commit --cleanup=strip -F -"]);
}

#[test]
fn bisect_finds_the_first_bad_commit() {
    let r = repo();
    // "bug" appears in commit 6 of 10.
    for n in 1..=10 {
        let content = if n >= 6 { format!("{n} bug") } else { n.to_string() };
        r.commit_file("app.txt", &content, &format!("Commit {n}"));
    }
    let sha = |rev: &str| r.git.run(["rev-parse", rev]).unwrap().trim().to_owned();
    let first_bad = sha("HEAD~4");
    assert_eq!(queries::bisect_state(&r.git).unwrap(), None);
    ops::bisect_mark(&r.git, &sha("HEAD"), ops::BisectMark::Bad).unwrap();
    ops::bisect_mark(&r.git, &sha("HEAD~9"), ops::BisectMark::Good).unwrap();
    let mut rounds = 0;
    loop {
        let state = queries::bisect_state(&r.git).unwrap().unwrap();
        if let Some(culprit) = state.culprit {
            assert_eq!(culprit, first_bad);
            break;
        }
        assert!(state.remaining.unwrap() > 0);
        let bad = std::fs::read_to_string(r.dir.join("app.txt")).unwrap().contains("bug");
        ops::bisect_mark(&r.git, &sha("HEAD"), if bad { ops::BisectMark::Bad } else { ops::BisectMark::Good }).unwrap();
        rounds += 1;
        assert!(rounds < 6, "bisect should take about log2(9) steps");
    }
    ops::bisect_reset(&r.git).unwrap();
    assert_eq!(queries::bisect_state(&r.git).unwrap(), None);
    assert_eq!(queries::current_branch(&r.git).unwrap().as_deref(), Some("main"));
}

#[test]
fn worktrees_are_added_and_removed() {
    let r = repo();
    r.commit_file("a.txt", "a", "First");
    let dir = r.dir.with_file_name(format!("{}-wt", r.dir.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.to_string_lossy().replace('\\', "/");
    ops::worktree_add(&r.git, &path, "topic", Some("HEAD")).unwrap();
    let trees = queries::worktrees(&r.git).unwrap();
    assert_eq!(trees.len(), 2);
    assert!(trees.iter().any(|w| w.branch.as_deref() == Some("topic") && !w.main));
    ops::worktree_remove(&r.git, &path, false).unwrap();
    assert_eq!(queries::worktrees(&r.git).unwrap().len(), 1);
    assert!(!dir.exists());
}
