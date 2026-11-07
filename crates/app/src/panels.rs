//! The left-hand panels: Status, Files (with the commit box), Branches, Commits and Stash. Each one reads the
//! snapshot and pushes intents; nothing here runs git directly.

use eframe::egui::{self, RichText};
use oxbow_engine::ops::{self, ResetMode, StashKind};
use oxbow_engine::queries::{LocalBranch, Operation, RemoteBranch, Stash, Tag};
use oxbow_engine::status::{self, StatusEntry};
use oxbow_engine::rebase::{PatchTarget, Rewrite};
use oxbow_engine::todo::{Direction, StepKind, TodoAction, TodoPlan};
use oxbow_engine::{CommitMark, Snapshot};

use crate::app::{CherryPicks, Intent};
use crate::detail::{Request, Selection};
use crate::dialogs::{self, Dialog};
use crate::theme;
use crate::worker::action;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchesTab {
    #[default]
    Local,
    Remotes,
    Tags,
    Worktrees,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitsTab {
    #[default]
    Commits,
    Reflog,
}

#[derive(Default)]
pub struct View {
    pub selection: Option<Selection>,
    pub commit_message: String,
    pub amend: bool,
    pub branches_tab: BranchesTab,
    pub commits_tab: CommitsTab,
    pub search: String,
    /// Commits picked with Ctrl- and Shift-click, the selected one included.
    pub selected_commits: std::collections::HashSet<String>,
    /// From the preferences, refreshed every frame.
    pub confirm_rewriting_pushed: bool,
    /// Commits copied for cherry-picking, oldest first (the order they are applied in): SHA and subject.
    pub cherry_picks: Vec<(String, String)>,
    /// Lines picked for a custom patch.
    pub picking: crate::detail::Picking,
}

/// A menu entry that pushes an intent and closes the menu.
fn item(ui: &mut egui::Ui, label: &str, intents: &mut Vec<Intent>, make: impl FnOnce() -> Intent) {
    if ui.button(label).clicked() {
        intents.push(make());
        ui.close();
    }
}

fn run(title: &str, task: impl FnOnce(&oxbow_engine::Git) -> oxbow_engine::Result<()> + Send + 'static) -> Intent {
    Intent::Run(action(title, task))
}

fn copy_item(ui: &mut egui::Ui, label: &str, text: &str) {
    if ui.button(label).clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.close();
    }
}

/// A titled box filling `height`, with buttons on the right of the title and a scrolling body.
pub fn section<S>(
    ui: &mut egui::Ui,
    title: &str,
    height: f32,
    state: &mut S,
    buttons: impl FnOnce(&mut egui::Ui, &mut S),
    body: impl FnOnce(&mut egui::Ui, &mut S),
) {
    let width = ui.available_width();
    ui.allocate_ui(egui::vec2(width, height), |ui| {
        egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(width - 14.0);
            ui.set_height(height - 14.0);
            ui.horizontal(|ui| {
                ui.strong(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui, state));
            });
            ui.separator();
            body(ui, state);
        });
    });
}

/// A scrolling list of `count` one-line rows of which only those in view are built, so a list of thousands (files,
/// branches, tags) costs no more per frame than a short one.
fn virtual_rows(ui: &mut egui::Ui, salt: &str, count: usize, mut row: impl FnMut(&mut egui::Ui, usize)) {
    let height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show_rows(ui, height, count, |ui, range| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            for i in range {
                row(ui, i);
            }
        });
    });
}

fn rows(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), add);
    });
}

// --- Status ----------------------------------------------------------------------------

pub fn status(ui: &mut egui::Ui, snap: &Snapshot, root: &str, running: Option<&str>, intents: &mut Vec<Intent>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(root).weak());
        ui.separator();
        match (&snap.branch, &snap.head) {
            (Some(b), _) => {
                ui.label(RichText::new(b).strong().color(theme::palette(ui).local_ref));
            }
            (None, Some(sha)) => {
                ui.label(RichText::new(format!("detached at {}", &sha[..7.min(sha.len())])).strong());
            }
            (None, None) => {
                ui.label("no commits yet");
            }
        }
        if let Some(b) = snap.current_branch() {
            match &b.upstream {
                Some(up) if b.gone => {
                    ui.weak(format!("{} (gone)", up.name));
                }
                Some(up) => {
                    ui.weak(format!("➡ {}", up.name));
                    if b.ahead > 0 || b.behind > 0 {
                        ui.label(format!("⬆{} ⬇{}", b.ahead, b.behind));
                    }
                }
                None => {
                    ui.weak("no upstream");
                }
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Fetch").on_hover_text("git fetch --prune").clicked() {
            intents.push(run("Fetching…", |g| ops::fetch(g, false)));
        }
        ui.menu_button("⏷", |ui| {
            item(ui, "Fetch all remotes", intents, || run("Fetching all remotes…", |g| ops::fetch(g, true)));
        });
        if ui.button("Pull").on_hover_text("git pull --no-rebase").clicked() {
            intents.push(run("Pulling…", |g| ops::pull(g, false)));
        }
        ui.menu_button("⏷", |ui| {
            item(ui, "Pull (rebase)", intents, || run("Pulling with rebase…", |g| ops::pull(g, true)));
        });
        let remote = snap.default_remote().map(str::to_owned);
        if ui.button("Push").on_hover_text("git push (sets the upstream on first push)").clicked() {
            let remote = remote.clone();
            intents.push(run("Pushing…", move |g| ops::push(g, false, remote.as_deref())));
        }
        ui.menu_button("⏷", |ui| {
            item(ui, "Force push (with lease)", intents, || {
                let remote = remote.clone();
                Intent::Dialog(dialogs::confirm(
                    "Force push?",
                    "This replaces the branch on the remote with yours. --force-with-lease refuses if someone else pushed in the meantime.",
                    "Force push",
                    action("Force pushing…", move |g| ops::push(g, true, remote.as_deref())),
                ))
            });
        });
        if let Some(title) = running {
            ui.spinner();
            ui.weak(title);
        }
    });
    if let Some(b) = &snap.bisect {
        bisect_banner(ui, snap, b, intents);
    }
    if let Some(op) = snap.operation {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::palette(ui).conflict, format!("{} in progress", capitalize(op.name())));
            let conflicts = snap.status.iter().filter(|e| e.is_conflict()).count();
            if conflicts > 0 {
                ui.weak(format!("{conflicts} conflicted"));
            }
            if ui.button("Continue").clicked() {
                intents.push(run("Continuing…", move |g| ops::continue_operation(g, op)));
            }
            if op != Operation::Merge && ui.button("Skip").clicked() {
                intents.push(run("Skipping…", move |g| ops::skip_operation(g, op)));
            }
            if ui.button("Abort").clicked() {
                intents.push(Intent::Dialog(dialogs::confirm(
                    format!("Abort the {}?", op.name()),
                    "Everything goes back to how it was before it started.",
                    "Abort",
                    action("Aborting…", move |g| ops::abort_operation(g, op)),
                )));
            }
        });
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn bisect_banner(ui: &mut egui::Ui, snap: &Snapshot, b: &oxbow_engine::queries::BisectState, intents: &mut Vec<Intent>) {
    let (bad, good) = (&b.terms.0, &b.terms.1);
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(theme::palette(ui).hunk, "Bisecting");
        match (&b.culprit, &b.bad, b.good.is_empty()) {
            (Some(c), _, _) => {
                let found = ui.link(format!("First {bad} commit: {} {}", short(c), b.culprit_subject));
                if found.clicked() {
                    intents.push(Intent::Select(Selection::Commit(c.clone()), Request::Rev(c.clone())));
                }
            }
            (None, None, _) => {
                ui.weak(format!("mark a commit that is {bad}"));
            }
            (None, Some(_), true) => {
                ui.weak(format!("now mark a commit that is {good}"));
            }
            (None, Some(_), false) => {
                let steps = b.steps.map(|s| format!(", about {}", plural(s.max(1), "step"))).unwrap_or_default();
                ui.weak(format!("{} left to check{steps}. Test the checked-out commit, then mark it.", plural(b.remaining.unwrap_or(0), "commit")));
            }
        }
        if b.culprit.is_none()
            && let Some(head) = snap.head.clone()
        {
            for (label, mark) in [(bad.as_str(), ops::BisectMark::Bad), (good.as_str(), ops::BisectMark::Good), ("skip", ops::BisectMark::Skip)] {
                // HEAD rather than the SHA shown: a second click before the panels catch up marks the right commit.
                if ui.small_button(capitalize(label)).on_hover_text(format!("Marks the checked-out commit {} as {label}", short(&head))).clicked() {
                    intents.push(run("Bisecting…", move |g| ops::bisect_mark(g, "HEAD", mark)));
                }
            }
        }
        if ui.small_button("End bisect").on_hover_text(format!("git bisect reset: back to {}", b.start)).clicked() {
            intents.push(run("Ending the bisect…", ops::bisect_reset));
        }
    });
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

