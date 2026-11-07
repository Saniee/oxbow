//! The window: panels on the left, the selection's detail on the right, the command log below it.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use oxbow_engine::rebase::Rewrite;
use oxbow_engine::{Git, LogStatus, Snapshot, queries, undo};

use crate::detail::{self, Detail, Request, Selection};
use crate::dialogs::{self, Dialog, Outcome};
use crate::panels::{self, View};
use crate::settings::{self, Prefs};
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
    CherryPicks(CherryPicks),
    /// Forget the custom patch: it was used, or the commits it came from are gone.
    ClearPatch,
    Refresh,
    /// Open a repository: this one, or ask for one.
    OpenRepo(Option<PathBuf>),
    /// git init: in this folder, or ask for one.
    NewRepo(Option<PathBuf>),
    /// Close the repository and show the project list.
    Projects,
    Settings,
}

/// Changes to the list of commits copied for cherry-picking.
pub enum CherryPicks {
    /// SHA and subject, oldest first.
    Add(Vec<(String, String)>),
    Remove(Vec<(String, String)>),
    Clear,
}

struct Repo {
    /// Declared first so it is dropped before the worker: it holds the worker's job channel.
    watcher: Option<oxbow_engine::watch::Watcher>,
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
    prefs: Prefs,
    /// Where the preferences are saved; None in tests.
    prefs_path: Option<PathBuf>,
    settings: Option<settings::Editor>,
    projects: crate::start::Projects,
    /// Oxbow was started on a folder that is not a repository: offer to create one, and quit if not.
    offer_init: Option<PathBuf>,
    /// A folder picked with Open folder… that is not a repository: offer to create one, without quitting.
    ask_init: Option<PathBuf>,
}

impl App {
    /// The app with the saved preferences.
    pub fn new(ctx: egui::Context, start: Option<&Path>) -> Self {
        let path = Prefs::default_path();
        let prefs = path.as_deref().map(Prefs::load).unwrap_or_default();
        Self::with_prefs(ctx, start, prefs, path)
    }

    pub fn with_prefs(ctx: egui::Context, start: Option<&Path>, prefs: Prefs, prefs_path: Option<PathBuf>) -> Self {
        let mut app = App {
            ctx,
            repo: None,
            view: View::default(),
            detail: DetailState::default(),
            dialog: None,
            error: None,
            was_focused: true,
            projects: crate::start::Projects::load(prefs_path.as_deref().map(|p| p.with_file_name("recent.txt"))),
            prefs,
            prefs_path,
            settings: None,
            offer_init: None,
            ask_init: None,
        };
        if let Some(path) = start {
            match Git::open(path) {
                Ok(git) => app.open(git),
                Err(e) if path.join(".git").exists() => app.error = Some(e.to_string()),
                Err(_) if path.is_dir() => app.offer_init = Some(path.to_owned()),
                Err(_) => app.error = Some(format!("{} does not exist.", path.display())),
            }
        }
        app
    }

    fn open(&mut self, git: Git) {
        let title = format!("Oxbow — {}", git.root().display());
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        self.projects.opened(git.root());
        let worker = Worker::new(git, self.ctx.clone(), LoadOptions { settings: self.prefs.engine.clone(), search: None });
        self.repo = Some(Repo { watcher: None, worker, snapshot: None, error: None });
        self.update_watcher();
        // Commits copied for cherry-picking come along: worktrees of one repository share its commits.
        let cherry_picks = std::mem::take(&mut self.view.cherry_picks);
        self.view = View { cherry_picks, ..View::default() };
        self.detail = DetailState::default();
        self.error = None;
    }

    fn pick_repo(&mut self) {
        let Some(dir) = rfd::FileDialog::new().set_title("Open repository").pick_folder() else { return };
        match Git::open(&dir) {
            Ok(git) => self.open(git),
            Err(_) => self.ask_init = Some(dir),
        }
    }

