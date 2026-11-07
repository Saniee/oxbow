//! Themes, colours and small shared pieces of row rendering.
//!
//! A theme is the window's own colours (backgrounds, text, buttons, selection) and a palette for what Oxbow colours
//! itself: commit marks, status letters, refs, diff lines and graph lanes. The chosen theme lives in egui's context,
//! so anything drawing reads it from its `Ui` with [`palette`].

use eframe::egui::{self, Color32, RichText, TextFormat, text::LayoutJob};
use oxbow_engine::CommitMark;
use oxbow_engine::queries::Commit;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// egui's dark or light look, whichever the system uses.
    #[default]
    System,
    Dark,
    Light,
    OneDarkPro,
    GruvboxDark,
    GruvboxLight,
}

/// Each theme: its key in settings.txt and its name in Settings.
pub const THEMES: [(Choice, &str, &str); 6] = [
    (Choice::System, "system", "As the system is (dark or light)"),
    (Choice::Dark, "dark", "Dark"),
    (Choice::Light, "light", "Light"),
    (Choice::OneDarkPro, "one-dark-pro", "One Dark Pro"),
    (Choice::GruvboxDark, "gruvbox-dark", "Gruvbox Dark"),
    (Choice::GruvboxLight, "gruvbox-light", "Gruvbox Light"),
];

/// What Oxbow colours itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub merged: Color32,
    pub pushed: Color32,
    pub unpushed: Color32,
    pub staged: Color32,
    pub unstaged: Color32,
    pub untracked: Color32,
    /// Conflicts, errors and anything destructive.
    pub conflict: Color32,
    pub local_ref: Color32,
    pub remote_ref: Color32,
    pub tag_ref: Color32,
    /// Hunk headers, picked lines, and other things Oxbow points out.
    pub hunk: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    /// Lane colours of the commit graph.
    pub graph: [Color32; 8],
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn tint(hex: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha)
}

/// Oxbow's own colours, on egui's dark or light look.
const DEFAULT: Palette = Palette {
    merged: rgb(0x4cb864),
    pushed: rgb(0xd8b43c),
    unpushed: rgb(0xe05c5c),
    staged: rgb(0x4cb864),
    unstaged: rgb(0xe08c3c),
    untracked: rgb(0x6ca8e8),
    conflict: rgb(0xe05c5c),
    local_ref: rgb(0x4cb864),
    remote_ref: rgb(0x6ca8e8),
    tag_ref: rgb(0xd8b43c),
    hunk: rgb(0x9c7cd8),
    added_bg: tint(0x2ea043, 48),
    removed_bg: tint(0xf85149, 48),
    graph: [rgb(0x4f9de8), rgb(0xd78c3f), rgb(0x9b6fe0), rgb(0x3fb89e), rgb(0xe06c9f), rgb(0xb8b03f), rgb(0x6f8fb8), rgb(0xc06a5a)],
};

const DEFAULT_LIGHT: Palette = Palette { added_bg: tint(0x2ea043, 40), removed_bg: tint(0xf85149, 36), ..DEFAULT };

/// One Dark's syntax colours (Atom's One Dark, as VS Code's One Dark Pro has them).
const ONE_DARK: Palette = Palette {
    merged: rgb(0x98c379),
    pushed: rgb(0xe5c07b),
    unpushed: rgb(0xe06c75),
    staged: rgb(0x98c379),
    unstaged: rgb(0xd19a66),
    untracked: rgb(0x61afef),
    conflict: rgb(0xe06c75),
    local_ref: rgb(0x98c379),
    remote_ref: rgb(0x61afef),
    tag_ref: rgb(0xe5c07b),
    hunk: rgb(0xc678dd),
    added_bg: tint(0x98c379, 40),
    removed_bg: tint(0xe06c75, 40),
    graph: [rgb(0x61afef), rgb(0xd19a66), rgb(0xc678dd), rgb(0x56b6c2), rgb(0xe06c75), rgb(0xe5c07b), rgb(0x98c379), rgb(0xbe5046)],
};

/// Gruvbox's bright colours, for its dark background.
const GRUVBOX_DARK: Palette = Palette {
    merged: rgb(0xb8bb26),
    pushed: rgb(0xfabd2f),
    unpushed: rgb(0xfb4934),
    staged: rgb(0xb8bb26),
    unstaged: rgb(0xfe8019),
    untracked: rgb(0x83a598),
    conflict: rgb(0xfb4934),
    local_ref: rgb(0xb8bb26),
    remote_ref: rgb(0x83a598),
    tag_ref: rgb(0xfabd2f),
    hunk: rgb(0xd3869b),
    added_bg: tint(0xb8bb26, 40),
    removed_bg: tint(0xfb4934, 40),
    graph: [rgb(0x83a598), rgb(0xfe8019), rgb(0xd3869b), rgb(0x8ec07c), rgb(0xfb4934), rgb(0xfabd2f), rgb(0xb8bb26), rgb(0xd65d0e)],
};

