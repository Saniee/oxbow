//! History rewriting against real repositories in the temp directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use oxbow_engine::queries::{self, Operation};
use oxbow_engine::rebase::{self, Rewrite, UpdateRefs};
use oxbow_engine::todo::{Direction, StepKind, TodoAction, TodoPlan};
use oxbow_engine::{Git, ops, undo};

// git runs this test binary as its sequence editor during rebases.
#[ctor::ctor(unsafe)]
fn editor_hook() {
    if let Some(code) = oxbow_engine::editor::run_if_requested() {
        std::process::exit(code);
    }
}

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

/// main with commits one…four, each adding a file of that name.
fn repo() -> TempRepo {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oxbow-history-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = Git::new(&dir);
    git.run(["init", "-q", "-b", "main"]).unwrap();
    for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        git.run(["config", k, v]).unwrap();
    }
    let r = TempRepo { dir, git };
    for name in ["one", "two", "three", "four"] {
        r.commit_file(name, name, name);
    }
    r
}

impl TempRepo {
    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.dir.join(path), content).unwrap();
    }

    fn commit_file(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.git.run(["add", path]).unwrap();
        ops::commit(&self.git, message, false).unwrap();
    }

    /// Subjects, newest first.
    fn subjects(&self) -> Vec<String> {
        self.git.run(["log", "--format=%s"]).unwrap().lines().map(str::to_owned).collect()
    }

    fn sha(&self, rev: &str) -> String {
        self.git.run(["rev-parse", rev]).unwrap().trim().to_owned()
    }

    /// The commit whose subject is `subject`.
    fn find(&self, subject: &str) -> String {
        self.git.run(["log", "--all", "--format=%H", "--fixed-strings", &format!("--grep={subject}")]).unwrap().lines().next().unwrap().to_owned()
    }

    fn files_in(&self, rev: &str) -> Vec<String> {
        self.git.run(["show", "--format=", "--name-only", rev]).unwrap().lines().map(str::to_owned).collect()
    }

    fn operation(&self) -> Option<Operation> {
        queries::operation(&self.git).unwrap()
    }
}

#[test]
fn squashes_and_fixes_up() {
    let r = repo();
    Rewrite::new(&r.git).squash(&[r.find("three")], false).unwrap();
    assert_eq!(r.subjects(), ["four", "two", "one"]);
    // Squash keeps both messages.
    assert_eq!(queries::commit_message(&r.git, "HEAD~1").unwrap().trim(), "two\n\nthree");
    assert_eq!(r.files_in("HEAD~1"), ["three", "two"]);

    Rewrite::new(&r.git).squash(&[r.find("four")], true).unwrap();
    assert_eq!(r.subjects(), ["two", "one"]);
    assert_eq!(queries::commit_message(&r.git, "HEAD").unwrap().trim(), "two\n\nthree");
    assert_eq!(r.operation(), None);
}

#[test]
fn squashing_several_commits_at_once() {
    let r = repo();
    Rewrite::new(&r.git).squash(&[r.find("three"), r.find("four")], true).unwrap();
    assert_eq!(r.subjects(), ["two", "one"]);
    assert_eq!(r.files_in("HEAD"), ["four", "three", "two"]);
    let err = Rewrite::new(&r.git).squash(&[r.find("one")], true).unwrap_err();
    assert!(err.to_string().contains("no previous commit"));
}

#[test]
fn drops_commits() {
    let r = repo();
    Rewrite::new(&r.git).drop_commits(&[r.find("two"), r.find("four")]).unwrap();
    assert_eq!(r.subjects(), ["three", "one"]);
    assert!(!r.dir.join("two").exists());
    // The root commit too.
    Rewrite::new(&r.git).drop_commits(&[r.find("one")]).unwrap();
    assert_eq!(r.subjects(), ["three"]);
}

#[test]
fn rewords_head_and_older_commits() {
    let r = repo();
    Rewrite::new(&r.git).reword(&r.find("two"), "Second\n\nWith a body.").unwrap();
    assert_eq!(r.subjects(), ["four", "three", "Second", "one"]);
    assert_eq!(queries::commit_message(&r.git, "HEAD~2").unwrap().trim(), "Second\n\nWith a body.");
    Rewrite::new(&r.git).reword(&r.sha("HEAD"), "Fourth").unwrap();
    assert_eq!(r.subjects()[0], "Fourth");
    // The root commit, through a rebase from the root.
    Rewrite::new(&r.git).reword(&r.find("one"), "First").unwrap();
    assert_eq!(r.subjects(), ["Fourth", "three", "Second", "First"]);
    assert!(Rewrite::new(&r.git).reword(&r.sha("HEAD"), "  ").is_err());
}