// --- Files -----------------------------------------------------------------------------

pub fn files(ui: &mut egui::Ui, snap: &Snapshot, view: &mut View, height: f32, intents: &mut Vec<Intent>) {
    let title = if snap.status.is_empty() { "Files".to_owned() } else { format!("Files ({})", snap.status.len()) };
    section(
        ui,
        &title,
        height,
        &mut (view, intents),
        |ui, (_, intents)| {
            ui.menu_button("Stash ⏷", |ui| {
                stash_item(ui, "Stash changes…", StashKind::Tracked, intents);
                stash_item(ui, "Stash including untracked…", StashKind::IncludeUntracked, intents);
                stash_item(ui, "Stash staged changes…", StashKind::Staged, intents);
            });
            // The lists are made only when clicked: copying thousands of entries every frame adds up.
            let any_staged = snap.status.iter().any(|e| e.is_staged());
            if ui.add_enabled(any_staged, egui::Button::new("Unstage all")).clicked() {
                let unstage: Vec<StatusEntry> = snap.status.iter().filter(|e| e.is_staged()).cloned().collect();
                intents.push(run("Unstaging all…", move |g| status::unstage(g, &unstage)));
            }
            let any_unstaged = snap.status.iter().any(|e| e.is_unstaged() || e.is_conflict());
            if ui.add_enabled(any_unstaged, egui::Button::new("Stage all")).clicked() {
                let stage: Vec<StatusEntry> = snap.status.iter().filter(|e| e.is_unstaged() || e.is_conflict()).cloned().collect();
                intents.push(run("Staging all…", move |g| status::stage(g, &stage)));
            }
        },
        |ui, (view, intents)| {
            let box_height = 96.0;
            let list_height = (ui.available_height() - box_height).max(40.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), list_height), |ui| {
                if snap.status.is_empty() {
                    ui.weak("No changes");
                    ui.allocate_space(ui.available_size());
                    return;
                }
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                virtual_rows(ui, "files", snap.status.len(), |ui, i| file_row(ui, &snap.status[i], &mono, view, intents));
            });
            commit_box(ui, snap, view, intents);
        },
    );
}

fn stash_item(ui: &mut egui::Ui, label: &str, kind: StashKind, intents: &mut Vec<Intent>) {
    item(ui, label, intents, || {
        Intent::Dialog(Dialog::Prompt {
            title: "Stash".into(),
            label: "Message (optional)".into(),
            text: String::new(),
            multiline: false,
            check: None,
            make: Some(Box::new(move |message, _| action("Stashing…", move |g| ops::stash_push(g, &message, kind)))),
            focused: false,
        })
    });
}

fn file_row(ui: &mut egui::Ui, entry: &StatusEntry, mono: &egui::FontId, view: &View, intents: &mut Vec<Intent>) {
    let selected = view.selection == Some(Selection::File(entry.path.clone()));
    let mut job = theme::status_letters(&theme::palette(ui), entry.x, entry.y, entry.is_conflict(), entry.is_untracked(), mono);
    let name = match &entry.old_path {
        Some(old) => format!("{old} ➡ {}", entry.path),
        None => entry.path.clone(),
    };
    job.append(&name, 8.0, egui::TextFormat { font_id: egui::TextStyle::Body.resolve(ui.style()), color: ui.visuals().text_color(), ..Default::default() });
    let response = ui.selectable_label(selected, job);
    let toggle = |intents: &mut Vec<Intent>| {
        let e = entry.clone();
        if e.is_staged() && !e.is_unstaged() {
            intents.push(run("Unstaging…", move |g| status::unstage(g, &[e])));
        } else {
            intents.push(run("Staging…", move |g| status::stage(g, &[e])));
        }
    };
    if response.clicked() {
        intents.push(Intent::Select(Selection::File(entry.path.clone()), Request::File(entry.clone())));
    }
    if response.double_clicked() {
        toggle(intents);
    }
    response.context_menu(|ui| {
        if entry.is_unstaged() || entry.is_conflict() {
            let e = entry.clone();
            item(ui, if entry.is_conflict() { "Mark resolved (stage)" } else { "Stage" }, intents, || run("Staging…", move |g| status::stage(g, &[e])));
        }
        if entry.is_staged() {
            let e = entry.clone();
            item(ui, "Unstage", intents, || run("Unstaging…", move |g| status::unstage(g, &[e])));
        }
        ui.separator();
        if entry.is_staged() && entry.y != ' ' && !entry.is_untracked() {
            let e = entry.clone();
            item(ui, "Discard unstaged changes", intents, || {
                Intent::Dialog(dialogs::confirm(
                    "Discard unstaged changes?",
                    format!("The unstaged changes to {} are lost. Staged changes stay.", e.path),
                    "Discard",
                    action("Discarding…", move |g| status::discard(g, &[e], true)),
                ))
            });
        }
        let e = entry.clone();
        let what = if entry.is_untracked() { format!("{} is deleted.", e.path) } else { format!("All changes to {} are lost, staged and unstaged.", e.path) };
        item(ui, "Discard…", intents, || Intent::Dialog(dialogs::confirm("Discard changes?", what, "Discard", action("Discarding…", move |g| status::discard(g, &[e], false)))));
        ui.separator();
        copy_item(ui, "Copy path", &entry.path);
    });
}

