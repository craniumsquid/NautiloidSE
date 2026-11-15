//! The dialogs: exporting, leaving with changes unexported, choosing items to add, and
//! saying what went wrong.

use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::catalog::CatalogItem;
use crate::party::Holder;
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
    /// Choosing items from the game's to add to what someone holds.
    AddItem { holder: Holder, query: String },
    /// Something could not be done.
    Problem { title: String, detail: String },
}

/// How many matching items the add-item dialog shows at once.
const SHOWN: usize = 60;

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
                    ui.set_width(match dialog {
                        Dialog::AddItem { .. } => 640.0,
                        _ => 460.0,
                    });
                    ui.spacing_mut().item_spacing.y = 10.0;
                    match dialog {
                        Dialog::Export { name, error, .. } => {
                            export(ui, name, error, session, actions)
                        }
                        Dialog::Unexported { then } => unexported(ui, then, session, actions),
                        Dialog::AddItem { holder, query } => {
                            add_item(ui, *holder, query, session, actions)
                        }
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

fn add_item(
    ui: &mut Ui,
    holder: Holder,
    query: &mut String,
    session: Option<&Session>,
    actions: &mut Vec<Action>,
) {
    let Some(session) = session else { return };
    let who = session.holder_in_sentence(holder);
    title(ui, &format!("Add items for {who}"));
    ui.label(
        egui::RichText::new(format!(
            "Each new item is a copy of one {who} already holds, made into the item you \
             pick: a weapon from a weapon, armour from armour worn the same way, a potion \
             from a potion. Worn, equipped and story items, and stacks, can't be copied."
        ))
        .color(theme::TEXT_SOFT),
    );
    let Some(catalog) = session.catalog() else {
        ui.label(
            egui::RichText::new(
                "The game's files haven't been read, so there are no items to choose from.",
            )
            .color(theme::CORAL),
        );
        buttons(ui, |ui| {
            if widgets::button(ui, "Close", Kind::Secondary).clicked() {
                actions.push(Action::CloseDialog);
            }
        });
        return;
    };
    let field = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text("Search the game's items")
            .desired_width(f32::INFINITY)
            .font(theme::medium(16.0))
            .margin(Margin::symmetric(10.0, 8.0)),
    );
    // Put the cursor in the search when the dialog opens.
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }

    let wanted = query.trim().to_lowercase();
    if wanted.chars().count() < 2 {
        ui.label(
            egui::RichText::new(format!(
                "Type part of a name to search the game's {} items.",
                widgets::grouped(catalog.listed().count() as u64)
            ))
            .color(theme::MUTED),
        );
    } else {
        let mut found: Vec<&CatalogItem> = catalog.search(&wanted).collect();
        // Names that start with what was typed first, each lot by name.
        found.sort_by_key(|item| !item.display.to_lowercase().starts_with(&wanted));
        if found.is_empty() {
            ui.label(
                egui::RichText::new("No item in the game's files is called that.")
                    .color(theme::MUTED),
            );
        }
        egui::ScrollArea::vertical()
            .max_height(340.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for item in found.iter().take(SHOWN) {
                    found_item(ui, session, holder, item, actions);
                }
            });
        if found.len() > SHOWN {
            ui.label(
                egui::RichText::new(format!(
                    "{} more. Type more of the name to find them.",
                    found.len() - SHOWN
                ))
                .font(theme::body(12.5))
                .color(theme::MUTED),
            );
        }
    }
    buttons(ui, |ui| {
        if widgets::button(ui, "Done", Kind::Secondary).clicked() {
            actions.push(Action::CloseDialog);
        }
    });
}

/// One item the search found: what it is, and a way to add it or why it can't be.
fn found_item(
    ui: &mut Ui,
    session: &Session,
    holder: Holder,
    item: &CatalogItem,
    actions: &mut Vec<Action>,
) {
    let mut about = vec![item.kind.label().to_owned()];
    if !item.category.is_empty() {
        about.push(words(&item.category));
    }
    if !item.rarity.is_empty() && item.rarity != "Common" {
        about.push(item.rarity.clone());
    }
    about.push(item.stats.clone());
    let pattern = session.pattern_for(holder, item);
    let story = item.story.then_some(
        "A story item: the game's quests may count on there being only the one, so a second \
         may confuse them.",
    );
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let room = (ui.available_width() - 150.0).max(200.0);
        ui.vertical(|ui| {
            ui.set_max_width(room);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(egui::RichText::new(session.catalog_name(item)).font(theme::medium(15.0)));
            ui.label(
                egui::RichText::new(about.join(" · "))
                    .font(theme::body(12.0))
                    .color(theme::MUTED),
            );
            if let (Some(story), Ok(_)) = (story, &pattern) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(story)
                            .font(theme::body(12.0))
                            .color(theme::ACCENT),
                    )
                    .wrap(),
                );
            }
            if let Err(why) = &pattern {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(why)
                            .font(theme::body(12.0))
                            .color(theme::CORAL),
                    )
                    .wrap(),
                );
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pattern.is_ok() {
                let added = session.added(holder, &item.stats);
                if widgets::button(ui, "Add", Kind::Secondary).clicked() {
                    actions.push(Action::AddItem(holder, item.stats.clone()));
                }
                if added > 0 {
                    ui.label(egui::RichText::new(format!("{added} added")).color(theme::ACCENT));
                }
            }
        });
    });
}

/// `MagicScroll` as "Magic scroll".
fn words(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c == '_' {
            out.push(' ');
        } else if i > 0 && c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
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

    #[test]
    fn splits_category_names_into_words() {
        assert_eq!(super::words("MagicScroll"), "Magic scroll");
        assert_eq!(super::words("Helmet"), "Helmet");
        assert_eq!(super::words("VanityBody"), "Vanity body");
    }
}