#[test]
fn moves_commits_up_and_down() {
    let r = repo();
    Rewrite::new(&r.git).move_commit(&r.find("two"), Direction::Up).unwrap();
    assert_eq!(r.subjects(), ["four", "two", "three", "one"]);
    Rewrite::new(&r.git).move_commit(&r.find("four"), Direction::Down).unwrap();
    assert_eq!(r.subjects(), ["two", "four", "three", "one"]);
    assert!(Rewrite::new(&r.git).move_commit(&r.sha("HEAD"), Direction::Up).is_err());
    assert!(Rewrite::new(&r.git).move_commit(&r.find("one"), Direction::Down).is_err());
}

#[test]
fn amends_staged_changes_into_an_older_commit() {
    let r = repo();
    r.write("two", "two, amended");
    r.git.run(["add", "two"]).unwrap();
    // An unstaged change elsewhere survives (the rebase autostashes it).
    r.write("four", "four, unstaged");
    Rewrite::new(&r.git).amend_staged(&r.find("two")).unwrap();
    assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
    assert_eq!(r.git.run(["show", "HEAD~2:two"]).unwrap(), "two, amended");
    assert_eq!(r.git.run(["status", "--porcelain"]).unwrap(), " M four\n");
    assert!(Rewrite::new(&r.git).amend_staged(&r.find("two")).unwrap_err().to_string().contains("no staged changes"));
}

#[test]
fn fixup_commits_and_autosquash() {
    let r = repo();
    r.write("one", "one, fixed");
    r.git.run(["add", "one"]).unwrap();
    let one = r.find("one");
    Rewrite::new(&r.git).create_fixup(&r.find("two")).unwrap();
    assert_eq!(r.subjects()[0], "fixup! two");
    Rewrite::new(&r.git).autosquash(&one).unwrap();
    assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
    assert_eq!(r.files_in("HEAD~2"), ["one", "two"]);
}

#[test]
fn edit_stops_at_the_commit() {
    let r = repo();
    let two = r.find("two");
    Rewrite::new(&r.git).edit(&two).unwrap();
    assert_eq!(r.operation(), Some(Operation::Rebase));
    assert_eq!(r.sha("HEAD"), two);
    let steps = rebase::todo_steps(&r.git).unwrap();
    assert_eq!(steps.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(), ["three", "four"]);
    // Rewriting while one is stopped is refused.
    assert!(Rewrite::new(&r.git).squash(&[r.find("three")], true).is_err());
    ops::continue_operation(&r.git, Operation::Rebase).unwrap();
    assert_eq!(r.operation(), None);
    assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
}

#[test]
fn interactive_rebase_can_be_changed_while_paused() {
    let r = repo();
    let one = r.find("one");
    Rewrite::new(&r.git).rebase_interactive(&one).unwrap();
    assert_eq!(r.operation(), Some(Operation::Rebase));
    let steps = rebase::todo_steps(&r.git).unwrap();
    assert_eq!(steps.len(), 3);
    assert!(steps.iter().all(|s| s.kind == StepKind::Commit(TodoAction::Pick)));

    let rw = Rewrite::new(&r.git);
    rw.edit_todo(&TodoPlan { actions: vec![(r.find("three"), TodoAction::Drop)], ..Default::default() }).unwrap();
    rw.edit_todo(&TodoPlan { move_commit: Some((r.find("two"), Direction::Up)), ..Default::default() }).unwrap();
    rw.reword_todo(&r.find("four"), "Fourth").unwrap();
    let steps = rebase::todo_steps(&r.git).unwrap();
    assert_eq!(steps.iter().map(|s| (s.action().unwrap(), s.subject.as_str())).collect::<Vec<_>>(), [
        // Moving swaps with the neighbouring step, a dropped one too.
        (TodoAction::Drop, "three"),
        (TodoAction::Pick, "two"),
        (TodoAction::Reword, "four"),
    ]);
    ops::continue_operation(&r.git, Operation::Rebase).unwrap();
    assert_eq!(r.subjects(), ["Fourth", "two", "one"]);
}