fn commit_box(ui: &mut egui::Ui, snap: &Snapshot, view: &mut View, intents: &mut Vec<Intent>) {
    ui.separator();
    let hint = if view.amend { "New message for the last commit (empty keeps it)" } else { "Commit message (Ctrl+Enter to commit)" };
    let edit = egui::TextEdit::multiline(&mut view.commit_message).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY);
    let response = ui.add(edit);
    let ctrl_enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
    ui.horizontal(|ui| {
        let staged = snap.status.iter().any(|e| e.is_staged());
        let can_amend = snap.head.is_some();
        let before = view.amend;
        ui.add_enabled(can_amend, egui::Checkbox::new(&mut view.amend, "Amend last commit"));
        if view.amend != before {
            intents.push(Intent::Amend(view.amend));
        }
        let label = if view.amend { "Amend" } else { "Commit" };
        let enabled = if view.amend { true } else { staged && !view.commit_message.trim().is_empty() };
        let clicked = ui.add_enabled(enabled, egui::Button::new(label)).on_disabled_hover_text("Stage changes and write a message first").clicked();
        if (clicked || (ctrl_enter && enabled)) && enabled {
            let message = view.commit_message.clone();
            let amend = view.amend;
            intents.push(run(if amend { "Amending…" } else { "Committing…" }, move |g| ops::commit(g, &message, amend)));
        }
    });
}

// --- Branches --------------------------------------------------------------------------

pub fn branches(ui: &mut egui::Ui, snap: &Snapshot, view: &mut View, height: f32, intents: &mut Vec<Intent>) {
    section(
        ui,
        "Branches",
        height,
        &mut (view, intents),
        |ui, (view, intents)| {
            if view.branches_tab == BranchesTab::Local && ui.button("New branch").clicked() {
                intents.push(new_branch_dialog("HEAD", ""));
            }
            if view.branches_tab == BranchesTab::Tags && snap.head.is_some() && ui.button("New tag").clicked() {
                intents.push(new_tag_dialog("HEAD".into()));
            }
        },
        |ui, (view, intents)| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut view.branches_tab, BranchesTab::Local, format!("Local ({})", snap.local_branches.len()));
                ui.selectable_value(&mut view.branches_tab, BranchesTab::Remotes, format!("Remotes ({})", snap.remote_branches.len()));
                ui.selectable_value(&mut view.branches_tab, BranchesTab::Tags, format!("Tags ({})", snap.tags.len()));
                ui.selectable_value(&mut view.branches_tab, BranchesTab::Worktrees, format!("Worktrees ({})", snap.worktrees.len()));
            });
            match view.branches_tab {
                BranchesTab::Local => virtual_rows(ui, "local", snap.local_branches.len(), |ui, i| local_row(ui, snap, &snap.local_branches[i], view, intents)),
                BranchesTab::Remotes if snap.remote_branches.is_empty() => {
                    ui.weak(if snap.remotes.is_empty() { "No remotes" } else { "No remote branches; try Fetch" });
                }
                BranchesTab::Remotes => virtual_rows(ui, "remotes", snap.remote_branches.len(), |ui, i| remote_row(ui, snap, &snap.remote_branches[i], view, intents)),
                BranchesTab::Tags => virtual_rows(ui, "tags", snap.tags.len(), |ui, i| tag_row(ui, snap, &snap.tags[i], view, intents)),
                BranchesTab::Worktrees => rows(ui, "worktrees", |ui| {
                    if snap.worktrees.iter().any(|w| w.prunable) && ui.button("Prune worktrees whose folder is gone").clicked() {
                        intents.push(run("Pruning worktrees…", ops::worktree_prune));
                    }
                    for w in &snap.worktrees {
                        let mut text = w.path.display().to_string();
                        if let Some(b) = &w.branch {
                            text.push_str(&format!("  [{b}]"));
                        }
                        if w.main {
                            text.push_str("  (main)");
                        }
                        if w.locked {
                            text.push_str("  (locked)");
                        }
                        if w.prunable {
                            text.push_str("  (folder gone)");
                        }
                        let r = ui.selectable_label(w.current, text);
                        if r.double_clicked() && !w.current && !w.prunable {
                            intents.push(Intent::OpenRepo(Some(w.path.clone())));
                        }
                        r.context_menu(|ui| {
                            if !w.current && !w.prunable {
                                let path = w.path.clone();
                                item(ui, "Open in Oxbow", intents, || Intent::OpenRepo(Some(path)));
                            }
                            copy_item(ui, "Copy path", &w.path.display().to_string());
                            if !w.main && !w.current {
                                ui.separator();
                                let dir = w.path.display().to_string();
                                let title = format!("Remove the worktree at {dir}?");
                                item(ui, "Remove…", intents, || {
                                    Intent::Dialog(dialogs::confirm(
                                        title,
                                        "Its folder is deleted; its branch stays. git refuses if it has uncommitted changes.",
                                        "Remove",
                                        action("Removing the worktree…", move |g| ops::worktree_remove(g, &dir, false)),
                                    ))
                                });
                            }
                        });
                    }
                }),
            }
        },
    );
}

/// A folder beside the main worktree: `<repo>-<branch>`.
fn suggested_worktree_dir(snap: &Snapshot, branch: &str) -> String {
    let Some(main) = snap.worktrees.iter().find(|w| w.main) else { return String::new() };
    let name = main.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = branch.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();
    main.path.with_file_name(format!("{name}-{safe}")).display().to_string()
}

fn new_branch_dialog(start: &str, suggested: &str) -> Intent {
    let start = start.to_owned();
    let title = if start == "HEAD" { "New branch".to_owned() } else { format!("New branch from {start}") };
    Intent::Dialog(
        dialogs::prompt(title, "Branch name", suggested, move |name, checkout| {
            action(format!("Creating branch {name}…"), move |g| ops::create_branch(g, &name, &start, checkout))
        })
        .with_check("Check it out", true),
    )
}

fn new_tag_dialog(rev: String) -> Intent {
    Intent::Dialog(Dialog::Prompt {
        title: "New tag".into(),
        label: "Tag name (a message makes it annotated: add one after a space-free name, on the next line)".into(),
        text: String::new(),
        multiline: true,
        check: None,
        make: Some(Box::new(move |text, _| {
            let (name, message) = text.split_once('\n').map(|(n, m)| (n.trim().to_owned(), Some(m.trim().to_owned()))).unwrap_or((text.trim().to_owned(), None));
            action(format!("Creating tag {name}…"), move |g| ops::create_tag(g, &name, &rev, message.as_deref()))
        })),
        focused: false,
    })
}

