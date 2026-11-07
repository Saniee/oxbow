//! The real app, driven headless through its accessibility tree, against throwaway repositories.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxbow_engine::Git;

use crate::app::App;

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

/// main: "Initial commit" (main.rs), "Add README" (with a body), tagged v0.1. A modified main.rs, an untracked
/// notes.txt and one stash.
fn demo_repo() -> TempRepo {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oxbow-ui-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = Git::new(&dir);
    let run = |args: &[&str]| {
        git.run(args).unwrap();
    };
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    run(&["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        run(&["config", k, v]);
    }
    write("main.rs", "fn main() {\n    println!(\"hi\");\n}\n");
    run(&["add", "."]);
    run(&["commit", "-qm", "Initial commit"]);
    write("README.md", "# Demo\n");
    run(&["add", "."]);
    run(&["commit", "-qm", "Add README", "-m", "Explains the demo."]);
    run(&["tag", "v0.1"]);
    write("README.md", "# Demo\nstashed\n");
    run(&["stash", "push", "-q", "-m", "readme tweak"]);
    write("main.rs", "fn main() {\n    println!(\"hello, world\");\n}\n");
    write("notes.txt", "notes\n");
    TempRepo { dir, git }
}

fn harness(path: &Path) -> Harness<'static, App> {
    let path = path.to_owned();
    Harness::builder().with_size(eframe::egui::vec2(1280.0, 820.0)).build_eframe(move |cc| App::new(cc.egui_ctx.clone(), Some(&path)))
}

/// Steps the UI until `ready` holds; git runs on other threads, so this waits in real time.
fn wait(h: &mut Harness<'static, App>, what: &str, ready: impl Fn(&Harness<'static, App>) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        h.step();
        if ready(h) {
            h.step();
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let log: Vec<String> = h.state().command_log_text();
    panic!("timed out waiting for {what}; command log: {log:#?}");
}

fn shows(h: &Harness<'static, App>, text: &str) -> bool {
    h.query_by_label_contains(text).is_some()
}

fn wait_for_text(h: &mut Harness<'static, App>, text: &str) {
    let text = text.to_owned();
    wait(h, &format!("\"{text}\""), move |h| shows(h, &text));
}

/// Waits until the button labelled `label` can be clicked (the UI has caught up with git), then clicks it.
fn click_when_enabled(h: &mut Harness<'static, App>, label: &str) {
    let l = label.to_owned();
    wait(h, &format!("\"{label}\" to be enabled"), move |h| h.query_by_label(&l).is_some_and(|n| !n.accesskit_node().is_disabled()));
    // Like a real mouse: over the button first, then click.
    h.get_by_label(label).hover();
    h.run_steps(2);
    h.get_by_label(label).click();
}

fn status(repo: &TempRepo) -> String {
    repo.git.run(["status", "--porcelain"]).unwrap()
}

#[test]
fn shows_a_files_diff_when_selected() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "notes.txt");
    h.get_by_label_contains("main.rs").click();
    wait_for_text(&mut h, "hello, world");
    assert!(shows(&h, "Unstaged"));
}

#[test]
fn shows_a_commits_message_and_patch() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "Add README");
    h.get_by_label_contains("Add README").click();
    wait_for_text(&mut h, "Explains the demo.");
    assert!(shows(&h, "README.md"));
}

#[test]
fn stages_from_the_context_menu_and_commits() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "notes.txt");
    h.get_by_label_contains("notes.txt").click_secondary();
    h.run_steps(2);
    h.get_by_label("Stage").click();
    wait(&mut h, "notes.txt to be staged", |_| status(&repo).contains("A  notes.txt"));

    h.get_by_role(Role::MultilineTextInput).click();
    h.run_steps(2);
    h.get_by_role(Role::MultilineTextInput).type_text("Add notes");
    h.run_steps(2);
    click_when_enabled(&mut h, "Commit");
    wait(&mut h, "the commit", |_| repo.git.run(["log", "-1", "--format=%s"]).unwrap().trim() == "Add notes");
    // The message box is cleared once the commit is made.
    wait(&mut h, "the message box to clear", |h| h.get_by_role(Role::MultilineTextInput).value().unwrap_or_default().is_empty());
    assert!(!status(&repo).contains("notes.txt"));
}

#[test]
fn stage_all_and_unstage_all() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "notes.txt");
    click_when_enabled(&mut h, "Stage all");
    wait(&mut h, "everything staged", |_| status(&repo) == "M  main.rs\nA  notes.txt\n");
    click_when_enabled(&mut h, "Unstage all");
    wait(&mut h, "everything unstaged", |_| status(&repo) == " M main.rs\n?? notes.txt\n");
}

#[test]
fn creates_and_checks_out_a_branch() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "notes.txt");
    h.get_by_label("New branch").click();
    h.run_steps(3);
    // The dialog's name field has focus.
    h.event(eframe::egui::Event::Text("topic".into()));
    h.run_steps(2);
    h.get_by_label("OK").click();
    wait(&mut h, "the branch", |_| repo.git.run(["branch", "--show-current"]).unwrap().trim() == "topic");
    wait_for_text(&mut h, "HEAD -> topic");
}