#[test]
fn merges_are_kept_and_guarded() {
    let r = repo();
    r.git.run(["checkout", "-q", "-b", "side", "HEAD~2"]).unwrap();
    r.commit_file("side", "side", "side work");
    r.git.run(["checkout", "-q", "main"]).unwrap();
    r.git.run(["merge", "-q", "--no-ff", "--no-edit", "side"]).unwrap();
    r.commit_file("five", "five", "five");

    // Rewording a commit older than the merge recreates the merge.
    Rewrite::new(&r.git).reword(&r.find("two"), "Second").unwrap();
    assert_eq!(r.git.run(["rev-list", "--merges", "--count", "HEAD"]).unwrap().trim(), "1");
    assert!(r.subjects().contains(&"Second".to_owned()));
    assert_eq!(r.git.run(["log", "-1", "--format=%s", "HEAD~1"]).unwrap().trim(), "Merge branch 'side'");

    let merge = r.sha("HEAD~1");
    let err = Rewrite::new(&r.git).squash(std::slice::from_ref(&merge), true).unwrap_err();
    assert!(err.to_string().contains("merge commit"), "{err}");
    assert!(Rewrite::new(&r.git).squash(&[r.sha("HEAD")], true).is_err());
    Rewrite::new(&r.git).reword(&merge, "Merge the side").unwrap();
    assert_eq!(r.git.run(["log", "-1", "--format=%s", "HEAD~1"]).unwrap().trim(), "Merge the side");
    assert_eq!(r.git.run(["rev-list", "--merges", "--count", "HEAD"]).unwrap().trim(), "1");
}

#[test]
fn stacked_branches_move_with_update_refs() {
    let r = repo();
    r.git.run(["branch", "stacked", "HEAD~1"]).unwrap();
    Rewrite::new(&r.git).update_refs(UpdateRefs::Always).reword(&r.find("two"), "Second").unwrap();
    assert_eq!(r.git.run(["log", "-1", "--format=%s", "stacked~1"]).unwrap().trim(), "Second");
    assert_eq!(r.sha("stacked"), r.sha("HEAD~1"));
}

#[test]
fn undo_and_redo() {
    let r = repo();
    let plan = undo::load_plan(&r.git).unwrap();
    let action = plan.undo.unwrap();
    assert_eq!(action.kind, undo::UndoKind::Commit);
    // Undoing a commit leaves its changes staged.
    undo::apply(&r.git, &action, false).unwrap();
    assert_eq!(r.subjects(), ["three", "two", "one"]);
    assert_eq!(r.git.run(["status", "--porcelain"]).unwrap(), "A  four\n");
    let plan = undo::load_plan(&r.git).unwrap();
    assert_eq!(plan.redo.as_ref(), Some(&action));
    undo::apply(&r.git, &action, true).unwrap();
    assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
    assert_eq!(undo::load_plan(&r.git).unwrap().redo, None);

    // A rewrite is one step, and uncommitted changes survive undoing it.
    Rewrite::new(&r.git).squash(&[r.find("four"), r.find("three")], true).unwrap();
    r.write("one", "dirty");
    let plan = undo::load_plan(&r.git).unwrap();
    let action = plan.undo.unwrap();
    assert_eq!(action.kind, undo::UndoKind::Reset);
    undo::apply(&r.git, &action, false).unwrap();
    assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
    assert_eq!(r.git.run(["status", "--porcelain"]).unwrap(), " M one\n");

    // Checkouts too.
    r.git.run(["checkout", "-q", "-b", "dev"]).unwrap();
    let action = undo::load_plan(&r.git).unwrap().undo.unwrap();
    assert_eq!(action.kind, undo::UndoKind::Checkout);
    undo::apply(&r.git, &action, false).unwrap();
    assert_eq!(queries::current_branch(&r.git).unwrap().as_deref(), Some("main"));
}

// --- Custom patches ----------------------------------------------------------------------

use oxbow_engine::patch::{self, Picks};
use oxbow_engine::rebase::PatchTarget;

/// A commit "edit" on top of `repo()` that changes notes.txt in two places and adds extra.txt.
fn patch_repo() -> (TempRepo, String) {
    let r = repo();
    r.commit_file("notes.txt", "a\nb\nc\nd\ne\nf\ng\nh\n", "Add notes");
    r.write("notes.txt", "a\nB\nc\nd\ne\nf\ng\nH\n");
    r.write("extra.txt", "x\n");
    r.git.run(["add", "."]).unwrap();
    ops::commit(&r.git, "edit", false).unwrap();
    let sha = r.sha("HEAD");
    (r, sha)
}

