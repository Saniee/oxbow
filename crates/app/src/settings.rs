//! Preferences: what the engine loads with, plus how the app behaves. Kept in a small `key = value` file in the
//! user's config folder (%APPDATA%\Oxbow on Windows, ~/.config/oxbow elsewhere), so it is easy to read and edit.

use std::path::{Path, PathBuf};

use eframe::egui;
use oxbow_engine::Settings;
use oxbow_engine::queries::BranchSort;
use oxbow_engine::rebase::UpdateRefs;

use crate::theme;

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub engine: Settings,
    /// Ask before rewriting commits that are already pushed or on a main branch.
    pub confirm_rewriting_pushed: bool,
    /// Refresh when files in the repository change, not only when the window gets focus.
    pub watch_files: bool,
    /// How the left column's height is shared by Files, Branches, Commits and Stash; adds up to 1.
    pub panel_sizes: [f32; 4],
    pub theme: theme::Choice,
    /// Ask GitHub for a newer version when Oxbow starts.
    pub check_updates: bool,
    /// Download and install bug-fix versions (same major and minor) without asking.
    pub auto_update: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { engine: Settings::default(), confirm_rewriting_pushed: true, watch_files: true, panel_sizes: [0.34, 0.2, 0.34, 0.12], theme: theme::Choice::System, check_updates: true, auto_update: true }
    }
}

const SORTS: [(BranchSort, &str, &str); 3] = [(BranchSort::Recency, "recency", "Recently checked out"), (BranchSort::Date, "date", "Last commit date"), (BranchSort::Name, "name", "Name")];
const UPDATE_REFS: [(UpdateRefs, &str, &str); 3] = [
    (UpdateRefs::GitConfig, "git-config", "As git's rebase.updateRefs says"),
    (UpdateRefs::Always, "always", "Always"),
    (UpdateRefs::Never, "never", "Never"),
];

