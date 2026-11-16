//! The Approval tab: what each companion thinks of the player's character.

use super::party::field_stepper;
use super::{theme, widgets, Action};
use crate::session::{Field, Session};
use eframe::egui::{self, Ui};

pub fn show(ui: &mut Ui, session: &Session, actions: &mut Vec<Action>) {
    ui.label(egui::RichText::new("Approval").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new(format!(
            "What each companion thinks of your character, from {} to {}.",
            widgets::signed(crate::party::APPROVAL_MIN as i64),
            crate::party::APPROVAL_MAX
        ))
        .color(theme::MUTED),
    );
    ui.add_space(10.0);

    let characters = &session.party().characters;
    let rated: Vec<usize> = session
        .order()
        .iter()
        .copied()
        .filter(|&c| !characters[c].approval.is_empty())
        .collect();
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, "Companions");
        ui.add_space(2.0);
        if rated.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No companion in this save has an approval rating yet. One appears once a \
                     companion has joined.",
                )
                .color(theme::MUTED),
            );
        }
        for c in rated {
            let character = &characters[c];
            let place = if character.in_party() {
                "In the party"
            } else {
                "At camp"
            };
            let several = character.approval.len() > 1;
            for (which, rating) in character.approval.iter().enumerate() {
                // With more than one player, each has their own standing.
                let hint = match several {
                    true => format!("{place} · of {}", session.name_of(rating.of)),
                    false => place.to_owned(),
                };
                widgets::row(ui, &character.display_name(), Some(&hint), |ui| {
                    field_stepper(ui, session, Field::Approval(c, which), 1, 56.0, actions);
                });
            }
        }
    });
}