#[test]
fn drops_a_stash_after_confirming() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "readme tweak");
    h.get_by_label_contains("readme tweak").click_secondary();
    h.run_steps(2);
    h.get_by_label("Drop…").click();
    h.run_steps(3);
    // Nothing happens until the dialog is confirmed.
    assert_eq!(oxbow_engine::queries::stashes(&repo.git).unwrap().len(), 1);
    h.get_by_label("Drop").click();
    wait(&mut h, "the stash to be dropped", |_| oxbow_engine::queries::stashes(&repo.git).unwrap().is_empty());
    wait_for_text(&mut h, "No stashes");
}

#[test]
fn failed_actions_show_their_error() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "notes.txt");
    // There is no remote, so fetching fails, and says why.
    h.get_by_label("Push").click();
    wait_for_text(&mut h, "no remote");
    assert!(shows(&h, "Dismiss"));
}

// git runs this test binary as its sequence editor during rebases.
#[ctor::ctor(unsafe)]
fn editor_hook() {
    if let Some(code) = oxbow_engine::editor::run_if_requested() {
        std::process::exit(code);
    }
}

/// Subjects on HEAD, newest first.
fn subjects(repo: &TempRepo) -> Vec<String> {
    repo.git.run(["log", "--format=%s"]).unwrap().lines().map(str::to_owned).collect()
}

fn menu(h: &mut Harness<'static, App>, row: &str, entry: &str) {
    h.get_by_label_contains(row).click_secondary();
    h.run_steps(2);
    h.get_by_label(entry).click();
    h.run_steps(3);
}

#[test]
fn fixup_warns_about_published_commits_and_undo_restores_them() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "Add README");
    menu(&mut h, "Add README", "Fixup into previous");
    // Both commits are on main.
    assert!(shows(&h, "already pushed or on a main branch"));
    assert_eq!(subjects(&repo).len(), 2);
    h.get_by_label("Rewrite").click();
    wait(&mut h, "the fixup", |_| subjects(&repo) == ["Initial commit"]);
    wait(&mut h, "the list to catch up", |h| !shows(h, "Add README"));

    click_when_enabled(&mut h, "Undo");
    h.run_steps(3);
    // The dialog's button, over the toolbar's.
    h.get_all_by_label("Undo").last().unwrap().click();
    // Uncommitted changes survive both (the undo stashes them around its reset).
    wait(&mut h, "the undo", |_| subjects(&repo) == ["Add README", "Initial commit"] && status(&repo) == " M main.rs\n?? notes.txt\n");
}

#[test]
fn rewords_an_older_commit() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "Initial commit");
    menu(&mut h, "Initial commit", "Reword…");
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::A);
    h.event(eframe::egui::Event::Text("First commit".into()));
    h.run_steps(2);
    h.get_by_label("OK").click();
    wait(&mut h, "the reword", |_| subjects(&repo) == ["Add README", "First commit"]);
    wait_for_text(&mut h, "First commit");
}

#[test]
fn ctrl_click_selects_several_commits() {
    let repo = demo_repo();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "Add README");
    h.get_by_label_contains("Add README").click();
    h.run_steps(2);
    h.get_by_label_contains("Initial commit").click_modifiers(eframe::egui::Modifiers::COMMAND);
    wait_for_text(&mut h, "2 selected");
    h.get_by_label_contains("Add README").click_secondary();
    h.run_steps(2);
    assert!(shows(&h, "Drop 2 commits…"));
    assert!(shows(&h, "Cherry-pick 2 commits onto HEAD"));
    // The root commit has nothing to squash into.
    assert!(h.get_by_label("Squash 2 commits into their previous").accesskit_node().is_disabled());
}

#[test]
fn interactive_rebase_steps_can_be_changed_while_paused() {
    let repo = demo_repo();
    repo.git.run(["branch", "base", "HEAD~1"]).unwrap();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "   base");
    h.get_by_label("   base").click_secondary();
    h.run_steps(2);
    h.get_by_label("Rebase main onto this interactively").click();
    wait_for_text(&mut h, "Rebase steps still to apply (1)");
    menu(&mut h, "Add README", "Drop");
    wait(&mut h, "the step to be dropped", |h| shows(h, "drop"));
    click_when_enabled(&mut h, "Continue");
    // The rebase stashed the uncommitted changes and puts them back as it finishes.
    wait(&mut h, "the rebase to finish", |_| {
        oxbow_engine::queries::operation(&repo.git).unwrap().is_none() && status(&repo) == " M main.rs\n?? notes.txt\n"
    });
    assert_eq!(subjects(&repo), ["Initial commit"]);
}

#[test]
fn long_diffs_lay_out_only_what_is_in_view_and_lock_files_start_collapsed() {
    let repo = demo_repo();
    let long: String = (1..=1500).map(|n| format!("row number {n}\n")).collect();
    std::fs::write(repo.dir.join("long.txt"), &long).unwrap();
    std::fs::write(repo.dir.join("Cargo.lock"), "[[package]]\nname = \"locked-crate\"\n").unwrap();
    let mut h = harness(&repo.dir);
    wait_for_text(&mut h, "long.txt");
    h.get_by_label_contains("long.txt").click();
    wait_for_text(&mut h, "row number 1\n");
    // The end of the file is far below the view, so it is not laid out.
    assert!(!shows(&h, "row number 1400"));

    h.get_by_label_contains("Cargo.lock").click();
    wait_for_text(&mut h, "(generated)");
    assert!(!shows(&h, "locked-crate"));
}