/// The removal patch for the lines of `path` in `sha` whose text is in `texts` (added or removed lines).
fn removal(r: &TempRepo, sha: &str, path: &str, texts: &[&str]) -> String {
    let diff = queries::commit_patch(&r.git, sha, Some(&format!("{sha}^"))).unwrap();
    let files = oxbow_engine::diff::parse_diff(&diff);
    let file = files.iter().find(|f| f.path == path).unwrap();
    let mut picks = Picks::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        for (i, line) in hunk.lines.iter().enumerate() {
            if line.kind != oxbow_engine::diff::LineKind::Context && texts.contains(&line.text.as_str()) {
                picks.insert((h, i));
            }
        }
    }
    patch::removal_patch(file, &picks).unwrap().unwrap()
}

fn show(r: &TempRepo, rev_path: &str) -> String {
    r.git.run(["show", rev_path]).unwrap()
}

#[test]
fn lines_are_taken_out_of_a_commit() {
    let (r, sha) = patch_repo();
    let p = removal(&r, &sha, "notes.txt", &["H"]);
    Rewrite::new(&r.git).remove_patch(&sha, &p, &PatchTarget::Discard).unwrap();
    // The other change and the other file stay.
    assert_eq!(show(&r, "HEAD:notes.txt"), "a\nB\nc\nd\ne\nf\ng\nh\n");
    assert_eq!(show(&r, "HEAD:extra.txt"), "x\n");
    assert_eq!(r.subjects()[0], "edit");
    assert_eq!(r.operation(), None);
}

#[test]
fn lines_taken_out_can_go_to_the_working_tree_or_a_new_commit() {
    let (r, sha) = patch_repo();
    let p = removal(&r, &sha, "notes.txt", &["B"]);
    Rewrite::new(&r.git).remove_patch(&sha, &p, &PatchTarget::WorkingTree).unwrap();
    assert_eq!(show(&r, "HEAD:notes.txt"), "a\nb\nc\nd\ne\nf\ng\nH\n");
    assert_eq!(std::fs::read_to_string(r.dir.join("notes.txt")).unwrap(), "a\nB\nc\nd\ne\nf\ng\nH\n");

    r.git.run(["checkout", "--", "notes.txt"]).unwrap();
    let sha = r.sha("HEAD");
    let p = removal(&r, &sha, "extra.txt", &["x"]);
    Rewrite::new(&r.git).remove_patch(&sha, &p, &PatchTarget::NewCommit("Add extra".into())).unwrap();
    assert_eq!(r.subjects()[..2], ["Add extra", "edit"]);
    assert!(!r.files_in("HEAD~1").contains(&"extra.txt".to_owned()));
    assert_eq!(r.files_in("HEAD"), ["extra.txt"]);
}

#[test]
fn lines_move_into_an_older_commit() {
    let (r, sha) = patch_repo();
    let p = removal(&r, &sha, "notes.txt", &["H", "h"]);
    Rewrite::new(&r.git).move_patch(&sha, &p, &r.find("Add notes")).unwrap();
    assert_eq!(r.subjects()[..2], ["edit", "Add notes"]);
    assert_eq!(show(&r, "HEAD~1:notes.txt"), "a\nb\nc\nd\ne\nf\ng\nH\n");
    assert_eq!(show(&r, "HEAD:notes.txt"), "a\nB\nc\nd\ne\nf\ng\nH\n");
    assert_eq!(r.operation(), None);
}

#[test]
fn a_patch_applies_to_the_working_tree_both_ways() {
    let (r, sha) = patch_repo();
    let p = removal(&r, &sha, "notes.txt", &["B"]);
    Rewrite::new(&r.git).apply_patch(&sha, &p, true).unwrap();
    assert_eq!(std::fs::read_to_string(r.dir.join("notes.txt")).unwrap(), "a\nb\nc\nd\ne\nf\ng\nH\n");
    Rewrite::new(&r.git).apply_patch(&sha, &p, false).unwrap();
    assert_eq!(std::fs::read_to_string(r.dir.join("notes.txt")).unwrap(), "a\nB\nc\nd\ne\nf\ng\nH\n");
}