/// Gruvbox's faded colours, for its light background.
const GRUVBOX_LIGHT: Palette = Palette {
    merged: rgb(0x79740e),
    pushed: rgb(0xb57614),
    unpushed: rgb(0x9d0006),
    staged: rgb(0x79740e),
    unstaged: rgb(0xaf3a03),
    untracked: rgb(0x076678),
    conflict: rgb(0x9d0006),
    local_ref: rgb(0x79740e),
    remote_ref: rgb(0x076678),
    tag_ref: rgb(0xb57614),
    hunk: rgb(0x8f3f71),
    added_bg: tint(0x98971a, 48),
    removed_bg: tint(0xcc241d, 36),
    graph: [rgb(0x076678), rgb(0xaf3a03), rgb(0x8f3f71), rgb(0x427b58), rgb(0x9d0006), rgb(0xb57614), rgb(0x79740e), rgb(0xd65d0e)],
};

/// A theme's window colours.
struct Surface {
    dark: bool,
    /// Panels.
    bg: Color32,
    /// Dialogs, menus and tooltips.
    window: Color32,
    /// Text fields, the command log, scroll tracks.
    inset: Color32,
    /// Every other row of a striped list.
    faint: Color32,
    widget: Color32,
    hover: Color32,
    active: Color32,
    border: Color32,
    selection: Color32,
    text: Color32,
    weak: Color32,
    strong: Color32,
    accent: Color32,
    warn: Color32,
    error: Color32,
}

const ONE_DARK_SURFACE: Surface = Surface {
    dark: true,
    bg: rgb(0x282c34),
    window: rgb(0x21252b),
    inset: rgb(0x1e2227),
    faint: rgb(0x2c313a),
    widget: rgb(0x333842),
    hover: rgb(0x3a404b),
    active: rgb(0x454c59),
    border: rgb(0x3e4452),
    selection: rgb(0x3e4b63),
    text: rgb(0xabb2bf),
    weak: rgb(0x7f848e),
    strong: rgb(0xd7dae0),
    accent: rgb(0x61afef),
    warn: rgb(0xe5c07b),
    error: rgb(0xe06c75),
};

const GRUVBOX_DARK_SURFACE: Surface = Surface {
    dark: true,
    bg: rgb(0x282828),
    window: rgb(0x32302f),
    inset: rgb(0x1d2021),
    faint: rgb(0x32302f),
    widget: rgb(0x3c3836),
    hover: rgb(0x45403d),
    active: rgb(0x665c54),
    border: rgb(0x504945),
    selection: rgb(0x504945),
    text: rgb(0xebdbb2),
    weak: rgb(0xa89984),
    strong: rgb(0xfbf1c7),
    accent: rgb(0x83a598),
    warn: rgb(0xfabd2f),
    error: rgb(0xfb4934),
};

const GRUVBOX_LIGHT_SURFACE: Surface = Surface {
    dark: false,
    bg: rgb(0xfbf1c7),
    window: rgb(0xf2e5bc),
    inset: rgb(0xf9f5d7),
    faint: rgb(0xf2e5bc),
    widget: rgb(0xebdbb2),
    hover: rgb(0xe0cfa8),
    active: rgb(0xd5c4a1),
    border: rgb(0xd5c4a1),
    selection: rgb(0xd5c4a1),
    text: rgb(0x3c3836),
    weak: rgb(0x7c6f64),
    strong: rgb(0x282828),
    accent: rgb(0x076678),
    warn: rgb(0xb57614),
    error: rgb(0x9d0006),
};

impl Surface {
    /// egui's look of the same brightness, recoloured.
    fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = self.bg;
        v.window_fill = self.window;
        v.window_stroke.color = self.border;
        v.extreme_bg_color = self.inset;
        v.code_bg_color = self.inset;
        v.text_edit_bg_color = Some(self.inset);
        v.faint_bg_color = self.faint;
        v.weak_text_color = Some(self.weak);
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.warn;
        v.error_fg_color = self.error;
        v.selection.bg_fill = self.selection;
        v.selection.stroke.color = self.strong;
        v.text_cursor.stroke.color = self.accent;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.window;
        w.noninteractive.weak_bg_fill = self.window;
        w.noninteractive.bg_stroke.color = self.border;
        w.noninteractive.fg_stroke.color = self.text;
        for (state, fill, fg) in [(&mut w.inactive, self.widget, self.text), (&mut w.hovered, self.hover, self.strong), (&mut w.active, self.active, self.strong)] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.fg_stroke.color = fg;
        }
        w.hovered.bg_stroke.color = self.border;
        w.active.bg_stroke.color = self.accent;
        w.open.bg_fill = self.window;
        w.open.weak_bg_fill = self.hover;
        w.open.bg_stroke.color = self.border;
        w.open.fg_stroke.color = self.text;
        v
    }
}

