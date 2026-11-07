//! The right-hand pane: whatever is selected, such as a commit's message and patch, a file's changes or a branch's log.

use eframe::egui::{self, Color32, RichText, TextFormat, text::LayoutJob};
use oxbow_engine::diff::{self, DiffFile, LineKind};
use oxbow_engine::queries::{self, Commit, LogQuery, Stash};
use oxbow_engine::status::{self, StatusEntry};
use oxbow_engine::Git;

use crate::app::Intent;
use crate::theme;

/// Diffs longer than this are cut, so a huge commit stays responsive.
const MAX_DIFF_LINES: usize = 4000;

/// What is selected, by identity, so it survives a refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    File(String),
    Commit(String),
    Branch(String),
    Remote(String),
    Tag(String),
    Stash(String),
    Reflog(String),
}

/// What to load for a selection.
#[derive(Clone, Debug)]
pub enum Request {
    File(StatusEntry),
    Commit(Commit),
    /// A commit known only by SHA (from the reflog).
    Rev(String),
    /// The log of a branch, remote branch or tag.
    Log { title: String, rev: String },
    Stash(Stash),
}

pub enum Detail {
    File { entry: StatusEntry, staged: Vec<DiffFile>, unstaged: Vec<DiffFile>, omitted: usize },
    Commit { commit: Commit, message: String, files: Vec<DiffFile>, omitted: usize },
    Log { title: String, commits: Vec<Commit> },
    Stash { stash: Stash, files: Vec<DiffFile>, omitted: usize },
}

pub fn load(git: &Git, request: Request) -> Result<Detail, String> {
    let e = |e: oxbow_engine::GitError| e.to_string();
    Ok(match request {
        Request::File(entry) => {
            let mut staged = if entry.is_staged() || entry.is_conflict() { diff::parse_diff(&status::working_diff(git, &entry, true).map_err(e)?) } else { Vec::new() };
            let mut unstaged = if entry.is_unstaged() || entry.is_conflict() { diff::parse_diff(&status::working_diff(git, &entry, false).map_err(e)?) } else { Vec::new() };
            let omitted = diff::limit_diff(&mut staged, MAX_DIFF_LINES / 2) + diff::limit_diff(&mut unstaged, MAX_DIFF_LINES / 2);
            Detail::File { entry, staged, unstaged, omitted }
        }
        Request::Rev(sha) => {
            let found = queries::log(git, &LogQuery { limit: 1, rev: Some(sha.clone()), ..Default::default() }).map_err(e)?;
            let commit = found.into_iter().next().ok_or_else(|| format!("Commit {sha} not found"))?;
            return load(git, Request::Commit(commit));
        }
        Request::Commit(commit) => {
            let message = queries::commit_message(git, &commit.sha).map_err(e)?;
            let mut files = diff::parse_diff(&queries::commit_patch(git, &commit.sha, commit.parents.first().map(String::as_str)).map_err(e)?);
            let omitted = diff::limit_diff(&mut files, MAX_DIFF_LINES);
            Detail::Commit { commit, message, files, omitted }
        }
        Request::Log { title, rev } => {
            let commits = queries::log(git, &LogQuery { limit: 200, rev: Some(rev), ..Default::default() }).map_err(e)?;
            Detail::Log { title, commits }
        }
        Request::Stash(stash) => {
            let mut files = diff::parse_diff(&queries::stash_patch(git, &stash).map_err(e)?);
            let omitted = diff::limit_diff(&mut files, MAX_DIFF_LINES);
            Detail::Stash { stash, files, omitted }
        }
    })
}

