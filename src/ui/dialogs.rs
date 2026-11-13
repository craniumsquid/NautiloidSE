//! The dialogs: exporting, leaving with changes unexported, and saying what went wrong.

use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::session::Session;
use eframe::egui::{
    self, Align, Align2, Color32, Context, Frame, Id, Key, Layout, Margin, Order, Rounding, Sense,
    Ui,
};
use std::path::PathBuf;

/// What to do once the person has dealt with their unexported changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum After {
    /// Go back to the save browser.
    Back,
    /// Open the save in this folder.
    Open(PathBuf),
    /// Close the window.
    Close,
}

pub enum Dialog {
    /// Asking for the name to export under.
    Export {
        name: String,
        /// Why the last try failed.
        error: Option<String>,
        /// What the person was doing when they chose to export first.
        then: Option<After>,
    },
    /// About to leave a save with changes that have not been exported.
    Unexported { then: After },
    /// Something could not be done.
    Problem { title: String, detail: String },
}

/// Why `name` can't be a save's name, if it can't.
pub fn name_problem(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        Some("Give the save a name.")
    } else if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        Some("A name can't contain / \\ : * ? \" < > or |.")
    } else if name.ends_with('.') {
        Some("A name can't end with a full stop.")
    } else if name.chars().count() > 80 {
        Some("That name is too long.")
    } else {
        None
    }
}

pub fn show(
    ctx: &Context,
    dialog: &mut Dialog,
    session: Option<&Session>,
    actions: &mut Vec<Action>,
) {
    // Dim everything behind and swallow clicks meant for it.
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new("dialog-backdrop"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.allocate_response(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, Rounding::ZERO, Color32::from_black_alpha(170));
        });
    egui::Area::new(Id::new("dialog"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, [0.0, -20.0])
        .show(ctx, |ui| {
            Frame::none()
                .fill(theme::DIALOG)
                .stroke(theme::stroke(1.0, theme::LINE_STRONG))
                .rounding(Rounding::same(14.0))
                .inner_margin(Margin::same(24.0))
                .show(ui, |ui| {
                    ui.set_width(460.0);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    match dialog {
                        Dialog::Export { name, error, .. } => {
                            export(ui, name, error, session, actions)
                        }
                        Dialog::Unexported { then } => unexported(ui, then, session, actions),
                        Dialog::Problem { title, detail } => problem(ui, title, detail, actions),
                    }
                });
        });
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(Action::CloseDialog);
    }
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::display(22.0)));
}

fn buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

fn export(
    ui: &mut Ui,
    name: &mut String,
    error: &Option<String>,
    session: Option<&Session>,
    actions: &mut Vec<Action>,
) {
    title(ui, "Export as a new save");
    let count = session.map_or(0, |s| s.changes().len());
    ui.label(
        egui::RichText::new(match count {
            0 => "No changes: this writes a plain copy. The save you opened is not changed."
                .to_owned(),
            1 => "Writes a new save with your 1 change. The save you opened is not changed."
                .to_owned(),
            n => format!(
                "Writes a new save with your {n} changes. The save you opened is not changed."
            ),
        })
        .color(theme::TEXT_SOFT),
    );
    ui.label(
        egui::RichText::new("Name in the game's Load menu")
            .font(theme::body(12.5))
            .color(theme::MUTED),
    );
    let field = ui.add(
        egui::TextEdit::singleline(name)
            .desired_width(f32::INFINITY)
            .font(theme::medium(16.0))
            .margin(Margin::symmetric(10.0, 8.0)),
    );
    // Put the cursor in the name when the dialog opens.
    let focused = Id::new("export-focused");
    if !ui.data(|d| d.get_temp::<bool>(focused).unwrap_or(false)) {
        field.request_focus();
        ui.data_mut(|d| d.insert_temp(focused, true));
    }
    if let Some(session) = session {
        ui.label(
            egui::RichText::new(format!(
                "Goes in {}",
                session.default_export_folder().display()
            ))
            .font(theme::body(12.5))
            .color(theme::MUTED),
        );
    }
    if session.is_some_and(|s| s.summary().honour_mode) {
        ui.label(
            egui::RichText::new(
                "⚠ This is an Honour Mode save. The game keeps a single save for an Honour \
                 Mode playthrough, and a second one beside it hasn't been tried in the game.",
            )
            .color(theme::ACCENT),
        );
    }
    let problem = name_problem(name);
    if let Some(text) = problem.map(str::to_owned).or_else(|| error.clone()) {
        ui.label(egui::RichText::new(text).color(theme::CORAL));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    buttons(ui, |ui| {
        ui.add_enabled_ui(problem.is_none(), |ui| {
            if widgets::button(ui, "Export", Kind::Primary).clicked()
                || (enter && problem.is_none())
            {
                actions.push(Action::Export);
                ui.data_mut(|d| d.remove::<bool>(focused));
            }
        });
        if widgets::button(ui, "Cancel", Kind::Quiet).clicked() {
            actions.push(Action::CloseDialog);
            ui.data_mut(|d| d.remove::<bool>(focused));
        }
    });
}

fn unexported(ui: &mut Ui, then: &After, session: Option<&Session>, actions: &mut Vec<Action>) {
    let count = session.map_or(0, |s| s.changes().len());
    title(
        ui,
        &format!(
            "{count} change{} not exported",
            if count == 1 { "" } else { "s" }
        ),
    );
    ui.label(
        egui::RichText::new(match then {
            After::Close => "If you close now they are lost. The save you opened is as it was either way.",
            _ => "If you leave this save now they are lost. The save you opened is as it was either way.",
        })
        .color(theme::TEXT_SOFT),
    );
    buttons(ui, |ui| {
        if widgets::button(ui, "Export…", Kind::Primary).clicked() {
            actions.push(Action::AskExport);
        }
        if widgets::button(ui, "Discard changes", Kind::Secondary).clicked() {
            actions.push(Action::Discard);
        }
        if widgets::button(ui, "Cancel", Kind::Quiet).clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}

fn problem(ui: &mut Ui, heading: &str, detail: &str, actions: &mut Vec<Action>) {
    title(ui, heading);
    ui.add(egui::Label::new(egui::RichText::new(detail).color(theme::TEXT_SOFT)).wrap());
    buttons(ui, |ui| {
        if widgets::button(ui, "Close", Kind::Secondary).clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::name_problem;

    #[test]
    fn checks_save_names() {
        assert_eq!(name_problem("Shattered Sanctum - 8h 02m - Edited"), None);
        assert!(name_problem("   ").is_some());
        assert!(name_problem("a/b").is_some());
        assert!(name_problem("what?").is_some());
        assert!(name_problem("ends.").is_some());
        assert!(name_problem(&"x".repeat(81)).is_some());
    }
}