fn local_row(ui: &mut egui::Ui, snap: &Snapshot, b: &LocalBranch, view: &View, intents: &mut Vec<Intent>) {
    let selected = view.selection == Some(Selection::Branch(b.name.clone()));
    let mut text = RichText::new(format!("{}{}", if b.current { "⏺ " } else { "   " }, b.name));
    if b.current {
        text = text.color(theme::palette(ui).local_ref).strong();
    }
    let response = ui
        .horizontal(|ui| {
            let r = ui.selectable_label(selected, text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(&b.rel_date);
                if b.gone {
                    ui.weak("gone");
                } else if b.upstream.is_some() && (b.ahead > 0 || b.behind > 0) {
                    ui.label(format!("⬆{} ⬇{}", b.ahead, b.behind));
                }
                if let Some(wt) = &b.worktree {
                    ui.weak("worktree").on_hover_text(wt.display().to_string());
                }
            });
            r
        })
        .inner;
    if response.clicked() {
        intents.push(Intent::Select(Selection::Branch(b.name.clone()), Request::Log { title: b.name.clone(), rev: format!("refs/heads/{}", b.name) }));
    }
    if response.double_clicked() && !b.current {
        let name = b.name.clone();
        intents.push(run(&format!("Checking out {name}…"), move |g| ops::checkout(g, &name)));
    }
    response.context_menu(|ui| {
        let name = b.name.clone();
        let current = snap.branch.clone();
        if !b.current {
            let n = name.clone();
            item(ui, "Check out", intents, || run(&format!("Checking out {n}…"), move |g| ops::checkout(g, &n)));
        }
        item(ui, "New branch from here…", intents, || new_branch_dialog(&name, ""));
        if let (false, Some(cur)) = (b.current, &current) {
            let n = name.clone();
            item(ui, &format!("Merge into {cur}"), intents, || run(&format!("Merging {n}…"), move |g| ops::merge(g, &n)));
            let n = name.clone();
            item(ui, &format!("Rebase {cur} onto this"), intents, || run(&format!("Rebasing onto {n}…"), move |g| ops::rebase_onto(g, &n)));
            let n = name.clone();
            item(ui, &format!("Rebase {cur} onto this interactively"), intents, || run(&format!("Rebasing onto {n}…"), move |g| Rewrite::new(g).rebase_interactive(&n)));
        }
        if !b.current && b.worktree.is_none() {
            let n = name.clone();
            let dir = suggested_worktree_dir(snap, &n);
            item(ui, "Check out in a new worktree…", intents, || {
                Intent::Dialog(dialogs::prompt(format!("New worktree for {n}"), "Folder for the worktree", dir, move |dir, _| {
                    action("Adding a worktree…", move |g| ops::worktree_add(g, &dir, &n, None))
                }))
            });
        }
        if b.upstream.is_some() && b.behind > 0 && b.ahead == 0 {
            let br = b.clone();
            item(ui, "Fast-forward to upstream", intents, || run(&format!("Fast-forwarding {}…", br.name), move |g| ops::fast_forward(g, &br)));
        }
        if b.upstream.is_none() && !snap.remote_branches.is_empty() {
            let n = name.clone();
            let guess = snap.default_remote().map(|r| format!("{r}/{n}")).unwrap_or_default();
            item(ui, "Set upstream…", intents, || {
                Intent::Dialog(dialogs::prompt(format!("Upstream for {n}"), "Remote branch, e.g. origin/main", guess, move |up, _| {
                    action("Setting upstream…", move |g| ops::set_upstream(g, &n, &up))
                }))
            });
        }
        ui.separator();
        let n = name.clone();
        item(ui, "Rename…", intents, || {
            let old = n.clone();
            Intent::Dialog(dialogs::prompt(format!("Rename {n}"), "New name", n.clone(), move |new, _| action("Renaming branch…", move |g| ops::rename_branch(g, &old, &new))))
        });
        if !b.current {
            let n = name.clone();
            item(ui, "Delete…", intents, || {
                Intent::Dialog(dialogs::confirm(
                    format!("Delete {n}?"),
                    "Git refuses if the branch has commits that aren't merged anywhere; use Force delete for those.",
                    "Delete",
                    action(format!("Deleting {n}…"), move |g| ops::delete_branch(g, &n, false)),
                ))
            });
            let n = name.clone();
            item(ui, "Force delete…", intents, || {
                Intent::Dialog(dialogs::confirm(
                    format!("Force delete {n}?"),
                    "Commits only on this branch are no longer on any branch. They stay in the reflog for a while.",
                    "Force delete",
                    action(format!("Deleting {n}…"), move |g| ops::delete_branch(g, &n, true)),
                ))
            });
        }
        ui.separator();
        copy_item(ui, "Copy name", &b.name);
    });
}

fn remote_row(ui: &mut egui::Ui, snap: &Snapshot, b: &RemoteBranch, view: &View, intents: &mut Vec<Intent>) {
    let selected = view.selection == Some(Selection::Remote(b.name.clone()));
    let response = ui
        .horizontal(|ui| {
            let r = ui.selectable_label(selected, RichText::new(&b.name).color(theme::palette(ui).remote_ref));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(&b.rel_date));
            r
        })
        .inner;
    if response.clicked() {
        intents.push(Intent::Select(Selection::Remote(b.name.clone()), Request::Log { title: b.name.clone(), rev: format!("refs/remotes/{}", b.name) }));
    }
    response.context_menu(|ui| {
        let (full, local) = (b.name.clone(), b.branch.clone());
        item(ui, &format!("Check out as {local}"), intents, || run(&format!("Checking out {full}…"), move |g| ops::checkout_remote(g, &full, &local)));
        let name = b.name.clone();
        item(ui, "New branch from here…", intents, || new_branch_dialog(&name, &b.branch));
        if let Some(cur) = &snap.branch {
            let n = b.name.clone();
            item(ui, &format!("Merge into {cur}"), intents, || run(&format!("Merging {n}…"), move |g| ops::merge(g, &n)));
            let n = b.name.clone();
            item(ui, &format!("Rebase {cur} onto this"), intents, || run(&format!("Rebasing onto {n}…"), move |g| ops::rebase_onto(g, &n)));
        }
        ui.separator();
        let (remote, branch, name) = (b.remote.clone(), b.branch.clone(), b.name.clone());
        item(ui, "Delete on remote…", intents, || {
            Intent::Dialog(dialogs::confirm(
                format!("Delete {name} on {remote}?"),
                "The branch is removed from the remote for everyone.",
                "Delete",
                action(format!("Deleting {name}…"), move |g| ops::delete_remote_branch(g, &remote, &branch)),
            ))
        });
        ui.separator();
        copy_item(ui, "Copy name", &b.name);
    });
}

fn tag_row(ui: &mut egui::Ui, snap: &Snapshot, t: &Tag, view: &View, intents: &mut Vec<Intent>) {
    let selected = view.selection == Some(Selection::Tag(t.name.clone()));
    let response = ui
        .horizontal(|ui| {
            let r = ui.selectable_label(selected, RichText::new(&t.name).color(theme::palette(ui).tag_ref));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(&t.rel_date);
                if t.annotated {
                    ui.weak(&t.subject);
                }
            });
            r
        })
        .inner;
    if response.clicked() {
        intents.push(Intent::Select(Selection::Tag(t.name.clone()), Request::Log { title: format!("Tag {}", t.name), rev: format!("refs/tags/{}", t.name) }));
    }
    response.context_menu(|ui| {
        let name = t.name.clone();
        let n = name.clone();
        item(ui, "Check out (detached)", intents, || run(&format!("Checking out {n}…"), move |g| ops::checkout_detached(g, &n)));
        item(ui, "New branch from here…", intents, || new_branch_dialog(&name, ""));
        if let Some(remote) = snap.default_remote() {
            let (r, n) = (remote.to_owned(), name.clone());
            item(ui, &format!("Push to {remote}"), intents, || run(&format!("Pushing tag {n}…"), move |g| ops::push_tag(g, &r, &n)));
        }
        ui.separator();
        let n = name.clone();
        item(ui, "Delete…", intents, || {
            Intent::Dialog(dialogs::confirm(format!("Delete tag {n}?"), "Only the local tag is deleted.", "Delete", action(format!("Deleting tag {n}…"), move |g| ops::delete_tag(g, &n))))
        });
        ui.separator();
        copy_item(ui, "Copy name", &t.name);
    });
}

// --- Commits ---------------------------------------------------------------------------