    fn init_repo(&mut self, dir: &Path) {
        match oxbow_engine::ops::init(dir) {
            Ok(git) => self.open(git),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn close_repo(&mut self) {
        self.repo = None;
        self.detail = DetailState::default();
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title("Oxbow".into()));
    }

    fn select(&mut self, selection: Selection, request: Request) {
        self.view.selected_commits = match &selection {
            Selection::Commit(sha) => std::iter::once(sha.clone()).collect(),
            _ => Default::default(),
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

    /// After a status-only refresh: the selected file's diff may have changed, nothing else can have.
    fn refresh_file_detail(&mut self) {
        if matches!(self.view.selection, Some(Selection::File(_))) {
            self.refresh_detail();
        }
    }

    /// After a refresh: reload what is shown when it may have changed, or clear it when it is gone.
    fn refresh_detail(&mut self) {
        let Some(snap) = self.repo.as_ref().and_then(|r| r.snapshot.as_ref()) else { return };
        // Rewritten commits are gone from the list.
        if !self.view.selected_commits.is_empty() {
            let listed: std::collections::HashSet<&str> = snap.commits.iter().map(|c| c.sha.as_str()).collect();
            self.view.selected_commits.retain(|sha| listed.contains(sha.as_str()));
        }
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
                    if let Some(repo) = &self.repo {
                        repo.worker.read(
                            |g| queries::commit_message(g, "HEAD"),
                            |app, message| {
                                // Unless amending was turned off, or something was typed, in the meantime.
                                if let Ok(message) = message
                                    && app.view.amend
                                    && app.view.commit_message.trim().is_empty()
                                {
                                    app.view.commit_message = message;
                                }
                            },
                        );
                    }
                } else if !on {
                    self.view.commit_message.clear();
                }
            }
            Intent::ToggleCommit(sha) => {
                if !self.view.selected_commits.remove(&sha) {
                    self.view.selected_commits.insert(sha);
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
            Intent::OpenRepo(None) => self.pick_repo(),
            Intent::NewRepo(None) => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Folder for the new repository").pick_folder() {
                    self.init_repo(&dir);
                }
            }
            Intent::NewRepo(Some(dir)) => self.init_repo(&dir),
            Intent::Projects => self.close_repo(),
            Intent::OpenRepo(Some(path)) => match Git::open(&path) {
                Ok(git) => self.open(git),
                Err(e) => self.error = Some(e.to_string()),
            },
            Intent::ClearPatch => self.view.picking = detail::Picking::default(),
            Intent::CherryPicks(change) => {
                let list = &mut self.view.cherry_picks;
                match change {
                    CherryPicks::Add(picks) => {
                        for p in picks {
                            if !list.iter().any(|(s, _)| *s == p.0) {
                                list.push(p);
                            }
                        }
                    }
                    CherryPicks::Remove(picks) => list.retain(|(s, _)| !picks.iter().any(|p| p.0 == *s)),
                    CherryPicks::Clear => list.clear(),
                }
            }
            Intent::Settings => self.settings = Some(settings::Editor::new(&self.prefs)),
        }
    }

    /// Reads the commit's message in the background, then opens the dialog.
    fn reword_dialog(&mut self, sha: String, warning: Option<String>, paused: bool) {
        let Some(repo) = &self.repo else { return };
        let read = sha.clone();
        repo.worker.read(
            move |g| queries::commit_message(g, &read),
            move |app, message| match message {
                Ok(message) if app.dialog.is_none() => app.show_reword(sha, message, warning, paused),
                Ok(_) => {}
                Err(e) => app.error = Some(e.to_string()),
            },
        );
    }

