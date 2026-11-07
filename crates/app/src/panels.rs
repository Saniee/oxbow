//! The left-hand panels: Status, Files (with the commit box), Branches, Commits and Stash. Each one reads the
//! snapshot and pushes intents; nothing here runs git directly.

use eframe::egui::{self, RichText};
use oxbow_engine::ops::{self, ResetMode, StashKind};
use oxbow_engine::queries::{LocalBranch, Operation, RemoteBranch, Stash, Tag};
use oxbow_engine::status::{self, StatusEntry};
use oxbow_engine::rebase::Rewrite;
use oxbow_engine::todo::{Direction, StepKind, TodoAction, TodoPlan};
use oxbow_engine::{CommitMark, Snapshot};

use crate::app::Intent;
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
    pub selected_commits: Vec<String>,
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
                ui.label(RichText::new(b).strong().color(theme::LOCAL_REF));
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
    if let Some(op) = snap.operation {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::CONFLICT, format!("{} in progress", capitalize(op.name())));
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
            let all = snap.status.clone();
            ui.menu_button("Stash ⏷", |ui| {
                stash_item(ui, "Stash changes…", StashKind::Tracked, intents);
                stash_item(ui, "Stash including untracked…", StashKind::IncludeUntracked, intents);
                stash_item(ui, "Stash staged changes…", StashKind::Staged, intents);
            });
            let unstage: Vec<StatusEntry> = all.iter().filter(|e| e.is_staged()).cloned().collect();
            if ui.add_enabled(!unstage.is_empty(), egui::Button::new("Unstage all")).clicked() {
                intents.push(run("Unstaging all…", move |g| status::unstage(g, &unstage)));
            }
            let stage: Vec<StatusEntry> = all.iter().filter(|e| e.is_unstaged() || e.is_conflict()).cloned().collect();
            if ui.add_enabled(!stage.is_empty(), egui::Button::new("Stage all")).clicked() {
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
                rows(ui, "files", |ui| {
                    let mono = egui::TextStyle::Monospace.resolve(ui.style());
                    for entry in &snap.status {
                        file_row(ui, entry, &mono, view, intents);
                    }
                });
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
    let mut job = theme::status_letters(entry.x, entry.y, entry.is_conflict(), entry.is_untracked(), mono);
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
                BranchesTab::Local => rows(ui, "local", |ui| {
                    for b in &snap.local_branches {
                        local_row(ui, snap, b, view, intents);
                    }
                }),
                BranchesTab::Remotes => rows(ui, "remotes", |ui| {
                    if snap.remote_branches.is_empty() {
                        ui.weak(if snap.remotes.is_empty() { "No remotes" } else { "No remote branches; try Fetch" });
                    }
                    for b in &snap.remote_branches {
                        remote_row(ui, snap, b, view, intents);
                    }
                }),
                BranchesTab::Tags => rows(ui, "tags", |ui| {
                    for t in &snap.tags {
                        tag_row(ui, snap, t, view, intents);
                    }
                }),
                BranchesTab::Worktrees => rows(ui, "worktrees", |ui| {
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
                        let r = ui.selectable_label(w.current, text);
                        r.context_menu(|ui| copy_item(ui, "Copy path", &w.path.display().to_string()));
                    }
                }),
            }
        },
    );
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
        text = text.color(theme::LOCAL_REF).strong();
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
            let r = ui.selectable_label(selected, RichText::new(&b.name).color(theme::REMOTE_REF));
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
            let r = ui.selectable_label(selected, RichText::new(&t.name).color(theme::TAG_REF));
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
                    if snap.commits.is_empty() {
                        ui.weak(if view.search.is_empty() { "No commits yet" } else { "Nothing found" });
                        return;
                    }
                    egui::ScrollArea::vertical().id_salt("commits").auto_shrink([false, false]).show_rows(ui, row_height, snap.commits.len(), |ui, range| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            for i in range {
                                commit_row(ui, snap, i, view, intents);
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

/// The steps a paused interactive rebase still has to apply, newest on top like the commits below them.
fn todo_steps(ui: &mut egui::Ui, snap: &Snapshot, intents: &mut Vec<Intent>) {
    ui.horizontal(|ui| {
        ui.colored_label(theme::CONFLICT, format!("Rebase steps still to apply ({})", snap.todo.len()));
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
                let action_color = if step.action() == Some(TodoAction::Drop) { theme::CONFLICT } else { theme::HUNK };
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
fn published(snap: &Snapshot, oldest: usize) -> usize {
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
fn rewrite(snap: &Snapshot, oldest: usize, title: &str, ask: Option<(String, String, &str)>, task: Task) -> Intent {
    let warning = published_warning(published(snap, oldest));
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

fn commit_row(ui: &mut egui::Ui, snap: &Snapshot, index: usize, view: &View, intents: &mut Vec<Intent>) {
    let c = &snap.commits[index];
    let selected = view.selected_commits.contains(&c.sha);
    let response = ui.selectable_label(selected, theme::commit_row(ui, c, Some(snap.mark(&c.sha))));
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
        if single {
            let (sha, merge) = (c.sha.clone(), c.is_merge());
            item(ui, "Revert", intents, || run("Reverting…", move |g| ops::revert(g, &sha, merge)));
        }
        ui.separator();
        history_menu(ui, snap, index, &targets, intents);
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
fn rewrite_item(ui: &mut egui::Ui, label: &str, enabled: bool, hover: &str, snap: &Snapshot, oldest: usize, title: &str, intents: &mut Vec<Intent>, task: Task) {
    let mut response = ui.add_enabled(enabled, egui::Button::new(label));
    if !hover.is_empty() {
        response = response.on_hover_text(hover);
    }
    if response.clicked() {
        ui.close();
        intents.push(rewrite(snap, oldest, title, None, task));
    }
}

/// Squash, fixup, drop, reword, edit, move and amend: rewriting the current branch. `targets` are indexes in the
/// commit list, newest first; `index` is the clicked row.
fn history_menu(ui: &mut egui::Ui, snap: &Snapshot, index: usize, targets: &[usize], intents: &mut Vec<Intent>) {
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
        let warning = published_warning(published(snap, index));
        item(ui, "Reword…", intents, || Intent::Reword { sha: c.sha.clone(), warning, paused: false });
        let sha = c.sha.clone();
        let hover = "Rebases up to this commit and stops there, so you can change it, then continue";
        rewrite_item(ui, "Edit (stop here)", true, hover, snap, index, "Stopping at the commit…", intents, Box::new(move |g| Rewrite::new(g).edit(&sha)));
    }
    let can_meld = !targets.iter().any(|&i| is_root(i));
    let (squash, fixup) = if single {
        ("Squash into previous".to_owned(), "Fixup into previous".to_owned())
    } else {
        (format!("Squash {} into their previous", plural(n, "commit")), format!("Fixup {} into their previous", plural(n, "commit")))
    };
    let s = shas.clone();
    rewrite_item(ui, &squash, can_meld, "Melds into the commit before, combining both messages", snap, below(oldest), "Squashing…", intents, Box::new(move |g| Rewrite::new(g).squash(&s, false)));
    let s = shas.clone();
    rewrite_item(ui, &fixup, can_meld, "Melds into the commit before, keeping its message", snap, below(oldest), "Fixing up…", intents, Box::new(move |g| Rewrite::new(g).squash(&s, true)));
    let drop_label = if single { "Drop…".to_owned() } else { format!("Drop {}…", plural(n, "commit")) };
    let s = shas.clone();
    item(ui, &drop_label, intents, || {
        let (title, what) = if single {
            ("Drop the commit?".to_owned(), format!("\"{}\" and its changes are removed from the history.", c.subject))
        } else {
            (format!("Drop {}?", plural(n, "commit")), format!("{} and their changes are removed from the history.", plural(n, "commit")))
        };
        rewrite(snap, oldest, "Dropping…", Some((title, format!("{what} Undo brings them back."), "Drop")), Box::new(move |g| Rewrite::new(g).drop_commits(&s)))
    });
    if !single {
        return;
    }
    ui.separator();
    let sha = c.sha.clone();
    let newest = snap.head.as_deref() == Some(c.sha.as_str());
    rewrite_item(ui, "Move up", !newest, "Swap with the newer commit above", snap, index, "Moving the commit…", intents, Box::new(move |g| Rewrite::new(g).move_commit(&sha, Direction::Up)));
    let sha = c.sha.clone();
    rewrite_item(ui, "Move down", !is_root(index), "Swap with the older commit below", snap, below(index), "Moving the commit…", intents, Box::new(move |g| Rewrite::new(g).move_commit(&sha, Direction::Down)));
    ui.separator();
    let sha = c.sha.clone();
    let hover = if staged { "Folds the staged changes into this commit" } else { "Stage the changes to fold in first" };
    rewrite_item(ui, "Amend staged changes into this", staged, hover, snap, index, "Amending the commit…", intents, Box::new(move |g| Rewrite::new(g).amend_staged(&sha)));
    let sha = c.sha.clone();
    let fixup_commit = ui.add_enabled(staged, egui::Button::new("Create fixup commit")).on_hover_text("Commits the staged changes as \"fixup! <subject>\"");
    if fixup_commit.clicked() {
        ui.close();
        intents.push(run("Creating a fixup commit…", move |g| Rewrite::new(g).create_fixup(&sha)));
    }
    let sha = c.sha.clone();
    let hover = "Melds every \"fixup!\" and \"squash!\" commit above into its target (git rebase --autosquash)";
    rewrite_item(ui, "Squash fixup commits above", true, hover, snap, index, "Squashing fixup commits…", intents, Box::new(move |g| Rewrite::new(g).autosquash(&sha)));
    let sha = c.sha.clone();
    rewrite_item(ui, "Reset author", true, "Makes you the author", snap, index, "Resetting the author…", intents, Box::new(move |g| Rewrite::new(g).reset_author(&sha)));
}

// --- Stash -----------------------------------------------------------------------------

pub fn stash(ui: &mut egui::Ui, snap: &Snapshot, view: &View, height: f32, intents: &mut Vec<Intent>) {
    let title = if snap.stashes.is_empty() { "Stash".to_owned() } else { format!("Stash ({})", snap.stashes.len()) };
    section(ui, &title, height, intents, |_, _| {}, |ui, intents| {
        if snap.stashes.is_empty() {
            ui.weak("No stashes");
            return;
        }
        rows(ui, "stashes", |ui| {
            for s in &snap.stashes {
                stash_row(ui, s, view, intents);
            }
        });
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