pub fn commits(ui: &mut egui::Ui, snap: &Snapshot, view: &mut View, height: f32, intents: &mut Vec<Intent>) {
    section(
        ui,
        "Commits",
        height,
        &mut (view, intents),
        |ui, (view, intents)| {
            if view.commits_tab == CommitsTab::Commits {
                let r = ui.add(egui::TextEdit::singleline(&mut view.search).hint_text("Search").desired_width(140.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    intents.push(Intent::Search(view.search.clone()));
                }
                if !view.search.is_empty() && ui.small_button("✖").clicked() {
                    view.search.clear();
                    intents.push(Intent::Search(String::new()));
                }
            }
        },
        |ui, (view, intents)| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut view.commits_tab, CommitsTab::Commits, format!("Commits ({})", snap.commits.len()));
                ui.selectable_value(&mut view.commits_tab, CommitsTab::Reflog, "Reflog");
                if view.commits_tab == CommitsTab::Commits && view.selected_commits.len() > 1 {
                    ui.weak(format!("{} selected", view.selected_commits.len()));
                }
            });
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let row_height = ui.spacing().interact_size.y;
            match view.commits_tab {
                CommitsTab::Commits => {
                    if !snap.todo.is_empty() {
                        todo_steps(ui, snap, intents);
                    }
                    if !view.cherry_picks.is_empty() {
                        cherry_pick_bar(ui, snap, view, intents);
                    }
                    if view.picking.patch.is_some() {
                        patch_bar(ui, snap, view, intents);
                    }
                    if snap.commits.is_empty() {
                        ui.weak(if view.search.is_empty() { "No commits yet" } else { "Nothing found" });
                        return;
                    }
                    // Very wide graphs are cut off rather than pushing the text out of view.
                    let lanes = snap.graph.iter().map(|r| r.width).max().unwrap_or(0).min(MAX_LANES);
                    egui::ScrollArea::vertical().id_salt("commits").auto_shrink([false, false]).show_rows(ui, row_height, snap.commits.len(), |ui, range| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            for i in range {
                                commit_row(ui, snap, i, lanes, view, intents);
                            }
                        });
                    });
                }
                CommitsTab::Reflog => {
                    egui::ScrollArea::vertical().id_salt("reflog").auto_shrink([false, false]).show_rows(ui, row_height, snap.reflog.len(), |ui, range| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            for e in &snap.reflog[range] {
                                let selected = view.selection == Some(Selection::Reflog(e.selector.clone()));
                                let text = RichText::new(format!("{}  {}  {}", e.short_sha, e.subject, e.rel_date));
                                let r = ui.selectable_label(selected, text);
                                if r.clicked() {
                                    intents.push(Intent::Select(Selection::Reflog(e.selector.clone()), Request::Rev(e.sha.clone())));
                                }
                                r.context_menu(|ui| {
                                    let sha = e.sha.clone();
                                    item(ui, "Check out (detached)", intents, || run("Checking out…", move |g| ops::checkout_detached(g, &sha)));
                                    let sha = e.sha.clone();
                                    item(ui, "New branch from here…", intents, || new_branch_dialog(&sha, ""));
                                    let sha = e.sha.clone();
                                    item(ui, "Reset hard to here…", intents, || reset_hard_dialog(sha));
                                    ui.separator();
                                    copy_item(ui, "Copy SHA", &e.sha);
                                });
                            }
                        });
                    });
                }
            }
        },
    );
}

/// The custom patch: what is in it, and what to do with it.
fn patch_bar(ui: &mut egui::Ui, snap: &Snapshot, view: &View, intents: &mut Vec<Intent>) {
    let Some(p) = &view.picking.patch else { return };
    let source = snap.commits.iter().position(|c| c.sha == p.sha);
    ui.horizontal(|ui| {
        let files = p.files.keys().cloned().collect::<Vec<_>>().join("\n");
        let what = format!("Patch: {} in {} from {}", plural(p.lines(), "line"), plural(p.files.len(), "file"), short(&p.sha));
        ui.colored_label(theme::palette(ui).hunk, what).on_hover_text(format!("{}\n\n{files}", p.subject));
        let cx = Cx { snap, confirm: view.confirm_rewriting_pushed };
        let removal = p.removal();
        let (sha, subject) = (p.sha.clone(), p.subject.clone());
        ui.menu_button("Use it ⏷", |ui| {
            let busy = snap.operation.is_some();
            let Ok(removal) = removal else {
                ui.colored_label(theme::palette(ui).conflict, removal.err().unwrap_or_default());
                return;
            };
            match source {
                Some(index) if !busy => {
                    let targets = [
                        ("Take it out of the commit", "The lines are removed from the commit. Undo brings them back.", PatchTarget::Discard),
                        ("Move it to the working tree", "The lines leave the commit and become uncommitted changes.", PatchTarget::WorkingTree),
                    ];
                    for (label, hover, target) in targets {
                        let (s, r) = (sha.clone(), removal.clone());
                        let task: Task = Box::new(move |g| Rewrite::new(g).remove_patch(&s, &r, &target));
                        if ui.button(label).on_hover_text(hover).clicked() {
                            ui.close();
                            intents.push(rewrite(cx, index, "Taking the lines out of the commit…", None, task));
                        }
                    }
                    let (s, r) = (sha.clone(), removal.clone());
                    let suggested = format!("Split out of \"{subject}\"");
                    if ui.button("Move it into a new commit…").on_hover_text("Right after the commit it comes from").clicked() {
                        ui.close();
                        intents.push(Intent::Dialog(dialogs::prompt("Move the patch into a new commit", "Message for the new commit", suggested, move |message, _| {
                            action("Splitting the commit…", move |g| Rewrite::new(g).remove_patch(&s, &r, &PatchTarget::NewCommit(message)))
                        })));
                    }
                    ui.weak("To move it into another commit, right-click that commit.");
                    ui.separator();
                }
                Some(_) => {
                    ui.add_enabled(false, egui::Button::new("Finish what is in progress to change commits"));
                    ui.separator();
                }
                None => {
                    ui.weak("Its commit is not on the current branch: it can only be applied.");
                    ui.separator();
                }
            }
            for (label, reverse) in [("Apply it to the working tree", false), ("Revert it in the working tree", true)] {
                let (s, r) = (sha.clone(), removal.clone());
                if ui.button(label).clicked() {
                    ui.close();
                    intents.push(run("Applying the patch…", move |g| Rewrite::new(g).apply_patch(&s, &r, reverse)));
                }
            }
        });
        if ui.small_button("Clear").clicked() {
            intents.push(Intent::ClearPatch);
        }
    });
    ui.separator();
}

/// The commits copied for cherry-picking, with Paste and Clear.
fn cherry_pick_bar(ui: &mut egui::Ui, snap: &Snapshot, view: &View, intents: &mut Vec<Intent>) {
    ui.horizontal(|ui| {
        let n = view.cherry_picks.len();
        let list = view.cherry_picks.iter().map(|(sha, subject)| format!("{} {subject}", short(sha))).collect::<Vec<_>>().join("\n");
        ui.colored_label(theme::palette(ui).hunk, format!("{} copied", plural(n, "commit"))).on_hover_text(list);
        let paste = ui.add_enabled(snap.operation.is_none(), egui::Button::new("Paste onto HEAD").small());
        if paste.on_hover_text("Cherry-picks them, oldest first").clicked() {
            let shas: Vec<String> = view.cherry_picks.iter().map(|(s, _)| s.clone()).collect();
            intents.push(run(&format!("Cherry-picking {}…", plural(n, "commit")), move |g| ops::cherry_pick(g, &shas)));
            intents.push(Intent::CherryPicks(CherryPicks::Clear));
        }
        if ui.small_button("Clear").clicked() {
            intents.push(Intent::CherryPicks(CherryPicks::Clear));
        }
    });
    ui.separator();
}

