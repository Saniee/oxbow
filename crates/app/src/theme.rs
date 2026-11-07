//! Colours and small shared pieces of row rendering.

use eframe::egui::{self, Color32, RichText, TextFormat, text::LayoutJob};
use oxbow_engine::CommitMark;
use oxbow_engine::queries::Commit;

pub const MERGED: Color32 = Color32::from_rgb(0x4c, 0xb8, 0x64);
pub const PUSHED: Color32 = Color32::from_rgb(0xd8, 0xb4, 0x3c);
pub const UNPUSHED: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
pub const STAGED: Color32 = Color32::from_rgb(0x4c, 0xb8, 0x64);
pub const UNSTAGED: Color32 = Color32::from_rgb(0xe0, 0x8c, 0x3c);
pub const UNTRACKED: Color32 = Color32::from_rgb(0x6c, 0xa8, 0xe8);
pub const CONFLICT: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
pub const LOCAL_REF: Color32 = Color32::from_rgb(0x4c, 0xb8, 0x64);
pub const REMOTE_REF: Color32 = Color32::from_rgb(0x6c, 0xa8, 0xe8);
pub const TAG_REF: Color32 = Color32::from_rgb(0xd8, 0xb4, 0x3c);
pub const HUNK: Color32 = Color32::from_rgb(0x9c, 0x7c, 0xd8);

pub fn added_bg(dark: bool) -> Color32 {
    if dark { Color32::from_rgba_unmultiplied(46, 160, 67, 48) } else { Color32::from_rgba_unmultiplied(46, 160, 67, 40) }
}

pub fn removed_bg(dark: bool) -> Color32 {
    if dark { Color32::from_rgba_unmultiplied(248, 81, 73, 48) } else { Color32::from_rgba_unmultiplied(248, 81, 73, 36) }
}

pub fn mark_color(mark: CommitMark) -> Color32 {
    match mark {
        CommitMark::Merged => MERGED,
        CommitMark::Pushed => PUSHED,
        CommitMark::Unpushed => UNPUSHED,
    }
}

/// A ref decoration from `%D`: "HEAD -> main", "origin/main", "tag: v1".
pub fn ref_color(r: &str) -> Color32 {
    if r.starts_with("tag: ") {
        TAG_REF
    } else if r.contains('/') && !r.starts_with("HEAD -> ") {
        REMOTE_REF
    } else {
        LOCAL_REF
    }
}

pub fn ref_chip(ui: &mut egui::Ui, r: &str) {
    ui.label(RichText::new(r).small().color(ref_color(r)));
}

/// One commit as a single line: mark, short SHA, refs, subject, author and date.
pub fn commit_row(ui: &egui::Ui, c: &Commit, mark: Option<CommitMark>) -> LayoutJob {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let mut job = LayoutJob::default();
    let fmt = |font: &egui::FontId, color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    if let Some(mark) = mark {
        job.append("⏺ ", 0.0, fmt(&body, mark_color(mark)));
    }
    job.append(&c.short_sha, 0.0, fmt(&mono, weak));
    for r in &c.refs {
        job.append(&format!("[{r}]"), 6.0, fmt(&body, ref_color(r)));
    }
    job.append(&c.subject, 6.0, fmt(&body, text));
    job.append(&format!("{} · {}", c.author, c.rel_date), 10.0, fmt(&body, weak));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// git's two status letters, coloured: staged (X) and unstaged (Y).
pub fn status_letters(x: char, y: char, conflict: bool, untracked: bool, font: &egui::FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    if conflict {
        job.append(&format!("{x}{y}"), 0.0, fmt(CONFLICT));
    } else if untracked {
        job.append("??", 0.0, fmt(UNTRACKED));
    } else {
        job.append(&x.to_string(), 0.0, fmt(STAGED));
        job.append(&y.to_string(), 0.0, fmt(UNSTAGED));
    }
    job
}