    fn show_reword(&mut self, sha: String, message: String, warning: Option<String>, paused: bool) {
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

    /// Starts or stops watching the repository's files, as the preferences say.
    fn update_watcher(&mut self) {
        let Some(repo) = &mut self.repo else { return };
        if !self.prefs.watch_files {
            repo.watcher = None;
        } else if repo.watcher.is_none() {
            match oxbow_engine::watch::watch(&repo.worker.git, repo.worker.refresher()) {
                Ok(w) => repo.watcher = Some(w),
                // Not fatal: refreshing on focus still works.
                Err(e) => repo.error = Some(format!("Could not watch the files for changes: {e}")),
            }
        }
    }

    /// Applies changed preferences: saves them and reloads with them.
    fn apply_prefs(&mut self, prefs: Prefs) {
        if prefs == self.prefs {
            return;
        }
        if let Some(path) = &self.prefs_path
            && let Err(e) = prefs.save(path)
        {
            self.error = Some(format!("Could not save the settings to {}: {e}", path.display()));
        }
        let reload = prefs.engine != self.prefs.engine;
        self.prefs = prefs;
        self.update_watcher();
        if reload && let Some(repo) = &self.repo {
            repo.worker.options.lock().unwrap().settings = self.prefs.engine.clone();
            repo.worker.refresh();
        }
    }

    fn poll(&mut self) {
        let Some(repo) = &mut self.repo else { return };
        let mut refreshed = false;
        let mut status_only = false;
        let mut replies = Vec::new();
        for event in repo.worker.poll() {
            match event {
                Event::Snapshot(Ok(snap)) => {
                    if let Some(old) = repo.snapshot.replace(*snap) {
                        std::thread::spawn(move || drop(old));
                    }
                    repo.error = None;
                    refreshed = true;
                }
                Event::Snapshot(Err(e)) => repo.error = Some(e),
                Event::Status(Ok(status)) => {
                    if let Some(snap) = &mut repo.snapshot {
                        snap.status = status;
                        status_only = true;
                    }
                }
                Event::Status(Err(e)) => repo.error = Some(e),
                Event::Done { title, error } => match error {
                    Some(e) => self.error = Some(e),
                    None => {
                        if title == "Committing…" || title == "Amending…" {
                            self.view.commit_message.clear();
                            self.view.amend = false;
                        }
                        // The patch was used, and the commit it came from is rewritten.
                        if PATCH_ACTIONS.contains(&title.as_str()) {
                            self.view.picking = detail::Picking::default();
                        }
                    }
                },
                Event::Detail { generation, result } => {
                    if generation == self.detail.generation {
                        self.detail.loading = false;
                        self.detail.current = Some(result);
                    }
                }
                Event::Reply(reply) => replies.push(reply),
            }
        }
        for reply in replies {
            reply(self);
        }
        if refreshed {
            self.refresh_detail();
        } else if status_only {
            self.refresh_file_detail();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        ui.horizontal(|ui| {
            if self.repo.is_some() && ui.button("Projects").on_hover_text("Close this repository and pick another").clicked() {
                intents.push(Intent::Projects);
            }
            if self.repo.is_some() && ui.button("⟳ Refresh").clicked() {
                intents.push(Intent::Refresh);
            }
            if ui.button("⚙ Settings").clicked() {
                intents.push(Intent::Settings);
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
        self.view.confirm_rewriting_pushed = self.prefs.confirm_rewriting_pushed;
        panels::status(ui, snap, &root, running.as_deref(), intents);
        ui.add_space(4.0);
        let spacing = ui.spacing().item_spacing.y;
        // What the four sections share, after the three handles between them.
        let usable = (ui.available_height() - 3.0 * (SPLITTER + spacing) - 4.0 * spacing).max(4.0 * MIN_SECTION);
        let shares = &mut self.prefs.panel_sizes;
        let height = |i: usize, shares: &[f32; 4]| (usable * shares[i]).max(MIN_SECTION);
        let mut released = false;
        panels::files(ui, snap, &mut self.view, height(0, shares), intents);
        released |= splitter(ui, shares, 0, usable);
        panels::branches(ui, snap, &mut self.view, height(1, shares), intents);
        released |= splitter(ui, shares, 1, usable);
        panels::commits(ui, snap, &mut self.view, height(2, shares), intents);
        released |= splitter(ui, shares, 2, usable);
        panels::stash(ui, snap, &self.view, height(3, shares), intents);
        if released && let Some(path) = &self.prefs_path {
            let _ = self.prefs.save(path);
        }
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

    /// The last failed action, until dismissed.
    fn error_banner(&mut self, ui: &mut egui::Ui) {
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
    }

    fn detail(&mut self, ui: &mut egui::Ui, intents: &mut Vec<Intent>) {
        self.error_banner(ui);
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
            Some(Ok(d)) => detail::show(ui, d, intents, &mut self.view.picking),
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
    /// HEAD as the panels show it.
    pub fn shown_head(&self) -> Option<String> {
        self.repo.as_ref().and_then(|r| r.snapshot.as_ref()).and_then(|s| s.head.clone())
    }

    pub fn is_running(&self) -> bool {
        self.repo.as_ref().is_some_and(|r| r.worker.running().is_some())
    }

    pub fn command_log_text(&self) -> Vec<String> {
        let Some(repo) = &self.repo else { return Vec::new() };
        repo.worker.log.entries().iter().map(|e| format!("{} {:?}: {}", e.title, e.status, e.commands.join(" ; "))).collect()
    }
}

/// Titles of the actions that use up the custom patch.
pub const PATCH_ACTIONS: [&str; 3] = ["Taking the lines out of the commit…", "Splitting the commit…", "Moving the lines…"];

const SPLITTER: f32 = 6.0;
const MIN_SECTION: f32 = 70.0;

/// The handle between sections `i` and `i + 1`: dragging moves height from one to the other, double-clicking puts
/// all four back to their default sizes. True when a drag ends, so the sizes can be saved.
fn splitter(ui: &mut egui::Ui, shares: &mut [f32; 4], i: usize, usable: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), SPLITTER), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let color = if response.hovered() || response.dragged() { ui.visuals().widgets.hovered.bg_stroke.color } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    ui.painter().hline(rect.x_range().shrink(rect.width() * 0.4), rect.center().y, egui::Stroke::new(2.0, color));
    if response.dragged() {
        let min = MIN_SECTION / usable;
        let pair = shares[i] + shares[i + 1];
        let moved = (shares[i] + response.drag_delta().y / usable).clamp(min, (pair - min).max(min));
        shares[i] = moved;
        shares[i + 1] = pair - moved;
    }
    if response.double_clicked() {
        *shares = Prefs::default().panel_sizes;
        return true;
    }
    response.drag_stopped()
}

/// "Create a repository in <dir>?": Some(true) to create, Some(false) for `no`, None while unanswered.
fn init_question(ctx: &egui::Context, dir: &Path, no: &str) -> Option<bool> {
    let mut answer = None;
    egui::Modal::new(egui::Id::unique("init-question")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("Not a Git repository");
        ui.add_space(4.0);
        ui.label(format!("{} is not a Git repository. Create one there (git init)?", dir.display()));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Create repository").clicked() {
                answer = Some(true);
            }
            if ui.button(no).clicked() {
                answer = Some(false);
            }
        });
    });
    answer
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
        if self.repo.is_some() {
            egui::Panel::left("panels").resizable(true).default_size(460.0).min_size(320.0).show(ui, |ui| self.left(ui, &mut intents));
            egui::Panel::bottom("log").resizable(true).default_size(150.0).min_size(60.0).show(ui, |ui| self.command_log(ui));
            egui::CentralPanel::default().show(ui, |ui| self.detail(ui, &mut intents));
        } else {
            egui::CentralPanel::default().show(ui, |ui| {
                self.error_banner(ui);
                crate::start::show(ui, &mut self.projects, &mut intents);
            });
        }
        if let Some(dir) = self.offer_init.clone() {
            match init_question(ui.ctx(), &dir, "Quit") {
                Some(true) => {
                    self.offer_init = None;
                    self.init_repo(&dir);
                }
                Some(false) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                None => {}
            }
        }
        if let Some(dir) = self.ask_init.clone() {
            match init_question(ui.ctx(), &dir, "Cancel") {
                Some(true) => {
                    self.ask_init = None;
                    self.init_repo(&dir);
                }
                Some(false) => self.ask_init = None,
                None => {}
            }
        }

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
        if let Some(editor) = &mut self.settings {
            match settings::show(ui.ctx(), editor, self.prefs_path.as_deref()) {
                settings::Outcome::Open => {}
                settings::Outcome::Close => self.settings = None,
                settings::Outcome::Changed => {
                    let mut draft = editor.draft.clone();
                    // Resized while the window was open: the sizes are not the window's to change.
                    draft.panel_sizes = self.prefs.panel_sizes;
                    self.apply_prefs(draft);
                }
            }
        }
        for intent in intents {
            self.handle(intent);
        }
    }
}