/// The steps a paused interactive rebase still has to apply, newest on top like the commits below them.
fn todo_steps(ui: &mut egui::Ui, snap: &Snapshot, intents: &mut Vec<Intent>) {
    ui.horizontal(|ui| {
        ui.colored_label(theme::palette(ui).conflict, format!("Rebase steps still to apply ({})", snap.todo.len()));
        ui.weak("right-click a step to change it");
    });
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let body = egui::TextStyle::Body.resolve(ui.style());
    let (weak, text) = (ui.visuals().weak_text_color(), ui.visuals().text_color());
    let max = ui.available_height() * 0.45;
    egui::ScrollArea::vertical().id_salt("todo").max_height(max).auto_shrink([false, true]).show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            let n = snap.todo.len();
            for (i, step) in snap.todo.iter().enumerate().rev() {
                let word = match &step.kind {
                    StepKind::Commit(a) => a.name(),
                    StepKind::Merge => "merge",
                    StepKind::UpdateRef(_) => "branch",
                };
                let fmt = |font: &egui::FontId, color| egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
                let action_color = if step.action() == Some(TodoAction::Drop) { theme::palette(ui).conflict } else { theme::palette(ui).hunk };
                let mut job = egui::text::LayoutJob::default();
                job.append(&format!("{word:<7}"), 0.0, fmt(&mono, action_color));
                job.append(&step.sha, 6.0, fmt(&mono, weak));
                job.append(&step.subject, 6.0, fmt(&body, text));
                let r = ui.selectable_label(false, job);
                if r.clicked() && !step.sha.is_empty() {
                    intents.push(Intent::Select(Selection::Commit(step.sha.clone()), Request::Rev(step.sha.clone())));
                }
                let StepKind::Commit(current) = step.kind else { continue };
                r.context_menu(|ui| {
                    for action in TodoAction::ALL {
                        if action == current {
                            continue;
                        }
                        let sha = step.sha.clone();
                        let label = if action == TodoAction::Reword { "Reword…".to_owned() } else { capitalize(action.name()) };
                        if ui.button(label).on_hover_text(action.describe()).clicked() {
                            ui.close();
                            intents.push(if action == TodoAction::Reword {
                                Intent::Reword { sha, warning: None, paused: true }
                            } else {
                                run("Changing the rebase step…", move |g| Rewrite::new(g).edit_todo(&TodoPlan { actions: vec![(sha, action)], ..Default::default() }))
                            });
                        }
                    }
                    ui.separator();
                    for (label, direction, possible) in [("Move up", Direction::Up, i + 1 < n), ("Move down", Direction::Down, i > 0)] {
                        let sha = step.sha.clone();
                        if ui.add_enabled(possible, egui::Button::new(label)).clicked() {
                            ui.close();
                            intents.push(run("Moving the rebase step…", move |g| Rewrite::new(g).edit_todo(&TodoPlan { move_commit: Some((sha, direction)), ..Default::default() })));
                        }
                    }
                });
            }
        });
    });
    ui.separator();
}

fn reset_hard_dialog(sha: String) -> Intent {
    Intent::Dialog(dialogs::confirm(
        format!("Reset hard to {}?", &sha[..7.min(sha.len())]),
        "The current branch moves there and all uncommitted changes are lost. Commits left behind stay in the reflog.",
        "Reset",
        action("Resetting…", move |g| ops::reset(g, &sha, ResetMode::Hard)),
    ))
}

/// How many of the commits a rewrite changes, from `oldest` (an index in the commit list) up to HEAD, are already
/// pushed or on a main branch.
/// What a history rewrite needs to know to decide whether to ask first.
#[derive(Clone, Copy)]
struct Cx<'a> {
    snap: &'a Snapshot,
    /// Ask before rewriting published commits.
    confirm: bool,
}

fn published(cx: Cx, oldest: usize) -> usize {
    if !cx.confirm {
        return 0;
    }
    let snap = cx.snap;
    let end = (oldest + 1).min(snap.commits.len());
    snap.commits[..end].iter().filter(|c| snap.mark(&c.sha) != CommitMark::Unpushed).count()
}

fn published_warning(count: usize) -> Option<String> {
    (count > 0).then(|| {
        let what = if count == 1 { "1 commit that is".to_owned() } else { format!("{count} commits that are") };
        format!("This rewrites {what} already pushed or on a main branch: you'll need to force push, and anyone who has them will have to deal with the new history.")
    })
}

type Task = Box<dyn FnOnce(&oxbow_engine::Git) -> oxbow_engine::Result<()> + Send>;

