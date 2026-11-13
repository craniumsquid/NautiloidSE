//! The Save info tab: what the save is and what is in it. Nothing here can be changed.

use super::widgets::{self, grouped};
use super::{browser, theme};
use crate::saves;
use crate::session::Session;
use eframe::egui::{self, Ui};

/// One line of the facts grid: what it is, then its value.
fn fact(ui: &mut Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).color(theme::MUTED));
    ui.add(egui::Label::new(egui::RichText::new(value).color(theme::TEXT_SOFT)).wrap());
    ui.end_row();
}

pub fn show(ui: &mut Ui, session: &Session, now: u64) {
    let summary = session.summary();
    ui.label(egui::RichText::new("Save info").font(theme::display(30.0)));
    ui.label(egui::RichText::new("What this save is and what it holds.").color(theme::MUTED));
    ui.add_space(10.0);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, "The save");
        egui::Grid::new("facts")
            .num_columns(2)
            .min_col_width(120.0)
            .min_row_height(22.0)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                let saved = saves::ago(now.saturating_sub(summary.saved_at));
                let rules = if summary.honour_mode {
                    "Honour Mode"
                } else {
                    "Not Honour Mode"
                };
                fact(ui, "Name", &summary.name);
                fact(ui, "Campaign", &summary.leader);
                fact(ui, "Kind", summary.kind.label());
                fact(ui, "Saved", &saved);
                fact(ui, "Where", browser::level_name(&summary.level));
                fact(ui, "Game version", &summary.game_version);
                fact(ui, "Rules", rules);
                fact(ui, "Folder", &summary.folder.display().to_string());
            });
    });
    ui.add_space(8.0);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Mods ({})", summary.mods.len()));
        if summary.mods.is_empty() {
            ui.label(egui::RichText::new("None.").color(theme::MUTED));
        }
        for name in &summary.mods {
            ui.label(egui::RichText::new(name).color(theme::TEXT_SOFT));
        }
    });
    ui.add_space(8.0);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, "Files in the save");
        egui::Grid::new("files")
            .num_columns(3)
            .spacing([28.0, 6.0])
            .show(ui, |ui| {
                for heading in ["File", "Size", "Stored as"] {
                    ui.label(
                        egui::RichText::new(heading)
                            .font(theme::body(12.5))
                            .color(theme::MUTED),
                    );
                }
                ui.end_row();
                for entry in session.save().package.entries() {
                    ui.label(egui::RichText::new(entry.name()).font(theme::mono(13.5)));
                    ui.label(format!("{} bytes", grouped(entry.len() as u64)));
                    ui.label(
                        egui::RichText::new(format!(
                            "{} bytes, {}",
                            grouped(entry.stored_len() as u64),
                            entry.method().label()
                        ))
                        .color(theme::MUTED),
                    );
                    ui.end_row();
                }
            });
    });
}