impl Choice {
    fn surface(self) -> Option<&'static Surface> {
        match self {
            Choice::System | Choice::Dark | Choice::Light => None,
            Choice::OneDarkPro => Some(&ONE_DARK_SURFACE),
            Choice::GruvboxDark => Some(&GRUVBOX_DARK_SURFACE),
            Choice::GruvboxLight => Some(&GRUVBOX_LIGHT_SURFACE),
        }
    }

    /// The palette, on a dark or light look (which only matters for the themes without their own).
    fn palette(self, dark: bool) -> Palette {
        match self {
            Choice::System | Choice::Dark | Choice::Light => {
                if dark {
                    DEFAULT
                } else {
                    DEFAULT_LIGHT
                }
            }
            Choice::OneDarkPro => ONE_DARK,
            Choice::GruvboxDark => GRUVBOX_DARK,
            Choice::GruvboxLight => GRUVBOX_LIGHT,
        }
    }
}

fn choice_id() -> egui::Id {
    egui::Id::NULL.with("oxbow-theme")
}

/// Switches the whole window to `choice`.
pub fn apply(ctx: &egui::Context, choice: Choice) {
    ctx.data_mut(|d| d.insert_temp(choice_id(), choice));
    match choice.surface() {
        Some(surface) => {
            let theme = if surface.dark { egui::Theme::Dark } else { egui::Theme::Light };
            ctx.set_visuals_of(theme, surface.visuals());
            ctx.set_theme(theme);
        }
        None => {
            // Back to egui's own look, which another theme may have recoloured.
            ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::dark());
            ctx.set_visuals_of(egui::Theme::Light, egui::Visuals::light());
            ctx.set_theme(match choice {
                Choice::Dark => egui::ThemePreference::Dark,
                Choice::Light => egui::ThemePreference::Light,
                _ => egui::ThemePreference::System,
            });
        }
    }
}

/// The colours to draw with.
pub fn palette(ui: &egui::Ui) -> Palette {
    let choice = ui.ctx().data(|d| d.get_temp::<Choice>(choice_id())).unwrap_or_default();
    choice.palette(ui.visuals().dark_mode)
}

impl Palette {
    pub fn mark(&self, mark: CommitMark) -> Color32 {
        match mark {
            CommitMark::Merged => self.merged,
            CommitMark::Pushed => self.pushed,
            CommitMark::Unpushed => self.unpushed,
        }
    }

    /// A ref decoration from `%D`: "HEAD -> main", "origin/main", "tag: v1".
    pub fn of_ref(&self, r: &str) -> Color32 {
        if r.starts_with("tag: ") {
            self.tag_ref
        } else if r.contains('/') && !r.starts_with("HEAD -> ") {
            self.remote_ref
        } else {
            self.local_ref
        }
    }

    pub fn lane(&self, i: usize) -> Color32 {
        self.graph[i % self.graph.len()]
    }
}

pub fn ref_chip(ui: &mut egui::Ui, r: &str) {
    let color = palette(ui).of_ref(r);
    ui.label(RichText::new(r).small().color(color));
}

/// One commit as a single line: mark, short SHA, refs, subject, author and date.
pub fn commit_row(ui: &egui::Ui, c: &Commit, mark: Option<CommitMark>) -> LayoutJob {
    commit_row_after(ui, c, mark, 0.0)
}

/// A commit row whose text starts `graph` points in, leaving room for the graph; with a graph, the dot drawn there
/// shows the mark.
pub fn commit_row_after(ui: &egui::Ui, c: &Commit, mark: Option<CommitMark>, graph: f32) -> LayoutJob {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let colors = palette(ui);
    let mut job = LayoutJob::default();
    let fmt = |font: &egui::FontId, color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    match mark {
        Some(mark) if graph == 0.0 => job.append("⏺ ", 0.0, fmt(&body, colors.mark(mark))),
        _ => {}
    }
    job.append(&c.short_sha, graph, fmt(&mono, weak));
    for r in &c.refs {
        job.append(&format!("[{r}]"), 6.0, fmt(&body, colors.of_ref(r)));
    }
    job.append(&c.subject, 6.0, fmt(&body, text));
    job.append(&format!("{} · {}", c.author, c.rel_date), 10.0, fmt(&body, weak));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// git's two status letters, coloured: staged (X) and unstaged (Y).
pub fn status_letters(colors: &Palette, x: char, y: char, conflict: bool, untracked: bool, font: &egui::FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    if conflict {
        job.append(&format!("{x}{y}"), 0.0, fmt(colors.conflict));
    } else if untracked {
        job.append("??", 0.0, fmt(colors.untracked));
    } else {
        job.append(&x.to_string(), 0.0, fmt(colors.staged));
        job.append(&y.to_string(), 0.0, fmt(colors.unstaged));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_recolours_the_window_and_going_back_restores_egui_s_own() {
        let ctx = egui::Context::default();
        apply(&ctx, Choice::GruvboxLight);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(ctx.global_style().visuals.panel_fill, GRUVBOX_LIGHT_SURFACE.bg);
        apply(&ctx, Choice::OneDarkPro);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(ctx.global_style().visuals.panel_fill, ONE_DARK_SURFACE.bg);
        apply(&ctx, Choice::Dark);
        assert_eq!(ctx.global_style().visuals.panel_fill, egui::Visuals::dark().panel_fill);
    }
}