/// Runs a history rewrite that changes the commits from index `oldest` up to HEAD. Asks first when some of them are
/// published, or when `ask` gives a reason (title, body, button).
fn rewrite(cx: Cx, oldest: usize, title: &str, ask: Option<(String, String, &str)>, task: Task) -> Intent {
    let warning = published_warning(published(cx, oldest));
    let action = action(title, task);
    match (ask, warning) {
        (None, None) => Intent::Run(action),
        (Some((t, body, button)), warning) => {
            let body = match warning {
                Some(w) => format!("{body}\n\n{w}"),
                None => body,
            };
            Intent::Dialog(dialogs::confirm(t, body, button, action))
        }
        (None, Some(warning)) => Intent::Dialog(dialogs::confirm("Rewrite published commits?", warning, "Rewrite", action)),
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

const MAX_LANES: usize = 10;
const LANE_WIDTH: f32 = 12.0;

/// Draws a commit's row of the graph in the `lanes`-wide strip at the left of `rect`.
fn paint_graph(ui: &egui::Ui, rect: egui::Rect, row: &oxbow_engine::graph::Row, lanes: usize, dot: egui::Color32) {
    use oxbow_engine::graph::Part;
    let painter = ui.painter();
    // Rows have a gap between them; lines run through it so they join up.
    let gap = ui.spacing().item_spacing.y / 2.0;
    let (top, bottom, mid) = (rect.top() - gap, rect.bottom() + gap, rect.center().y);
    let x = |lane: usize| rect.left() + LANE_WIDTH * (lane.min(lanes) as f32 + 0.5);
    let hidden = |lane: usize| lane >= lanes;
    for line in &row.lines {
        if hidden(line.from) && hidden(line.to) {
            continue;
        }
        let stroke = egui::Stroke::new(1.6, theme::palette(ui).lane(line.color));
        let (a, b) = match line.part {
            Part::Through => (egui::pos2(x(line.from), top), egui::pos2(x(line.to), bottom)),
            Part::Top => (egui::pos2(x(line.from), top), egui::pos2(x(line.to), mid)),
            Part::Bottom => (egui::pos2(x(line.from), mid), egui::pos2(x(line.to), bottom)),
        };
        if a.x == b.x {
            painter.line_segment([a, b], stroke);
        } else {
            // A lane changing sideways bends rather than cutting straight across.
            let bend = egui::epaint::CubicBezierShape::from_points_stroke(
                [a, egui::pos2(a.x, (a.y + b.y) / 2.0), egui::pos2(b.x, (a.y + b.y) / 2.0), b],
                false,
                egui::Color32::TRANSPARENT,
                stroke,
            );
            painter.add(bend);
        }
    }
    if !hidden(row.lane) {
        let center = egui::pos2(x(row.lane), mid);
        painter.circle(center, 4.0, dot, egui::Stroke::new(1.5, theme::palette(ui).lane(row.color)));
    }
}

fn commit_row(ui: &mut egui::Ui, snap: &Snapshot, index: usize, lanes: usize, view: &View, intents: &mut Vec<Intent>) {
    let c = &snap.commits[index];
    let selected = view.selected_commits.contains(&c.sha);
    let mark = snap.mark(&c.sha);
    let graph = snap.graph.get(index).filter(|_| lanes > 0);
    let space = if graph.is_some() { lanes as f32 * LANE_WIDTH + 4.0 } else { 0.0 };
    let response = ui.selectable_label(selected, theme::commit_row_after(ui, c, Some(mark), space));
    if let Some(row) = graph {
        let strip = egui::Rect::from_min_size(egui::pos2(response.rect.left() + ui.spacing().button_padding.x, response.rect.top()), egui::vec2(space, response.rect.height()));
        paint_graph(ui, strip, row, lanes, theme::palette(ui).mark(mark));
    }
    if response.clicked() {
        let modifiers = ui.input(|i| i.modifiers);
        intents.push(if modifiers.command {
            Intent::ToggleCommit(c.sha.clone())
        } else if modifiers.shift {
            Intent::CommitRange(c.sha.clone())
        } else {
            Intent::Select(Selection::Commit(c.sha.clone()), Request::Commit(c.clone()))
        });
    }
    response.context_menu(|ui| {
        // The selection when the row is part of it, otherwise just this row; newest first.
        let targets: Vec<usize> = if selected && view.selected_commits.len() > 1 {
            (0..snap.commits.len()).filter(|&i| view.selected_commits.contains(&snap.commits[i].sha)).collect()
        } else {
            vec![index]
        };
        let shas: Vec<String> = targets.iter().map(|&i| snap.commits[i].sha.clone()).collect();
        let single = targets.len() == 1;
        if single {
            copy_item(ui, "Copy SHA", &c.sha);
            copy_item(ui, "Copy subject", &c.subject);
            ui.separator();
            let sha = c.sha.clone();
            item(ui, "Check out (detached)", intents, || run("Checking out…", move |g| ops::checkout_detached(g, &sha)));
            item(ui, "New branch from here…", intents, || new_branch_dialog(&c.sha, ""));
            item(ui, "New tag here…", intents, || new_tag_dialog(c.sha.clone()));
            ui.separator();
        }
        if snap.head.as_deref() != Some(c.sha.as_str()) || !single {
            // Oldest first, so they apply in their original order.
            let picks: Vec<String> = shas.iter().rev().cloned().collect();
            let label = if single { "Cherry-pick onto HEAD".to_owned() } else { format!("Cherry-pick {} onto HEAD", plural(picks.len(), "commit")) };
            item(ui, &label, intents, || run("Cherry-picking…", move |g| ops::cherry_pick(g, &picks)));
        }
        let copied = shas.iter().all(|s| view.cherry_picks.iter().any(|(c, _)| c == s));
        let copy_label = match (copied, single) {
            (true, true) => "Remove from the cherry-pick list".to_owned(),
            (true, false) => format!("Remove {} from the cherry-pick list", plural(shas.len(), "commit")),
            (false, true) => "Copy for cherry-pick".to_owned(),
            (false, false) => format!("Copy {} for cherry-pick", plural(shas.len(), "commit")),
        };
        // Oldest first: the order they are applied in.
        let picked: Vec<(String, String)> = targets.iter().rev().map(|&i| (snap.commits[i].sha.clone(), snap.commits[i].subject.clone())).collect();
        item(ui, &copy_label, intents, || Intent::CherryPicks(if copied { CherryPicks::Remove(picked) } else { CherryPicks::Add(picked) }));
        if single {
            let (sha, merge) = (c.sha.clone(), c.is_merge());
            item(ui, "Revert", intents, || run("Reverting…", move |g| ops::revert(g, &sha, merge)));
        }
        if single && snap.operation.is_none() {
            let b = snap.bisect.as_ref();
            let (bad, good) = b.map(|b| (b.terms.0.clone(), b.terms.1.clone())).unwrap_or_else(|| ("bad".into(), "good".into()));
            ui.menu_button("Bisect", |ui| {
                if b.is_none() {
                    ui.weak("Find the commit that introduced a problem:\nmark one that has it and one that does not.");
                }
                for (label, mark) in [(format!("Mark as {bad}"), ops::BisectMark::Bad), (format!("Mark as {good}"), ops::BisectMark::Good)] {
                    let sha = c.sha.clone();
                    item(ui, &label, intents, || run("Bisecting…", move |g| ops::bisect_mark(g, &sha, mark)));
                }
                if b.is_some() {
                    let sha = c.sha.clone();
                    item(ui, "Skip", intents, || run("Bisecting…", move |g| ops::bisect_mark(g, &sha, ops::BisectMark::Skip)));
                    item(ui, "End bisect", intents, || run("Ending the bisect…", ops::bisect_reset));
                }
            });
        }
        if let (Some(p), true) = (&view.picking.patch, single)
            && p.sha != c.sha
            && let Some(source) = snap.commits.iter().position(|x| x.sha == p.sha)
            && snap.operation.is_none()
        {
            match p.removal() {
                Ok(removal) => {
                    let (from, to) = (p.sha.clone(), c.sha.clone());
                    let task: Task = Box::new(move |g| Rewrite::new(g).move_patch(&from, &removal, &to));
                    let label = format!("Move the patch from {} into this commit", short(&p.sha));
                    if ui.button(label).clicked() {
                        ui.close();
                        intents.push(rewrite(Cx { snap, confirm: view.confirm_rewriting_pushed }, source.max(index), "Moving the lines…", None, task));
                    }
                }
                Err(e) => {
                    ui.add_enabled(false, egui::Button::new(format!("Move the patch here: {e}")));
                }
            }
        }
        ui.separator();
        history_menu(ui, Cx { snap, confirm: view.confirm_rewriting_pushed }, index, &targets, intents);
        if single {
            ui.separator();
            ui.menu_button("Reset to here", |ui| {
                let sha = c.sha.clone();
                item(ui, "Soft: keep changes staged", intents, || run("Resetting…", move |g| ops::reset(g, &sha, ResetMode::Soft)));
                let sha = c.sha.clone();
                item(ui, "Mixed: keep changes unstaged", intents, || run("Resetting…", move |g| ops::reset(g, &sha, ResetMode::Mixed)));
                let sha = c.sha.clone();
                item(ui, "Hard: discard changes…", intents, || reset_hard_dialog(sha));
            });
        }
    });
}

/// A menu entry for a history rewrite: pushes [`rewrite`]'s intent when clicked.
#[allow(clippy::too_many_arguments)]
fn rewrite_item(ui: &mut egui::Ui, label: &str, enabled: bool, hover: &str, cx: Cx, oldest: usize, title: &str, intents: &mut Vec<Intent>, task: Task) {
    let mut response = ui.add_enabled(enabled, egui::Button::new(label));
    if !hover.is_empty() {
        response = response.on_hover_text(hover);
    }
    if response.clicked() {
        ui.close();
        intents.push(rewrite(cx, oldest, title, None, task));
    }
}

/// Squash, fixup, drop, reword, edit, move and amend: rewriting the current branch. `targets` are indexes in the
/// commit list, newest first; `index` is the clicked row.
fn history_menu(ui: &mut egui::Ui, cx: Cx, index: usize, targets: &[usize], intents: &mut Vec<Intent>) {
    let snap = cx.snap;
    if let Some(op) = snap.operation {
        ui.add_enabled(false, egui::Button::new(format!("Finish the {} to rewrite history", op.name())));
        return;
    }
    let c = &snap.commits[index];
    let n = targets.len();
    let single = n == 1;
    let shas: Vec<String> = targets.iter().map(|&i| snap.commits[i].sha.clone()).collect();
    let oldest = targets.iter().copied().max().unwrap_or(index);
    // Squashing rewrites the commit each one melds into as well.
    let below = |i: usize| (i + 1).min(snap.commits.len() - 1);
    let is_root = |i: usize| snap.commits[i].parents.is_empty();
    let staged = snap.status.iter().any(|e| e.is_staged());

    if single {
        let warning = published_warning(published(cx, index));
        item(ui, "Reword…", intents, || Intent::Reword { sha: c.sha.clone(), warning, paused: false });
        let sha = c.sha.clone();
        let hover = "Rebases up to this commit and stops there, so you can change it, then continue";
        rewrite_item(ui, "Edit (stop here)", true, hover, cx, index, "Stopping at the commit…", intents, Box::new(move |g| Rewrite::new(g).edit(&sha)));
    }
    let can_meld = !targets.iter().any(|&i| is_root(i));
    let (squash, fixup) = if single {
        ("Squash into previous".to_owned(), "Fixup into previous".to_owned())
    } else {
        (format!("Squash {} into their previous", plural(n, "commit")), format!("Fixup {} into their previous", plural(n, "commit")))
    };
    let s = shas.clone();
    rewrite_item(ui, &squash, can_meld, "Melds into the commit before, combining both messages", cx, below(oldest), "Squashing…", intents, Box::new(move |g| Rewrite::new(g).squash(&s, false)));
    let s = shas.clone();
    rewrite_item(ui, &fixup, can_meld, "Melds into the commit before, keeping its message", cx, below(oldest), "Fixing up…", intents, Box::new(move |g| Rewrite::new(g).squash(&s, true)));
    let drop_label = if single { "Drop…".to_owned() } else { format!("Drop {}…", plural(n, "commit")) };
    let s = shas.clone();
    item(ui, &drop_label, intents, || {
        let (title, what) = if single {
            ("Drop the commit?".to_owned(), format!("\"{}\" and its changes are removed from the history.", c.subject))
        } else {
            (format!("Drop {}?", plural(n, "commit")), format!("{} and their changes are removed from the history.", plural(n, "commit")))
        };
        rewrite(cx, oldest, "Dropping…", Some((title, format!("{what} Undo brings them back."), "Drop")), Box::new(move |g| Rewrite::new(g).drop_commits(&s)))
    });
    if !single {
        return;
    }
    ui.separator();
    let sha = c.sha.clone();
    let newest = snap.head.as_deref() == Some(c.sha.as_str());
    rewrite_item(ui, "Move up", !newest, "Swap with the newer commit above", cx, index, "Moving the commit…", intents, Box::new(move |g| Rewrite::new(g).move_commit(&sha, Direction::Up)));
    let sha = c.sha.clone();
    rewrite_item(ui, "Move down", !is_root(index), "Swap with the older commit below", cx, below(index), "Moving the commit…", intents, Box::new(move |g| Rewrite::new(g).move_commit(&sha, Direction::Down)));
    ui.separator();
    let sha = c.sha.clone();
    let hover = if staged { "Folds the staged changes into this commit" } else { "Stage the changes to fold in first" };
    rewrite_item(ui, "Amend staged changes into this", staged, hover, cx, index, "Amending the commit…", intents, Box::new(move |g| Rewrite::new(g).amend_staged(&sha)));
    let sha = c.sha.clone();
    let fixup_commit = ui.add_enabled(staged, egui::Button::new("Create fixup commit")).on_hover_text("Commits the staged changes as \"fixup! <subject>\"");
    if fixup_commit.clicked() {
        ui.close();
        intents.push(run("Creating a fixup commit…", move |g| Rewrite::new(g).create_fixup(&sha)));
    }
    let sha = c.sha.clone();
    let hover = "Melds every \"fixup!\" and \"squash!\" commit above into its target (git rebase --autosquash)";
    rewrite_item(ui, "Squash fixup commits above", true, hover, cx, index, "Squashing fixup commits…", intents, Box::new(move |g| Rewrite::new(g).autosquash(&sha)));
    let sha = c.sha.clone();
    rewrite_item(ui, "Reset author", true, "Makes you the author", cx, index, "Resetting the author…", intents, Box::new(move |g| Rewrite::new(g).reset_author(&sha)));
}

// --- Stash -----------------------------------------------------------------------------

pub fn stash(ui: &mut egui::Ui, snap: &Snapshot, view: &View, height: f32, intents: &mut Vec<Intent>) {
    let title = if snap.stashes.is_empty() { "Stash".to_owned() } else { format!("Stash ({})", snap.stashes.len()) };
    section(ui, &title, height, intents, |_, _| {}, |ui, intents| {
        if snap.stashes.is_empty() {
            ui.weak("No stashes");
            return;
        }
        virtual_rows(ui, "stashes", snap.stashes.len(), |ui, i| stash_row(ui, &snap.stashes[i], view, intents));
    });
}

fn stash_row(ui: &mut egui::Ui, s: &Stash, view: &View, intents: &mut Vec<Intent>) {
    let selected = view.selection == Some(Selection::Stash(s.sha.clone()));
    let r = ui.selectable_label(selected, format!("{}  {}  ·  {}", s.selector, s.message, s.rel_date));
    if r.clicked() {
        intents.push(Intent::Select(Selection::Stash(s.sha.clone()), Request::Stash(s.clone())));
    }
    r.context_menu(|ui| {
        let sel = s.selector.clone();
        item(ui, "Apply", intents, || run("Applying stash…", move |g| ops::stash_apply(g, &sel)));
        let sel = s.selector.clone();
        item(ui, "Pop", intents, || run("Popping stash…", move |g| ops::stash_pop(g, &sel)));
        let sel = s.selector.clone();
        item(ui, "New branch from stash…", intents, || {
            Intent::Dialog(dialogs::prompt("New branch from stash", "Branch name", "", move |name, _| action("Creating branch…", move |g| ops::stash_branch(g, &name, &sel))))
        });
        ui.separator();
        let (sel, message) = (s.selector.clone(), s.message.clone());
        item(ui, "Drop…", intents, || {
            Intent::Dialog(dialogs::confirm("Drop stash?", format!("\"{message}\" is deleted."), "Drop", action("Dropping stash…", move |g| ops::stash_drop(g, &sel))))
        });
    });
}
