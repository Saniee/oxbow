//! The window: panels on the left, the selection's detail on the right, the command log below it.

use std::path::Path;

use eframe::egui::{self, RichText};
use oxbow_engine::rebase::Rewrite;
use oxbow_engine::{Git, LogStatus, Snapshot, queries, undo};

use crate::detail::{self, Detail, Request, Selection};
use crate::dialogs::{self, Dialog, Outcome};
use crate::panels::{self, View};
use crate::worker::{Action, Event, LoadOptions, Worker, action};

/// What a panel asks for; handled after the frame's UI is drawn.
pub enum Intent {
    Select(Selection, Request),
    Run(Action),
    Dialog(Dialog),
    Search(String),
    Amend(bool),
    /// Ctrl-click: add the commit to the selection, or take it out.
    ToggleCommit(String),
    /// Shift-click: select the commits from the selected one to this one.
    CommitRange(String),
    /// Ask for a commit's new message. `paused`: a step of the paused rebase.
    Reword { sha: String, warning: Option<String>, paused: bool },
    /// Undo (false) or redo (true), after confirming.
    Undo(bool),
    Refresh,
    OpenRepo,
}

struct Repo {
    worker: Worker<Detail>,
    snapshot: Option<Snapshot>,
    error: Option<String>,
}

#[derive(Default)]
struct DetailState {
    generation: u64,
    loading: bool,
    current: Option<Result<Detail, String>>,
}

pub struct App {
    ctx: egui::Context,
    repo: Option<Repo>,
    view: View,
    detail: DetailState,
    dialog: Option<Dialog>,
    /// The last failed action, until dismissed.
    error: Option<String>,
    was_focused: bool,
}

impl App {
    pub fn new(ctx: egui::Context, start: Option<&Path>) -> Self {
        let mut app = App { ctx, repo: None, view: View::default(), detail: DetailState::default(), dialog: None, error: None, was_focused: true };
        if let Some(path) = start {
            match Git::open(path) {
                Ok(git) => app.open(git),
                Err(e) if path.join(".git").exists() => app.error = Some(e.to_string()),
                Err(_) => {}
            }
        }
        app
    }

    fn open(&mut self, git: Git) {
        let title = format!("Oxbow — {}", git.root().display());
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        self.repo = Some(Repo { worker: Worker::new(git, self.ctx.clone(), LoadOptions::default()), snapshot: None, error: None });
        self.view = View::default();
        self.detail = DetailState::default();
        self.error = None;
    }

    fn pick_repo(&mut self) {
        let Some(dir) = rfd::FileDialog::new().set_title("Open repository").pick_folder() else { return };
        match Git::open(&dir) {
            Ok(git) => self.open(git),
            Err(_) => self.error = Some(format!("{} is not inside a Git repository.", dir.display())),
        }
    }

    fn select(&mut self, selection: Selection, request: Request) {
        self.view.selected_commits = match &selection {
            Selection::Commit(sha) => vec![sha.clone()],
            _ => Vec::new(),
        };
        self.view.selection = Some(selection);
        self.request_detail(request);
    }

    fn request_detail(&mut self, request: Request) {
        let Some(repo) = &self.repo else { return };
        self.detail.generation += 1;
        self.detail.loading = true;
        repo.worker.load_detail(self.detail.generation, move |git| detail::load(git, request));
    }

