//! The right-hand pane: whatever is selected, such as a commit's message and patch, a file's changes or a branch's log.

use eframe::egui::{self, Color32, RichText, TextFormat, text::LayoutJob};
use oxbow_engine::diff::{self, DiffFile, LineKind};
use oxbow_engine::queries::{self, Commit, LogQuery, Stash};
use oxbow_engine::status::{self, StatusEntry};
use oxbow_engine::Git;
use oxbow_engine::patch::{self, Picks};
use std::collections::BTreeMap;

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

/// lazygit's custom patch: lines picked from one commit's diff, across its files, kept until acted on.
#[derive(Clone, Debug)]
pub struct CustomPatch {
    pub sha: String,
    pub subject: String,
    /// By path: the file's diff in the commit and the lines picked in it.
    pub files: BTreeMap<String, (DiffFile, Picks)>,
}

impl CustomPatch {
    /// Changed lines picked, once change blocks are filled in.
    pub fn lines(&self) -> usize {
        self.files.values().map(|(f, p)| patch::picked_lines(f, p)).sum()
    }

    /// The patch that takes the picked lines out of the commit, all files together.
    pub fn removal(&self) -> Result<String, String> {
        let mut out = String::new();
        for (file, picks) in self.files.values() {
            if let Some(p) = patch::removal_patch(file, picks)? {
                out.push_str(&p);
            }
        }
        if out.is_empty() { Err("The patch has no changes in it.".into()) } else { Ok(out) }
    }
}

/// Picking lines for a custom patch, in the commit shown.
#[derive(Default)]
pub struct Picking {
    /// Clicking a changed line picks it.
    pub on: bool,
    pub patch: Option<CustomPatch>,
}

impl Picking {
    /// The picks for a file of `sha`, starting a new patch if the current one is from another commit.
    fn file_mut(&mut self, sha: &str, subject: &str, file: &DiffFile) -> &mut Picks {
        if self.patch.as_ref().is_none_or(|p| p.sha != sha) {
            self.patch = Some(CustomPatch { sha: sha.to_owned(), subject: subject.to_owned(), files: BTreeMap::new() });
        }
        let patch = self.patch.as_mut().unwrap();
        &mut patch.files.entry(file.path.clone()).or_insert_with(|| (file.clone(), Picks::new())).1
    }

    /// Drops files left without picks, and the patch when nothing is left.
    fn tidy(&mut self) {
        if let Some(p) = &mut self.patch {
            p.files.retain(|_, (_, picks)| !picks.is_empty());
            if p.files.is_empty() {
                self.patch = None;
            }
        }
    }

    fn picks(&self, sha: &str, path: &str) -> Option<&Picks> {
        self.patch.as_ref().filter(|p| p.sha == sha).and_then(|p| p.files.get(path)).map(|(_, picks)| picks)
    }
}

/// What a diff view needs to let lines be picked.
struct Picker<'a> {
    sha: &'a str,
    subject: &'a str,
    picking: &'a mut Picking,
}

pub fn show(ui: &mut egui::Ui, detail: &Detail, intents: &mut Vec<Intent>, picking: &mut Picking) {
    if let Detail::Log { title, commits } = detail {
        ui.heading(title);
        ui.add_space(4.0);
        let height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical().id_salt("detail-log").auto_shrink([false, false]).show_rows(ui, height, commits.len(), |ui, range| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for c in &commits[range] {
                    let row = ui.selectable_label(false, theme::commit_row(ui, c, None));
                    if row.clicked() {
                        intents.push(Intent::Select(Selection::Commit(c.sha.clone()), Request::Commit(c.clone())));
                    }
                }
            });
        });
        return;
    }
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
                diff_view(ui, staged, "staged", None);
            }
            if !unstaged.is_empty() {
                ui.add_space(6.0);
                ui.strong(if entry.is_untracked() { "Untracked" } else { "Unstaged" });
                diff_view(ui, unstaged, "unstaged", None);
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
            if !files.is_empty() && !commit.is_merge() {
                ui.horizontal(|ui| {
                    ui.toggle_value(&mut picking.on, "✂ Pick lines")
                        .on_hover_text("Click changed lines to collect them in a patch: then take them out of this commit, move them to another, or apply them to your working tree");
                    if picking.on {
                        ui.weak("click a changed line to pick it; its block's other side comes along");
                    }
                });
            }
            ui.separator();
            if files.is_empty() {
                ui.weak("No changes");
            }
            let picker = (!commit.is_merge()).then_some(Picker { sha: &commit.sha, subject: &commit.subject, picking });
            diff_view(ui, files, &commit.sha, picker);
            omitted_note(ui, *omitted);
        }
        Detail::Log { .. } => {}
        Detail::Stash { stash, files, omitted } => {
            ui.heading(&stash.message);
            ui.weak(format!("{} · {}", stash.selector, stash.rel_date));
            ui.add_space(6.0);
            ui.separator();
            diff_view(ui, files, &stash.sha, None);
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

/// Generated files nobody reads line by line; their diffs start collapsed.
fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".lock") || matches!(name, "package-lock.json" | "pnpm-lock.yaml" | "npm-shrinkwrap.json" | "go.sum")
}

