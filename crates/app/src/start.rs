//! The start screen: recently opened repositories, plus opening a folder or creating a new repository. Shown when
//! Oxbow starts without a repository and after "Projects" in the toolbar.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, RichText};

use crate::app::Intent;

const MAX_RECENT: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    /// Seconds since the Unix epoch when it was last opened.
    pub opened: u64,
}

/// Recently opened repositories, newest first, kept in `recent.txt` beside the settings (`<seconds>\t<path>` lines).
#[derive(Default)]
pub struct Projects {
    pub list: Vec<Recent>,
    path: Option<PathBuf>,
    pub filter: String,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Projects {
    pub fn load(path: Option<PathBuf>) -> Self {
        let text = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        Self { list: parse(&text), path, filter: String::new() }
    }

    fn save(&self) {
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, to_text(&self.list));
        }
    }

    /// Puts `root` at the top of the list.
    pub fn opened(&mut self, root: &Path) {
        self.list.retain(|r| r.path != root);
        self.list.insert(0, Recent { path: root.to_owned(), opened: now() });
        self.list.truncate(MAX_RECENT);
        self.save();
    }

    pub fn forget(&mut self, path: &Path) {
        self.list.retain(|r| r.path != path);
        self.save();
    }
}

fn parse(text: &str) -> Vec<Recent> {
    text.lines()
        .filter_map(|line| {
            let (opened, path) = line.split_once('\t')?;
            Some(Recent { path: PathBuf::from(path), opened: opened.parse().unwrap_or(0) })
        })
        .collect()
}

fn to_text(list: &[Recent]) -> String {
    list.iter().map(|r| format!("{}\t{}\n", r.opened, r.path.display())).collect()
}

/// "3 days ago", roughly.
fn ago(then: u64) -> String {
    let s = now().saturating_sub(then);
    let (n, unit) = match s {
        0..60 => return "just now".into(),
        60..3600 => (s / 60, "minute"),
        3600..86_400 => (s / 3600, "hour"),
        86_400..2_592_000 => (s / 86_400, "day"),
        2_592_000..31_536_000 => (s / 2_592_000, "month"),
        _ => (s / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

pub fn show(ui: &mut egui::Ui, projects: &mut Projects, intents: &mut Vec<Intent>) {
    let width = ui.available_width().min(760.0);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.08).min(60.0));
        ui.heading(RichText::new("Oxbow").size(28.0));
        ui.weak("Open a repository to work in");
        ui.add_space(16.0);
    });
    // A column `width` wide, centred, reaching to the bottom of the window so the list can use all of it.
    let rest = ui.available_rect_before_wrap();
    let column = egui::Rect::from_min_size(egui::pos2(rest.center().x - width / 2.0, rest.top()), egui::vec2(width, rest.height()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(column).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.horizontal(|ui| {
            ui.strong("Projects");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("New repository…").on_hover_text("Initialize a repository").clicked() {
                    intents.push(Intent::NewRepo(None));
                }
                if ui.button("Open folder…").clicked() {
                    intents.push(Intent::OpenRepo(None));
                }
                if projects.list.len() > 6 {
                    ui.add(egui::TextEdit::singleline(&mut projects.filter).hint_text("Filter").desired_width(160.0));
                }
            });
        });
        ui.separator();
        if projects.list.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.weak("Repositories you open show up here."));
            return;
        }
        let filter = projects.filter.to_lowercase();
        let mut forget = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for recent in &projects.list {
                let path = recent.path.display().to_string();
                if !filter.is_empty() && !path.to_lowercase().contains(&filter) {
                    continue;
                }
                let missing = !recent.path.exists();
                let name = recent.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
                let mut job = egui::text::LayoutJob::default();
                let body = egui::TextStyle::Body.resolve(ui.style());
                let (text, weak) = (ui.visuals().text_color(), ui.visuals().weak_text_color());
                let fmt = |size: f32, color| egui::TextFormat { font_id: egui::FontId::new(size, body.family.clone()), color, ..Default::default() };
                job.append(&name, 0.0, fmt(body.size + 2.0, if missing { weak } else { text }));
                job.append(&format!("\n{path}"), 0.0, fmt(body.size - 1.0, weak));
                let when = if missing { "Folder not found".to_owned() } else { ago(recent.opened) };
                // One button per row, the whole width: the name and path on the left, when it was opened on the right.
                let row = egui::Button::selectable(false, job).right_text(RichText::new(when).weak()).min_size(egui::vec2(ui.available_width(), 44.0));
                let r = ui.add(row);
                if r.clicked() && !missing {
                    intents.push(Intent::OpenRepo(Some(recent.path.clone())));
                }
                r.context_menu(|ui| {
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(path.clone());
                        ui.close();
                    }
                    if ui.button("Remove from the list").clicked() {
                        forget = Some(recent.path.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(path) = forget {
            projects.forget(&path);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_list_round_trips() {
        let list = vec![Recent { path: PathBuf::from("D:/work/oxbow"), opened: 1_700_000_000 }, Recent { path: PathBuf::from("/home/me/site"), opened: 5 }];
        assert_eq!(parse(&to_text(&list)), list);
        assert!(parse("garbage\n").is_empty());
    }

    /// The list once filled only a scroll area of egui's minimum height (64 pixels) under the buttons, so it showed one
    /// project and a half, and each row's name sat in the middle of a wide button.
    #[test]
    fn the_project_list_fills_the_window_and_its_rows_span_it() {
        use egui_kittest::kittest::Queryable;
        let mut projects = Projects::load(None);
        projects.list = (0..MAX_RECENT).map(|i| Recent { path: PathBuf::from(format!("Z:/nowhere/project-{i}")), opened: 0 }).collect();
        let size = egui::vec2(1280.0, 820.0);
        let harness = egui_kittest::Harness::builder().with_size(size).build_ui(move |ui| show(ui, &mut projects, &mut Vec::new()));

        // The scroll bar runs the height of the list, which reaches the bottom of the window.
        let bar = harness.get_by_role(egui::accesskit::Role::ScrollBar).rect();
        assert!(bar.height() > 400.0 && bar.bottom() > size.y - 20.0, "the list is {bar:?} in a window {size:?}");

        // A row is as wide as the column, its name on the left and the time on the right.
        let row = harness.get_by_label_contains("project-0").rect();
        let projects_label = harness.get_by_label("Projects").rect();
        assert!((row.left() - projects_label.left()).abs() < 2.0 && row.width() > 700.0, "row {row:?}, heading {projects_label:?}");
    }
}