    /// After a refresh: reload what is shown when it may have changed, or clear it when it is gone.
    fn refresh_detail(&mut self) {
        let Some(snap) = self.repo.as_ref().and_then(|r| r.snapshot.as_ref()) else { return };
        // Rewritten commits are gone from the list.
        self.view.selected_commits.retain(|sha| snap.commits.iter().any(|c| &c.sha == sha));
        let request = match &self.view.selection {
            Some(Selection::File(path)) => match snap.status.iter().find(|e| &e.path == path) {
                Some(entry) => Some(Request::File(entry.clone())),
                None => {
                    self.view.selection = None;
                    self.detail.current = None;
                    None
                }
            },
            Some(Selection::Branch(name)) => Some(Request::Log { title: name.clone(), rev: format!("refs/heads/{name}") }),
            Some(Selection::Remote(name)) => Some(Request::Log { title: name.clone(), rev: format!("refs/remotes/{name}") }),
            Some(Selection::Stash(sha)) if !snap.stashes.iter().any(|s| &s.sha == sha) => {
                self.view.selection = None;
                self.detail.current = None;
                None
            }
            _ => None,
        };
        if let Some(request) = request {
            self.request_detail(request);
        }
    }

    fn handle(&mut self, intent: Intent) {
        match intent {
            Intent::Select(selection, request) => self.select(selection, request),
            Intent::Run(action) => {
                if let Some(repo) = &self.repo {
                    repo.worker.run(action);
                }
            }
            Intent::Dialog(dialog) => self.dialog = Some(dialog),
            Intent::Search(text) => {
                if let Some(repo) = &self.repo {
                    let text = text.trim().to_owned();
                    repo.worker.options.lock().unwrap().search = (!text.is_empty()).then_some(text);
                    repo.worker.refresh();
                }
            }
            Intent::Amend(on) => {
                // Start from the last commit's message, so amending can reword it.
                if on && self.view.commit_message.trim().is_empty() {
                    if let Some(repo) = &self.repo
                        && let Ok(message) = queries::commit_message(&repo.worker.git, "HEAD") {
                            self.view.commit_message = message;
                        }
                } else if !on {
                    self.view.commit_message.clear();
                }
            }
            Intent::ToggleCommit(sha) => {
                if let Some(i) = self.view.selected_commits.iter().position(|s| s == &sha) {
                    self.view.selected_commits.remove(i);
                } else {
                    self.view.selected_commits.push(sha);
                }
            }
            Intent::CommitRange(sha) => {
                let Some(snap) = self.repo.as_ref().and_then(|r| r.snapshot.as_ref()) else { return };
                let anchor = match &self.view.selection {
                    Some(Selection::Commit(a)) => snap.commits.iter().position(|c| &c.sha == a),
                    _ => None,
                };
                let Some(to) = snap.commits.iter().position(|c| c.sha == sha) else { return };
                let from = anchor.unwrap_or(to);
                self.view.selected_commits = snap.commits[from.min(to)..=from.max(to)].iter().map(|c| c.sha.clone()).collect();
            }
            Intent::Reword { sha, warning, paused } => self.reword_dialog(sha, warning, paused),
            Intent::Undo(redo) => self.undo_dialog(redo),
            Intent::Refresh => {
                if let Some(repo) = &self.repo {
                    repo.worker.refresh();
                }
            }
            Intent::OpenRepo => self.pick_repo(),
        }
    }

