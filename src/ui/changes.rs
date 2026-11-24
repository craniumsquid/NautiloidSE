//! The Changes panel: every difference from the save as opened, each with its own way back,
//! and the way out.

use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::session::Session;
use eframe::egui::{self, Align, Frame, Layout, Margin, Rounding, Ui};

fn number(text: &str) -> String {
    text.parse::<i64>()
        .map_or_else(|_| text.to_owned(), widgets::signed)
}

pub fn show(ui: &mut Ui, session: &Session, actions: &mut Vec<Action>) {
    let changes = session.changes();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Changes").font(theme::display(22.0)));
        if !changes.is_empty() {
            widgets::badge(
                ui,
                &changes.len().to_string(),
                theme::ON_ACCENT,
                theme::CHANGED,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, "Revert all", Kind::Quiet).clicked() {
                    actions.push(Action::RevertAll);
                }
            });
        }
    });
    ui.label(
        egui::RichText::new(if changes.is_empty() {
            "Nothing changed yet. What you change is listed here, and nothing is written until \
             you export."
        } else {
            "Not written anywhere yet. Export to keep them."
        })
        .font(theme::body(13.0))
        .color(theme::MUTED),
    );
    ui.add_space(6.0);

    // The export button stays at the bottom; the list scrolls in what is left.
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.label(
            egui::RichText::new("Writes a new save. The one you opened is never changed.")
                .font(theme::body(12.5))
                .color(theme::MUTED),
        );
        if widgets::wide_button(ui, "Export as a new save…", Kind::Primary).clicked() {
            actions.push(Action::AskExport);
        }
        ui.add_space(6.0);
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("changes")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    repairs(ui, session);
                    let mut owner = "";
                    for change in &changes {
                        if change.who != owner {
                            owner = &change.who;
                            ui.add_space(4.0);
                            widgets::section_title(ui, owner);
                        }
                        Frame::none()
                            .fill(theme::PANEL)
                            .stroke(theme::stroke(1.0, theme::LINE))
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::symmetric(12.0, 9.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.label(
                                            egui::RichText::new(&change.what)
                                                .font(theme::semibold(14.5)),
                                        );
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            ui.label(
                                                egui::RichText::new(number(&change.before))
                                                    .color(theme::MUTED),
                                            );
                                            ui.label(egui::RichText::new("➡").color(theme::MUTED));
                                            ui.label(
                                                egui::RichText::new(number(&change.after))
                                                    .font(theme::semibold(15.0))
                                                    .color(theme::CHANGED),
                                            );
                                        });
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let tip = format!("Put back {}", number(&change.before));
                                        if widgets::icon_button(ui, "⟲", &tip).clicked() {
                                            actions.push(Action::Revert(change.field));
                                        }
                                    });
                                });
                            });
                    }
                });
        });
    });
}

/// What every export repairs, when the save needs it: items sharing statuses with another,
/// as copies made by earlier versions do. It can't be undone, as there's nothing to want back.
fn repairs(ui: &mut Ui, session: &Session) {
    let repairs = session.repairs();
    if repairs.is_empty() {
        return;
    }
    ui.add_space(4.0);
    widgets::section_title(ui, "Repaired on export");
    let mut names: Vec<String> = repairs.iter().map(|r| session.repair_name(r)).collect();
    names.sort();
    let shown = 6;
    let mut list = names
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > shown {
        list.push_str(&format!(" and {} more", names.len() - shown));
    }
    let count = match repairs.len() {
        1 => "One item has".to_owned(),
        n => format!("{n} items have"),
    };
    Frame::none()
        .fill(theme::PANEL)
        .stroke(theme::stroke(1.0, theme::LINE))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(egui::RichText::new(list).font(theme::semibold(14.5)));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{count} hidden status records that earlier versions of this editor \
                         left broken. Loading the save, or selling or combining one of them, \
                         could crash the game. The export repairs them; an item may lose a \
                         glow or sparkle, and nothing else changes."
                    ))
                    .font(theme::body(12.5))
                    .color(theme::MUTED),
                )
                .wrap(),
            );
        });
}
