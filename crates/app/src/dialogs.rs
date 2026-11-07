//! Modal dialogs: confirming a destructive action, or asking for a name or message.

use eframe::egui;

use crate::worker::Action;

type Make = Box<dyn FnOnce(String, bool) -> Action + Send>;

pub enum Dialog {
    Confirm { title: String, body: String, button: String, action: Option<Action> },
    Prompt { title: String, label: String, text: String, multiline: bool, check: Option<(String, bool)>, make: Option<Make>, focused: bool },
}

pub fn confirm(title: impl Into<String>, body: impl Into<String>, button: impl Into<String>, action: Action) -> Dialog {
    Dialog::Confirm { title: title.into(), body: body.into(), button: button.into(), action: Some(action) }
}

/// Asks for a line of text; `make` turns it into the action to run.
pub fn prompt(title: impl Into<String>, label: impl Into<String>, initial: impl Into<String>, make: impl FnOnce(String, bool) -> Action + Send + 'static) -> Dialog {
    Dialog::Prompt { title: title.into(), label: label.into(), text: initial.into(), multiline: false, check: None, make: Some(Box::new(make)), focused: false }
}

impl Dialog {
    /// Adds a checkbox whose value is passed to `make`.
    pub fn with_check(mut self, label: impl Into<String>, value: bool) -> Self {
        if let Dialog::Prompt { check, .. } = &mut self {
            *check = Some((label.into(), value));
        }
        self
    }
}

pub enum Outcome {
    Open,
    Cancel,
    Run(Action),
}

pub fn show(ctx: &egui::Context, dialog: &mut Dialog) -> Outcome {
    let mut outcome = Outcome::Open;
    let modal = egui::Modal::new(egui::Id::unique("oxbow-dialog")).show(ctx, |ui| {
        ui.set_width(420.0);
        match dialog {
            Dialog::Confirm { title, body, button, action } => {
                ui.heading(title.as_str());
                ui.add_space(6.0);
                ui.label(body.as_str());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if (ui.button(button.as_str()).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        && let Some(a) = action.take() {
                            outcome = Outcome::Run(a);
                        }
                    if ui.button("Cancel").clicked() {
                        outcome = Outcome::Cancel;
                    }
                });
            }
            Dialog::Prompt { title, label, text, multiline, check, make, focused } => {
                ui.heading(title.as_str());
                ui.add_space(6.0);
                ui.label(label.as_str());
                let edit = if *multiline {
                    egui::TextEdit::multiline(text).desired_rows(4).desired_width(f32::INFINITY)
                } else {
                    egui::TextEdit::singleline(text).desired_width(f32::INFINITY)
                };
                let response = ui.add(edit);
                if !*focused {
                    response.request_focus();
                    *focused = true;
                }
                if let Some((label, value)) = check {
                    ui.checkbox(value, label.as_str());
                }
                ui.add_space(10.0);
                let submit_key = if *multiline {
                    ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command)
                } else {
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                };
                ui.horizontal(|ui| {
                    let ok = ui.add_enabled(!text.trim().is_empty(), egui::Button::new("OK")).clicked();
                    if (ok || submit_key) && !text.trim().is_empty()
                        && let Some(make) = make.take() {
                            outcome = Outcome::Run(make(text.trim().to_owned(), check.as_ref().is_some_and(|c| c.1)));
                        }
                    if ui.button("Cancel").clicked() {
                        outcome = Outcome::Cancel;
                    }
                });
            }
        }
    });
    if modal.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Cancel;
    }
    outcome
}
