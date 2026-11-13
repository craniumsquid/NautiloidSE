//! The Party tab: each character's experience, level-ups, ability points and gold, and the
//! party's Inspiration.

use super::widgets::{self, grouped};
use super::{theme, Action};
use crate::party::{self, Character};
use crate::session::{Field, Session};
use eframe::egui::{self, Align, Layout, Ui};

/// Below this width the character cards go in one column.
const TWO_COLUMNS_FROM: f32 = 860.0;

pub fn show(ui: &mut Ui, session: &Session, actions: &mut Vec<Action>) {
    ui.label(egui::RichText::new("Party").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new(
            "Experience, level-ups, ability points and gold. Nothing is written until you export.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(10.0);

    shared(ui, session, actions);
    ui.add_space(8.0);

    if ui.available_width() >= TWO_COLUMNS_FROM {
        for pair in session.order().chunks(2) {
            ui.columns(2, |columns| {
                for (column, &index) in columns.iter_mut().zip(pair) {
                    character_card(column, session, index, actions);
                }
            });
            ui.add_space(8.0);
        }
    } else {
        for &index in session.order() {
            character_card(ui, session, index, actions);
            ui.add_space(8.0);
        }
    }
}

/// A stepper for `field` with a button to put it back when it has changed.
fn field_stepper(
    ui: &mut Ui,
    session: &Session,
    field: Field,
    step: u32,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Some(value) = session.value(field) else {
        return;
    };
    let changed = session.is_changed(field);
    if let Some(new) = widgets::stepper(ui, value, session.range(field), step, width, changed) {
        actions.push(Action::Set(field, new));
    }
    if changed {
        let original = session.original_value(field).unwrap_or(value);
        let tip = format!("Put back {}", grouped(original as u64));
        if widgets::icon_button(ui, "⟲", &tip).clicked() {
            actions.push(Action::Revert(field));
        }
    }
}

/// What the party shares.
fn shared(ui: &mut Ui, session: &Session, actions: &mut Vec<Action>) {
    let party = session.party();
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, "The party");
        ui.add_space(2.0);
        match party.inspiration {
            Some(inspiration) => {
                let hint = format!("Up to {} in this save.", inspiration.max);
                widgets::row(ui, "Inspiration", Some(&hint), |ui| {
                    field_stepper(ui, session, Field::Inspiration, 1, 56.0, actions);
                });
            }
            None => widgets::row(
                ui,
                "Inspiration",
                Some(
                    "This save has none yet. Earn a point in the game and it can be changed here.",
                ),
                |_| {},
            ),
        }
        widgets::row(
            ui,
            "Gold",
            Some("The total of what each character carries, below."),
            |ui| {
                ui.label(
                    egui::RichText::new(grouped(party.gold_total()))
                        .font(theme::semibold(16.0))
                        .color(theme::TEXT_SOFT),
                );
            },
        );
        if let Some(supplies) = party.camp_supplies {
            widgets::row(
                ui,
                "Camp supplies",
                Some("Worked out by the game from the food you carry, so it can't be set here."),
                |ui| {
                    ui.label(
                        egui::RichText::new(grouped(supplies as u64))
                            .font(theme::semibold(16.0))
                            .color(theme::MUTED),
                    );
                },
            );
        }
    });
}

/// "Level 5 · Sorcerer / Warlock", with the classes from the save's own summary.
fn subtitle(session: &Session, index: usize, character: &Character) -> String {
    let classes = session
        .summary_of(index)
        .map(|member| {
            member
                .classes
                .iter()
                .map(|c| c.main.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    if classes.is_empty() {
        format!("Level {}", character.level)
    } else {
        format!("Level {} · {classes}", character.level)
    }
}

fn character_card(ui: &mut Ui, session: &Session, index: usize, actions: &mut Vec<Action>) {
    let character = &session.party().characters[index];
    let original = &session.original().characters[index];
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(character.display_name()).font(theme::semibold(20.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(subtitle(session, index, character)).color(theme::MUTED),
                );
            });
        });
        ui.add_space(4.0);

        widgets::row(
            ui,
            "Experience",
            Some("The total. The game shows experience into the current level."),
            |ui| field_stepper(ui, session, Field::Experience(index), 100, 84.0, actions),
        );
        let offered = session.value(Field::LevelUps(index)).unwrap_or(0);
        let hint = match offered {
            0 => "The game takes you through the choices for each one.".to_owned(),
            n => format!(
                "The game will offer level {}.",
                (1..=n)
                    .map(|i| (character.level + i).to_string())
                    .collect::<Vec<_>>()
                    .join(", then ")
            ),
        };
        widgets::row(ui, "Level-ups to offer", Some(&hint), |ui| {
            field_stepper(ui, session, Field::LevelUps(index), 1, 56.0, actions);
        });

        ui.add_space(6.0);
        widgets::section_title(ui, "Ability points");
        match (character.allocated, original.allocated) {
            (Some(allocated), Some(before)) => {
                ui.label(
                    egui::RichText::new(format!(
                        "Points above the starting {}. The game adds the character's other bonuses.",
                        party::ABILITY_BASE
                    ))
                    .font(theme::body(12.5))
                    .color(theme::MUTED),
                );
                for (ability, name) in party::ABILITIES.iter().enumerate() {
                    // The score the game will show: its current total, moved by however far
                    // the points have moved.
                    let total = character.abilities[ability] as i64 + allocated[ability] as i64
                        - before[ability] as i64;
                    let moved = allocated[ability] != before[ability];
                    ui.horizontal(|ui| {
                        ui.set_min_height(36.0);
                        ui.label(egui::RichText::new(*name).font(theme::medium(15.0)));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            // A fixed column for the score, so the steppers line up
                            // whether it has one digit or two.
                            let (cell, _) = ui
                                .allocate_exact_size(egui::vec2(72.0, 30.0), egui::Sense::hover());
                            ui.painter().text(
                                egui::pos2(cell.right(), cell.center().y),
                                egui::Align2::RIGHT_CENTER,
                                total.max(0).to_string(),
                                theme::bold(17.0),
                                if moved { theme::CHANGED } else { theme::TEXT },
                            );
                            ui.painter().text(
                                egui::pos2(cell.right() - 28.0, cell.center().y + 1.0),
                                egui::Align2::RIGHT_CENTER,
                                "score",
                                theme::body(12.0),
                                theme::MUTED,
                            );
                            field_stepper(
                                ui,
                                session,
                                Field::Allocated(index, ability),
                                1,
                                44.0,
                                actions,
                            );
                        });
                    });
                }
            }
            _ => {
                ui.label(
                    egui::RichText::new(
                        "This character has no ability points stored in the save, so they can't \
                         be changed here.",
                    )
                    .color(theme::MUTED),
                );
            }
        }

        ui.add_space(6.0);
        widgets::section_title(ui, "Gold");
        if character.gold.is_empty() {
            ui.label(
                egui::RichText::new(
                    "Carries none. Gold can only be changed where a character already has some.",
                )
                .font(theme::body(12.5))
                .color(theme::MUTED),
            );
        }
        let loose = character.gold.iter().filter(|g| !g.in_container).count();
        for (stack, gold) in character.gold.iter().enumerate() {
            let label = if gold.in_container {
                "In a bag or container".to_owned()
            } else if loose > 1 {
                format!("Carried (stack {})", stack + 1)
            } else {
                "Carried".to_owned()
            };
            widgets::row(ui, &label, None, |ui| {
                field_stepper(ui, session, Field::Gold(index, stack), 100, 96.0, actions);
            });
        }
    });
}
