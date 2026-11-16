//! The Mods tab: the mods this save asks for, and which of them the game has active. Read
//! only: the game's own mod manager is the place to change them.

use super::{theme, widgets};
use crate::session::Session;
use eframe::egui::{self, Align, Layout, Ui};

pub fn show(ui: &mut Ui, session: &Session) {
    ui.label(egui::RichText::new("Mods").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new(
            "The mods this save asks for. The game warns about any it can't find when the save \
             is loaded.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(10.0);

    let mods = session.mods();
    let active = session.active_mods();
    let added: Vec<usize> = (0..mods.len()).filter(|&m| !mods[m].base).collect();

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        let note = match active {
            Some(active) => format!(
                "Compared with the mods active in your game, from {}.",
                active.path.display()
            ),
            None => "This save isn't in the game's own saves folder, so there is no list of the \
                     mods your game has active to compare with."
                .to_owned(),
        };
        ui.label(
            egui::RichText::new(note)
                .font(theme::body(12.5))
                .color(theme::MUTED),
        );
    });
    ui.add_space(8.0);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Added mods ({})", added.len()));
        ui.add_space(2.0);
        if added.is_empty() {
            ui.label(egui::RichText::new("This save uses no mods.").color(theme::MUTED));
        }
        for &m in &added {
            let module = &mods[m];
            let hint = format!("{} · version {}", module.folder, module.version_text());
            widgets::row(ui, &module.name, Some(&hint), |ui| {
                match active.map(|a| a.contains(module)) {
                    Some(true) => {
                        widgets::badge(ui, "ACTIVE", theme::FINE, theme::FINE_BG);
                    }
                    Some(false) => {
                        widgets::badge(ui, "NOT ACTIVE", theme::CORAL, theme::CORAL_BG);
                    }
                    None => {}
                }
            });
        }
    });
    ui.add_space(8.0);

    let base: Vec<&str> = mods
        .iter()
        .filter(|m| m.base)
        .map(|m| m.name.as_str())
        .collect();
    if !base.is_empty() {
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::section_title(ui, &format!("Part of the game ({})", base.len()));
            ui.with_layout(
                Layout::left_to_right(Align::Min).with_main_wrap(true),
                |ui| {
                    ui.label(egui::RichText::new(base.join(" · ")).color(theme::MUTED));
                },
            );
        });
    }
}
