//! The Mods tab: the mods this save asks for, which of them the game has active, and taking
//! one off the list.

use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::session::{Field, Session};
use eframe::egui::{self, Align, Layout, Ui};

pub fn show(ui: &mut Ui, session: &Session, actions: &mut Vec<Action>) {
    ui.label(egui::RichText::new("Mods").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new(
            "The mods this save asks for. The game warns about any it can't find when the save \
             is loaded; take a mod off the list once it is no longer installed.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(10.0);

    let mods = session.mods();
    let active = session.active_mods();
    let added: Vec<usize> = (0..mods.len()).filter(|&m| !mods[m].base).collect();
    let removed = |m: usize| session.value(Field::Mod(m)) == Some(0);
    let inactive: Vec<usize> = match active {
        Some(active) => added
            .iter()
            .copied()
            .filter(|&m| !active.contains(&mods[m]) && !removed(m))
            .collect(),
        None => Vec::new(),
    };

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        match active {
            Some(active) => {
                ui.label(
                    egui::RichText::new(format!(
                        "Compared with the mods active in your game, from {}.",
                        active.path.display()
                    ))
                    .font(theme::body(12.5))
                    .color(theme::MUTED),
                );
                if !inactive.is_empty() {
                    ui.add_space(6.0);
                    let label = format!("Remove the {} not active in your game", inactive.len());
                    if widgets::button(ui, &label, Kind::Secondary).clicked() {
                        actions.extend(inactive.iter().map(|&m| Action::Set(Field::Mod(m), 0)));
                    }
                }
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "This save isn't in the game's own saves folder, so there is no list of \
                         the mods your game has active to compare with.",
                    )
                    .font(theme::body(12.5))
                    .color(theme::MUTED),
                );
            }
        }
    });
    ui.add_space(8.0);

    if added.iter().any(|&m| removed(m)) {
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(
                    "⚠ Taking a mod off the list does not take out what it added. Items, spells \
                     or looks from a removed mod are still in the save, and the game may drop \
                     them or misbehave over them. Keep a mod on the list while it is installed.",
                )
                .color(theme::ACCENT),
            );
        });
        ui.add_space(8.0);
    }

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Added mods ({})", added.len()));
        ui.add_space(2.0);
        if added.is_empty() {
            ui.label(egui::RichText::new("This save uses no mods.").color(theme::MUTED));
        }
        for &m in &added {
            let module = &mods[m];
            let gone = removed(m);
            let mut hint = format!("{} · version {}", module.folder, module.version_text());
            if gone {
                hint.push_str(" · will be taken off the list");
            }
            widgets::row(ui, &module.name, Some(&hint), |ui| {
                if gone {
                    if widgets::icon_button(ui, "⟲", "Keep it on the list").clicked() {
                        actions.push(Action::Revert(Field::Mod(m)));
                    }
                    widgets::badge(ui, "REMOVED", theme::ON_ACCENT, theme::CHANGED);
                } else {
                    if widgets::button(ui, "Remove", Kind::Quiet).clicked() {
                        actions.push(Action::Set(Field::Mod(m), 0));
                    }
                    match active.map(|a| a.contains(module)) {
                        Some(true) => {
                            widgets::badge(ui, "ACTIVE", theme::FINE, theme::FINE_BG);
                        }
                        Some(false) => {
                            widgets::badge(ui, "NOT ACTIVE", theme::CORAL, theme::CORAL_BG);
                        }
                        None => {}
                    }
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