    fn reword_dialog(&mut self, sha: String, warning: Option<String>, paused: bool) {
        let Some(repo) = &self.repo else { return };
        let message = match queries::commit_message(&repo.worker.git, &sha) {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let short = sha[..sha.len().min(7)].to_owned();
        let mut label = if paused { "The new message is applied when the rebase gets to this step.".to_owned() } else { "The new message (Ctrl+Enter for OK)".to_owned() };
        if let Some(w) = warning {
            label = format!("{w}\n\n{label}");
        }
        self.dialog = Some(Dialog::Prompt {
            title: format!("Reword {short}"),
            label,
            text: message.trim_end().to_owned(),
            multiline: true,
            check: None,
            make: Some(Box::new(move |message, _| {
                if paused {
                    action("Changing the rebase step…", move |g| Rewrite::new(g).reword_todo(&sha, &message))
                } else {
                    action("Rewording…", move |g| Rewrite::new(g).reword(&sha, &message))
                }
            })),
            focused: false,
        });
    }

    fn undo_dialog(&mut self, redo: bool) {
        let Some(snap) = self.repo.as_ref().and_then(|r| r.snapshot.as_ref()) else { return };
        let plan = &snap.undo;
        let Some(step) = (if redo { plan.redo.clone() } else { plan.undo.clone() }) else { return };
        let verb = if redo { "Redo" } else { "Undo" };
        let body = undo::describe(&step, redo, snap.branch.as_deref());
        self.dialog = Some(dialogs::confirm(
            format!("{verb} \"{}\"?", step.description),
            body,
            verb,
            action(if redo { "Redoing…" } else { "Undoing…" }, move |g| {
                // What the reflog says now: something may have happened since the dialog opened.
                let plan = undo::load_plan(g)?;
                if (if redo { plan.redo } else { plan.undo }).as_ref() != Some(&step) {
                    return Err(oxbow_engine::GitError::user("The history changed in the meantime. Check what's there now and try again."));
                }
                undo::apply(g, &step, redo)
            }),
        ));
    }

    fn poll(&mut self) {
        let Some(repo) = &mut self.repo else { return };
        let mut refreshed = false;
        for event in repo.worker.poll() {
            match event {
                Event::Snapshot(Ok(snap)) => {
                    repo.snapshot = Some(snap);
                    repo.error = None;
                    refreshed = true;
                }
                Event::Snapshot(Err(e)) => repo.error = Some(e),
                Event::Done { title, error } => match error {
                    Some(e) => self.error = Some(e),
                    None => {
                        if title == "Committing…" || title == "Amending…" {
                            self.view.commit_message.clear();
                            self.view.amend = false;
                        }
                    }
                },
                Event::Detail { generation, result } => {
                    if generation == self.detail.generation {
                        self.detail.loading = false;
                        self.detail.current = Some(result);
                    }
                }
            }
        }
        if refreshed {
            self.refresh_detail();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        ui.horizontal(|ui| {
            if ui.button("Open repository…").clicked() {
                intents.push(Intent::OpenRepo);
            }
            if self.repo.is_some() && ui.button("⟳ Refresh").clicked() {
                intents.push(Intent::Refresh);
            }
            let Some(snap) = self.repo.as_ref().and_then(|r| r.snapshot.as_ref()) else { return };
            ui.separator();
            let busy = snap.operation.is_some() || snap.undo.rebase_in_progress;
            for (redo, step, key) in [(false, &snap.undo.undo, "Ctrl+Z"), (true, &snap.undo.redo, "Ctrl+Shift+Z")] {
                let label = if redo { "Redo" } else { "Undo" };
                let hover = match step {
                    _ if busy => "Finish the operation in progress first".to_owned(),
                    Some(s) => format!("{label} \"{}\" ({key})", s.description),
                    None => format!("Nothing to {}", label.to_lowercase()),
                };
                let enabled = step.is_some() && !busy;
                let response = ui.add_enabled(enabled, egui::Button::new(label));
                let response = if enabled { response.on_hover_text(hover) } else { response.on_disabled_hover_text(hover) };
                if response.clicked() {
                    intents.push(Intent::Undo(redo));
                }
            }
            // The shortcuts, unless a text field has them.
            let (undo_key, redo_key) = ui.input(|i| {
                let z = i.key_pressed(egui::Key::Z) && i.modifiers.command;
                (z && !i.modifiers.shift, z && i.modifiers.shift)
            });
            if ui.memory(|m| m.focused().is_none()) && self.dialog.is_none() && !busy {
                if undo_key && snap.undo.undo.is_some() {
                    intents.push(Intent::Undo(false));
                } else if redo_key && snap.undo.redo.is_some() {
                    intents.push(Intent::Undo(true));
                }
            }
        });
    }

    fn left(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        let Some(repo) = &self.repo else { return };
        let Some(snap) = &repo.snapshot else {
            ui.spinner();
            return;
        };
        // The full path is in the window title.
        let root = repo.worker.git.root().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let running = repo.worker.running();
        panels::status(ui, snap, &root, running.as_deref(), intents);
        ui.add_space(4.0);
        let h = ui.available_height();
        let spacing = ui.spacing().item_spacing.y;
        let share = |f: f32| (h * f - spacing).max(80.0);
        panels::files(ui, snap, &mut self.view, share(0.34), intents);
        panels::branches(ui, snap, &mut self.view, share(0.2), intents);
        panels::commits(ui, snap, &mut self.view, share(0.34), intents);
        panels::stash(ui, snap, &self.view, share(0.12), intents);
    }

    fn command_log(&self, ui: &mut egui::Ui) {
        let Some(repo) = &self.repo else { return };
        ui.horizontal(|ui| {
            ui.strong("Command log");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    repo.worker.log.clear();
                }
            });
        });
        egui::ScrollArea::vertical().id_salt("command-log").auto_shrink([false, false]).show(ui, |ui| {
            let entries = repo.worker.log.entries();
            if entries.is_empty() {
                ui.weak("The git commands Oxbow runs to change things are listed here.");
            }
            for entry in entries {
                let (icon, color) = match entry.status {
                    LogStatus::Running => ("…", ui.visuals().text_color()),
                    LogStatus::Done => ("✔", crate::theme::MERGED),
                    LogStatus::Failed => ("✖", crate::theme::CONFLICT),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color, icon);
                    ui.label(&entry.title);
                });
                for command in &entry.commands {
                    ui.add(egui::Label::new(RichText::new(format!("    {command}")).monospace().weak()).wrap_mode(egui::TextWrapMode::Wrap));
                }
            }
        });
    }

    fn detail(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        if let Some(error) = self.error.clone() {
            egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(crate::theme::CONFLICT, "✖");
                    ui.add(egui::Label::new(RichText::new(&error).monospace()).wrap_mode(egui::TextWrapMode::Wrap));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        let Some(repo) = &self.repo else {
            ui.centered_and_justified(|ui| ui.weak("Open a repository to get started."));
            return;
        };
        if let Some(e) = &repo.error {
            ui.colored_label(crate::theme::CONFLICT, e);
        }
        match &self.detail.current {
            _ if self.detail.loading && self.detail.current.is_none() => {
                ui.spinner();
            }
            Some(Ok(d)) => detail::show(ui, d, intents),
            Some(Err(e)) => {
                ui.colored_label(crate::theme::CONFLICT, e);
            }
            None => {
                ui.centered_and_justified(|ui| ui.weak("Select a file, commit, branch or stash to see it here."));
            }
        }
    }
}