fn key_of<T: PartialEq>(options: &[(T, &'static str, &str)], value: T) -> &'static str {
    options.iter().find(|o| o.0 == value).map_or("", |o| o.1)
}

fn bool_text(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

impl Prefs {
    /// Where the preferences live, or None when there is no config folder.
    pub fn default_path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        };
        base.map(|b| b.join(if cfg!(windows) { "Oxbow" } else { "oxbow" }).join("settings.txt"))
    }

    /// The saved preferences; defaults for anything missing or unreadable.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut p = Prefs::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let flag = || value.eq_ignore_ascii_case("true");
            match key.trim() {
                "commit_limit" => p.engine.commit_limit = value.parse().unwrap_or(p.engine.commit_limit).clamp(10, 100_000),
                "reflog_limit" => p.engine.reflog_limit = value.parse().unwrap_or(p.engine.reflog_limit).clamp(10, 100_000),
                "main_branches" => p.engine.main_branches = value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
                "branch_sort" => p.engine.branch_sort = SORTS.iter().find(|s| s.1 == value).map_or(p.engine.branch_sort, |s| s.0),
                "update_refs" => p.engine.update_refs = UPDATE_REFS.iter().find(|s| s.1 == value).map_or(p.engine.update_refs, |s| s.0),
                "confirm_rewriting_pushed" => p.confirm_rewriting_pushed = flag(),
                "watch_files" => p.watch_files = flag(),
                "check_updates" => p.check_updates = flag(),
                "auto_update" => p.auto_update = flag(),
                "theme" => p.theme = theme::THEMES.iter().find(|t| t.1 == value).map_or(p.theme, |t| t.0),
                "panel_sizes" => {
                    let sizes: Vec<f32> = value.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    let total: f32 = sizes.iter().sum();
                    if let ([a, b, c, d], true) = (sizes.as_slice(), sizes.iter().all(|s| *s > 0.0)) {
                        p.panel_sizes = [a / total, b / total, c / total, d / total];
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn to_text(&self) -> String {
        let e = &self.engine;
        let (sort, update) = (key_of(&SORTS, e.branch_sort), key_of(&UPDATE_REFS, e.update_refs));
        format!(
            "# Oxbow settings. Edit here or in the app's Settings window.\n\
             commit_limit = {}\nreflog_limit = {}\nmain_branches = {}\nbranch_sort = {sort}\nupdate_refs = {update}\n\
             confirm_rewriting_pushed = {}\nwatch_files = {}\npanel_sizes = {}\ntheme = {}\n\
             check_updates = {}\nauto_update = {}\n",
            e.commit_limit,
            e.reflog_limit,
            e.main_branches.join(", "),
            bool_text(self.confirm_rewriting_pushed),
            bool_text(self.watch_files),
            self.panel_sizes.map(|s| format!("{s:.3}")).join(", "),
            key_of(&theme::THEMES, self.theme),
            bool_text(self.check_updates),
            bool_text(self.auto_update),
        )
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }
}

/// The Settings window's state: an edited copy, and the main-branches field as typed.
pub struct Editor {
    pub draft: Prefs,
    main_branches: String,
}

impl Editor {
    pub fn new(prefs: &Prefs) -> Self {
        Self { draft: prefs.clone(), main_branches: prefs.engine.main_branches.join(", ") }
    }
}

pub enum Outcome {
    Open,
    Close,
    /// The draft changed; apply and save it.
    Changed,
    CheckForUpdates,
}

fn combo<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let before = *value;
    let current = options.iter().find(|o| o.0 == *value).map_or("", |o| o.2);
    egui::ComboBox::from_id_salt(id).selected_text(current).width(220.0).show_ui(ui, |ui| {
        for (v, _, label) in options {
            ui.selectable_value(value, *v, *label);
        }
    });
    *value != before
}

/// The Settings window. `updates`: what the last update check found, and whether one is running.
pub fn show(ctx: &egui::Context, editor: &mut Editor, path: Option<&Path>, updates: (&str, bool)) -> Outcome {
    let mut open = true;
    let mut changed = false;
    let mut check = false;
    egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
        let d = &mut editor.draft;
        egui::Grid::new("settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Theme");
            changed |= combo(ui, "theme", &mut d.theme, &theme::THEMES);
            ui.end_row();

            ui.label("Commits shown");
            changed |= ui.add(egui::DragValue::new(&mut d.engine.commit_limit).range(10..=100_000).speed(10)).changed();
            ui.end_row();

            ui.label("Reflog entries shown");
            changed |= ui.add(egui::DragValue::new(&mut d.engine.reflog_limit).range(10..=100_000).speed(10)).changed();
            ui.end_row();

            ui.label("Main branches").on_hover_text("Commits already on one of these (or its upstream) are shown green, pushed ones yellow, unpushed ones red.");
            if ui.add(egui::TextEdit::singleline(&mut editor.main_branches).hint_text("main, master").desired_width(220.0)).changed() {
                d.engine.main_branches = editor.main_branches.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
                changed = true;
            }
            ui.end_row();

            ui.label("Sort local branches by");
            changed |= combo(ui, "branch_sort", &mut d.engine.branch_sort, &SORTS);
            ui.end_row();

            ui.label("Move stacked branches").on_hover_text(
                "When history is rewritten, also move other local branches that point at the rewritten commits (git rebase --update-refs, which needs git 2.38 or newer). Branches checked out in another worktree are left alone.",
            );
            changed |= combo(ui, "update_refs", &mut d.engine.update_refs, &UPDATE_REFS);
            ui.end_row();
        });
        ui.add_space(8.0);
        changed |= ui.checkbox(&mut d.confirm_rewriting_pushed, "Ask before rewriting commits that are already pushed").changed();
        changed |= ui.checkbox(&mut d.watch_files, "Refresh when files change").on_hover_text("Otherwise Oxbow refreshes when its window gets focus, or with Refresh.").changed();
        ui.add_space(8.0);
        ui.strong("Updates");
        changed |= ui.checkbox(&mut d.check_updates, "Check for updates when Oxbow starts").on_hover_text("Asks GitHub for Oxbow's newest release.").changed();
        changed |= ui
            .checkbox(&mut d.auto_update, "Install bug-fix updates automatically")
            .on_hover_text("Versions that only fix bugs (like 0.1.2 after 0.1.1) are downloaded on their own and installed when you close Oxbow. Versions with new features are offered first.")
            .changed();
        ui.horizontal(|ui| {
            let (status, checking) = updates;
            check = ui.add_enabled(!checking, egui::Button::new("Check now")).clicked();
            ui.label(status);
        });
        if let Some(path) = path {
            ui.add_space(8.0);
            ui.weak(format!("Saved in {}", path.display()));
        }
    });
    if !open {
        Outcome::Close
    } else if check {
        Outcome::CheckForUpdates
    } else if changed {
        Outcome::Changed
    } else {
        Outcome::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_junk() {
        let mut p = Prefs::default();
        p.engine.commit_limit = 500;
        p.engine.main_branches = vec!["trunk".into(), "develop".into()];
        p.engine.branch_sort = BranchSort::Name;
        p.engine.update_refs = UpdateRefs::Always;
        p.confirm_rewriting_pushed = false;
        p.theme = theme::Choice::GruvboxDark;
        p.auto_update = false;
        assert_eq!(Prefs::parse(&p.to_text()), p);

        let odd = Prefs::parse("commit_limit = lots\nnonsense\nupdate_refs = sometimes\nreflog_limit = 3\n");
        assert_eq!(odd.engine.commit_limit, 300);
        assert_eq!(odd.engine.update_refs, UpdateRefs::GitConfig);
        assert_eq!(odd.engine.reflog_limit, 10);
    }
}