/// Files with their hunks; each file can be collapsed. With a picker, lines can be picked for a custom patch.
fn diff_view(ui: &mut egui::Ui, files: &[DiffFile], salt: &str, mut picker: Option<Picker>) {
    for (i, file) in files.iter().enumerate() {
        let mut title = format!("{}  {}", file.status, file.path);
        if let Some(old) = &file.old_path {
            title = format!("{}  {old} ➡ {}", file.status, file.path);
        }
        let generated = is_generated(&file.path);
        let mut header = format!("{title}   +{} −{}", file.added, file.removed);
        if generated {
            header.push_str("   (generated)");
        }
        // Lines of this file in the patch, with their blocks filled in.
        let picked = picker.as_ref().and_then(|p| p.picking.picks(p.sha, &file.path)).map(|picks| (picks.clone(), patch::effective(file, picks)));
        if let Some((_, effective)) = &picked {
            header.push_str(&format!("   ✂ {} picked", effective.len()));
        }
        let splittable = !file.binary && file.status != 'D';
        let response = egui::CollapsingHeader::new(RichText::new(header).strong()).id_salt((salt, i, &file.path)).default_open(!generated).show(ui, |ui| {
            for note in &file.notes {
                ui.weak(note);
            }
            if file.binary {
                ui.weak("Binary file");
                return;
            }
            for (h, hunk) in file.hunks.iter().enumerate() {
                let clicked = hunk_view(ui, hunk, h, picker.as_ref().filter(|p| p.picking.on && splittable).map(|_| picked.as_ref().map(|(_, e)| e)));
                if let (Some(line), Some(p)) = (clicked, picker.as_mut()) {
                    let picks = p.picking.file_mut(p.sha, p.subject, file);
                    if !picks.remove(&(h, line)) {
                        picks.insert((h, line));
                    }
                    p.picking.tidy();
                }
            }
        });
        if let (Some(p), true) = (picker.as_mut(), splittable) {
            response.header_response.context_menu(|ui| {
                let all = patch::all_changes(file);
                let whole = picked.as_ref().is_some_and(|(_, e)| *e == all);
                if !whole && ui.button("Add the whole file to the patch").clicked() {
                    *p.picking.file_mut(p.sha, p.subject, file) = all;
                    ui.close();
                }
                if picked.is_some() && ui.button("Take the file out of the patch").clicked() {
                    p.picking.file_mut(p.sha, p.subject, file).clear();
                    p.picking.tidy();
                    ui.close();
                }
            });
        }
    }
}

/// One hunk. It takes the room of all its lines, but only the lines in view are laid out and drawn, so a diff
/// thousands of lines long scrolls as smoothly as a short one. With `picking` (the file's picked lines, if any),
/// changed lines can be clicked; returns the index of the one clicked.
fn hunk_view(ui: &mut egui::Ui, hunk: &oxbow_engine::diff::Hunk, h: usize, picking: Option<Option<&Picks>>) -> Option<usize> {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    let char_width = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    // Line numbers and sign: "{old:>5} {new:>5} + ".
    let widest = hunk.lines.iter().map(|l| l.text.chars().count() + 14).max().unwrap_or(0).max(hunk.header.chars().count());
    let rows = hunk.lines.len() + 1;
    let width = (widest as f32 * char_width).max(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, rows as f32 * row), egui::Sense::hover());

    let clip = ui.clip_rect();
    if !clip.intersects(rect) {
        return None;
    }
    let first = (((clip.top() - rect.top()) / row).floor().max(0.0) as usize).min(rows);
    let last = (((clip.bottom() - rect.top()) / row).ceil().max(0.0) as usize).min(rows);
    if first >= last {
        return None;
    }

    // Picking: each visible changed line is clickable, and picked ones are marked in the gutter.
    let mut clicked = None;
    if let Some(picked) = picking {
        for i in first.max(1)..last {
            let j = i - 1;
            if !matches!(hunk.lines[j].kind, LineKind::Added | LineKind::Removed) {
                continue;
            }
            let line_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + i as f32 * row), egui::vec2(rect.width(), row));
            let response = ui.interact(line_rect, ui.scope_id().with(("pick", h, j)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let is_picked = picked.is_some_and(|p| p.contains(&(h, j)));
            if is_picked {
                ui.painter().rect_filled(line_rect, 0.0, ui.visuals().selection.bg_fill.gamma_multiply(0.45));
                ui.painter().rect_filled(egui::Rect::from_min_size(line_rect.min, egui::vec2(3.0, row)), 0.0, theme::HUNK);
            } else if response.hovered() {
                ui.painter().rect_filled(line_rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill.gamma_multiply(0.5));
            }
            if response.clicked() {
                clicked = Some(j);
            }
        }
    }

    let dark = ui.visuals().dark_mode;
    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let fmt = |color: Color32, background: Color32| TextFormat { font_id: font.clone(), color, background, ..Default::default() };
    let mut job = LayoutJob::default();
    for i in first..last {
        let Some(line) = i.checked_sub(1).map(|j| &hunk.lines[j]) else {
            job.append(&format!("{}\n", hunk.header), 0.0, fmt(theme::HUNK, Color32::TRANSPARENT));
            continue;
        };
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
    let top = rect.top() + first as f32 * row;
    let visible = egui::Rect::from_min_max(egui::pos2(rect.left(), top), egui::pos2(rect.right(), rect.top() + last as f32 * row));
    let place = egui::UiBuilder::new().max_rect(visible).layout(egui::Layout::top_down(egui::Align::Min));
    // While picking, the text must not take the clicks for selecting it.
    ui.scope_builder(place, |ui| ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend).selectable(picking.is_none())));
    clicked
}
