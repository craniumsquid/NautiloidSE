//! The Inventory tab: what each character and the camp chest hold, how many each stack
//! holds, and items to add.

use super::game::{self, GameFiles};
use super::party::field_stepper;
use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::party::{Holder, Item};
use crate::session::{Field, Session};
use eframe::egui::{self, Id, Margin, Ui};

/// The game's name for an item and what else is worth knowing about it.
fn detail(item: &Item) -> String {
    let mut detail = item.stats.clone();
    if let Some(supply) = item.supply {
        detail.push_str(&format!(" · {supply} camp supplies each"));
    }
    if item.in_container {
        detail.push_str(" · in a container");
    }
    detail
}

pub fn show(ui: &mut Ui, session: &Session, game: &GameFiles, actions: &mut Vec<Action>) {
    ui.label(egui::RichText::new("Inventory").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new("What everyone carries, how many each stack holds, and items to add.")
            .color(theme::MUTED),
    );
    ui.add_space(10.0);
    game::notice(ui, game, actions);

    let party = session.party();
    if party.inventories.is_empty() {
        ui.label(egui::RichText::new("Nobody in this save holds anything.").color(theme::MUTED));
        return;
    }

    // Whose inventory is showing: the party, then those at camp, then the camp chest.
    let mut holders: Vec<usize> = (0..party.inventories.len()).collect();
    holders.sort_by_key(|&i| {
        match party.inventories[i].holder {
            Holder::Character(c) => session.order().iter().position(|&o| o == c),
            Holder::CampChest => None,
        }
        .unwrap_or(usize::MAX)
    });
    let chosen_id = Id::new("inventory-holder");
    let mut chosen = ui
        .data(|d| d.get_temp::<usize>(chosen_id))
        .filter(|chosen| *chosen < party.inventories.len())
        .unwrap_or(holders[0]);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for &index in &holders {
            let name = session.holder_name(party.inventories[index].holder);
            let kind = if index == chosen {
                Kind::Secondary
            } else {
                Kind::Quiet
            };
            if widgets::button(ui, &name, kind).clicked() {
                chosen = index;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(chosen_id, chosen));
    ui.add_space(6.0);

    let search_id = Id::new("inventory-search");
    let mut search = ui
        .data(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    let inventory = &party.inventories[chosen];
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut search)
                .hint_text("Search items")
                .desired_width(280.0)
                .margin(Margin::symmetric(10.0, 7.0)),
        );
        let can_add = session.catalog().is_some();
        let add = ui
            .add_enabled_ui(can_add, |ui| {
                widgets::button(ui, "Add items…", Kind::Secondary)
            })
            .inner;
        if add.clicked() {
            actions.push(Action::AskAddItem(inventory.holder));
        }
        if !can_add {
            add.on_disabled_hover_text("Needs the game's files, for what items there are.");
        }
    });
    ui.data_mut(|d| d.insert_temp(search_id, search.clone()));
    ui.add_space(8.0);
    adding(ui, session, inventory.holder, actions);
    let wanted = search.trim().to_lowercase();
    let matches = |item: &Item| {
        wanted.is_empty()
            || item.stats.to_lowercase().contains(&wanted)
            || session.item_name(item).to_lowercase().contains(&wanted)
    };
    let items = || {
        inventory
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| matches(i))
    };
    let stacks = items().filter(|(_, item)| item.stacked).count();
    let singles = items().count() - stacks;

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Stacks ({stacks})"));
        ui.add_space(2.0);
        if stacks == 0 {
            ui.label(egui::RichText::new("No stacks here.").color(theme::MUTED));
        }
        for (index, item) in items().filter(|(_, item)| item.stacked) {
            widgets::row(ui, &session.item_name(item), Some(&detail(item)), |ui| {
                field_stepper(ui, session, Field::Amount(chosen, index), 1, 64.0, actions);
            });
        }
    });
    ui.add_space(8.0);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Single items ({singles})"));
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "One of each: the save keeps no count for these, so there is none to \
                     change. To have more, press + beside one. An item that stacks is added \
                     as a new stack, whose size you set under Adding; anything else is added \
                     one at a time.",
                )
                .font(theme::body(12.5))
                .color(theme::MUTED),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        let columns = if ui.available_width() >= 700.0 { 2 } else { 1 };
        egui::Grid::new("single-items")
            .num_columns(columns)
            .min_col_width(ui.available_width() / columns as f32 - 12.0)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for (position, (_, item)) in items().filter(|(_, item)| !item.stacked).enumerate() {
                    ui.horizontal(|ui| {
                        more_button(ui, session, inventory.holder, item, actions);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(
                                egui::RichText::new(session.item_name(item))
                                    .color(theme::TEXT_SOFT),
                            );
                            ui.label(
                                egui::RichText::new(detail(item))
                                    .font(theme::body(12.0))
                                    .color(theme::MUTED),
                            );
                        });
                    });
                    if (position + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
    });
}

/// The items to be added for `holder`, with how many of each.
fn adding(ui: &mut Ui, session: &Session, holder: Holder, actions: &mut Vec<Action>) {
    let mine: Vec<usize> = (0..session.additions().len())
        .filter(|&i| session.additions()[i].holder == holder)
        .collect();
    if mine.is_empty() {
        return;
    }
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Adding ({})", mine.len()));
        ui.label(
            egui::RichText::new("New items go at the end of the inventory when you export.")
                .font(theme::body(12.5))
                .color(theme::MUTED),
        );
        ui.add_space(2.0);
        for index in mine {
            let addition = &session.additions()[index];
            let hint = format!("{} · {}", session.addition_hint(index), addition.item.stats);
            widgets::row(
                ui,
                &session.catalog_name(&addition.item),
                Some(&hint),
                |ui| field_stepper(ui, session, Field::Add(index), 1, 64.0, actions),
            );
        }
    });
    ui.add_space(8.0);
}

/// A + that adds one more of `item` for `holder`, or says why it can't.
fn more_button(
    ui: &mut Ui,
    session: &Session,
    holder: Holder,
    item: &Item,
    actions: &mut Vec<Action>,
) {
    let known = session
        .catalog()
        .and_then(|c| c.by_stats(&item.stats))
        .cloned();
    let why_not = match &known {
        None if session.catalog().is_none() => {
            Some("Needs the game's files, for what items there are.".to_owned())
        }
        None => Some("The game's files don't have this item, so it can't be added.".to_owned()),
        Some(known) => session.pattern_for(holder, known).err(),
    };
    let button = ui
        .add_enabled_ui(why_not.is_none(), |ui| {
            widgets::small_button(ui, "+", Kind::Quiet)
        })
        .inner;
    match why_not {
        Some(why) => {
            button.on_disabled_hover_text(why);
        }
        None => {
            let tip = match known.is_some_and(|k| k.max_stack > 1) {
                true => "Add more of these, as a new stack",
                false => "Add another of these",
            };
            if button.on_hover_text(tip).clicked() {
                actions.push(Action::AddItem(holder, item.stats.clone()));
            }
        }
    }
}