#[cfg(test)]
impl App {
    /// The command log as text, newest first, for test failures.
    pub fn command_log_text(&self) -> Vec<String> {
        let Some(repo) = &self.repo else { return Vec::new() };
        repo.worker.log.entries().iter().map(|e| format!("{} {:?}: {}", e.title, e.status, e.commands.join(" ; "))).collect()
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        // Changes made outside Oxbow (an editor, a terminal) show up when the window is focused again.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.was_focused
            && let Some(repo) = &self.repo {
                repo.worker.refresh();
            }
        self.was_focused = focused;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut intents = Vec::new();
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &mut intents));
        egui::Panel::left("panels").resizable(true).default_size(460.0).min_size(320.0).show(ui, |ui| self.left(ui, &mut intents));
        if self.repo.is_some() {
            egui::Panel::bottom("log").resizable(true).default_size(150.0).min_size(60.0).show(ui, |ui| self.command_log(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.detail(ui, &mut intents));

        if let Some(dialog) = &mut self.dialog {
            match dialogs::show(ui.ctx(), dialog) {
                Outcome::Open => {}
                Outcome::Cancel => self.dialog = None,
                Outcome::Run(action) => {
                    self.dialog = None;
                    intents.push(Intent::Run(action));
                }
            }
        }
        for intent in intents {
            self.handle(intent);
        }
    }
}