pub fn show(ui: &mut egui::Ui, detail: &Detail, intents: &mut Vec<Intent>) {
    egui::ScrollArea::both().id_salt("detail").auto_shrink([false, false]).show(ui, |ui| match detail {
        Detail::File { entry, staged, unstaged, omitted } => {
            ui.heading(&entry.path);
            if let Some(old) = &entry.old_path {
                ui.weak(format!("renamed from {old}"));
            }
            if entry.is_conflict() {
                ui.colored_label(theme::CONFLICT, "Conflicted: resolve it in your editor, then stage it to mark it resolved.");
            }
            if !staged.is_empty() {
                ui.add_space(6.0);
                ui.strong("Staged");
                diff_view(ui, staged, "staged");
            }
            if !unstaged.is_empty() {
                ui.add_space(6.0);
                ui.strong(if entry.is_untracked() { "Untracked" } else { "Unstaged" });
                diff_view(ui, unstaged, "unstaged");
            }
            omitted_note(ui, *omitted);
        }
        Detail::Commit { commit, message, files, omitted } => {
            let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
            ui.heading(subject);
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.monospace(&commit.sha);
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(commit.sha.clone());
                }
            });
            ui.label(format!("{} <{}> · {} ({})", commit.author, commit.author_email, commit.rel_date, commit.date));
            for co in &commit.co_authors {
                ui.weak(format!("Co-author: {} <{}>", co.name, co.email));
            }
            if commit.parents.len() > 1 {
                ui.weak(format!("Merge of {}; the patch is against the first parent.", commit.parents.iter().map(|p| &p[..p.len().min(7)]).collect::<Vec<_>>().join(", ")));
            }
            if !commit.refs.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for r in &commit.refs {
                        theme::ref_chip(ui, r);
                    }
                });
            }
            let body = body.trim();
            if !body.is_empty() {
                ui.add_space(6.0);
                ui.label(body);
            }
            ui.add_space(6.0);
            ui.separator();
            if files.is_empty() {
                ui.weak("No changes");
            }
            diff_view(ui, files, &commit.sha);
            omitted_note(ui, *omitted);
        }
        Detail::Log { title, commits } => {
            ui.heading(title);
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for c in commits {
                    let row = ui.selectable_label(false, theme::commit_row(ui, c, None));
                    if row.clicked() {
                        intents.push(Intent::Select(Selection::Commit(c.sha.clone()), Request::Commit(c.clone())));
                    }
                }
            });
        }
        Detail::Stash { stash, files, omitted } => {
            ui.heading(&stash.message);
            ui.weak(format!("{} · {}", stash.selector, stash.rel_date));
            ui.add_space(6.0);
            ui.separator();
            diff_view(ui, files, &stash.sha);
            omitted_note(ui, *omitted);
        }
    });
}

fn omitted_note(ui: &mut egui::Ui, omitted: usize) {
    if omitted > 0 {
        ui.add_space(6.0);
        ui.weak(format!("{omitted} more lines not shown."));
    }
}

/// Files with their hunks; each file can be collapsed.
pub fn diff_view(ui: &mut egui::Ui, files: &[DiffFile], salt: &str) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let dark = ui.visuals().dark_mode;
    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    for (i, file) in files.iter().enumerate() {
        let mut title = format!("{}  {}", file.status, file.path);
        if let Some(old) = &file.old_path {
            title = format!("{}  {old} ➡ {}", file.status, file.path);
        }
        let header = RichText::new(format!("{title}   +{} −{}", file.added, file.removed)).strong();
        egui::CollapsingHeader::new(header).id_salt((salt, i)).default_open(true).show(ui, |ui| {
            for note in &file.notes {
                ui.weak(note);
            }
            if file.binary {
                ui.weak("Binary file");
                return;
            }
            for hunk in &file.hunks {
                let mut job = LayoutJob::default();
                let fmt = |color: Color32, background: Color32| TextFormat { font_id: font.clone(), color, background, ..Default::default() };
                job.append(&format!("{}\n", hunk.header), 0.0, fmt(theme::HUNK, Color32::TRANSPARENT));
                for line in &hunk.lines {
                    let num = |n: Option<u32>| n.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                    let (sign, bg) = match line.kind {
                        LineKind::Added => ('+', theme::added_bg(dark)),
                        LineKind::Removed => ('-', theme::removed_bg(dark)),
                        LineKind::Context => (' ', Color32::TRANSPARENT),
                        LineKind::Note => ('\\', Color32::TRANSPARENT),
                    };
                    job.append(&format!("{} {} ", num(line.old), num(line.new)), 0.0, fmt(weak, bg));
                    let color = if line.kind == LineKind::Note { weak } else { text_color };
                    job.append(&format!("{sign} {}\n", line.text), 0.0, fmt(color, bg));
                }
                ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
            }
        });
    }
}
