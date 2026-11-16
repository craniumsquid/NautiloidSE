//! The Story tab: what the game's story is keeping track of in this save. Read only: the
//! story's rules run when the game changes one of these, not when a save is loaded, so a
//! change made here would leave undone whatever the game does about it.

use super::{theme, widgets};
use crate::session::Session;
use crate::story::{Flag, State, Tally};
use eframe::egui::{self, Align, Id, Layout, Margin, Ui};

/// How tall the list of flags is before it scrolls.
const FLAGS_HEIGHT: f32 = 420.0;
const FLAG_ROW: f32 = 24.0;

pub fn show(ui: &mut Ui, session: &Session) {
    ui.label(egui::RichText::new("Story").font(theme::display(30.0)));
    ui.label(
        egui::RichText::new(
            "What the game's story is keeping track of in this save, under the game's own \
             names. This tab only shows it: nothing here can be changed.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(10.0);

    let state = match session.story() {
        Ok(state) => state,
        Err(why) => {
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new("The story in this save couldn't be read.")
                        .font(theme::medium(15.0)),
                );
                ui.label(egui::RichText::new(why).color(theme::MUTED));
            });
            return;
        }
    };

    if !state.tallies.is_empty() {
        tallies(ui, state);
        ui.add_space(8.0);
    }
    counters(ui, state);
    ui.add_space(8.0);
    flags(ui, state);
}

/// A small dot: filled for something that is so, a ring for something that isn't.
fn dot(ui: &mut Ui, on: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let painter = ui.painter();
    if on {
        painter.circle_filled(rect.center(), 4.5, theme::FINE);
    } else {
        painter.circle_stroke(rect.center(), 4.0, theme::stroke(1.5, theme::MUTED));
    }
}

/// "GaveNightOrchid" as "Gave Night Orchid".
pub fn spaced(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut before: Option<char> = None;
    for c in name.chars() {
        let breaks = before.is_some_and(|b| {
            (c.is_uppercase() && (b.is_lowercase() || b.is_ascii_digit()))
                || (c.is_ascii_digit() && b.is_alphabetic())
        });
        if breaks {
            out.push(' ');
        }
        out.push(c);
        before = Some(c);
    }
    out
}

/// A name of the story's as a title: "ORI_Shadowheart_NightsongPoints" as "Shadowheart ·
/// Nightsong Points". The first part, when it is all capitals, says which part of the game
/// it belongs to and is left out.
pub fn title(name: &str) -> String {
    let mut parts: Vec<&str> = name.split('_').filter(|p| !p.is_empty()).collect();
    let area = |part: &str| {
        part.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    if parts.len() > 1 && area(parts[0]) {
        parts.remove(0);
    }
    parts
        .iter()
        .map(|part| spaced(part))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn tallies(ui: &mut Ui, state: &State) {
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, "Points the story is counting");
        ui.label(
            egui::RichText::new(
                "Each point comes from something that happened. The game decides what the \
                 total leads to.",
            )
            .font(theme::body(12.5))
            .color(theme::MUTED),
        );
        ui.add_space(4.0);
        for (i, tally) in state.tallies.iter().enumerate() {
            if i > 0 {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(2.0);
            }
            ui.scope(|ui| one_tally(ui, tally));
        }
    });
}

fn one_tally(ui: &mut Ui, tally: &Tally) {
    let (name, count) = match &tally.counter {
        Some((name, count)) => (name.as_str(), Some(*count)),
        None => (tally.name.as_str(), None),
    };
    widgets::row(ui, &title(name), Some(name), |ui| {
        let done = tally.events.iter().filter(|e| e.done).count() as i64;
        let text = widgets::signed(count.unwrap_or(done));
        ui.label(
            egui::RichText::new(text)
                .font(theme::semibold(18.0))
                .color(theme::TEXT),
        );
        match tally.enough {
            Some(true) => {
                widgets::badge(ui, "ENOUGH", theme::FINE, theme::FINE_BG);
            }
            Some(false) => {
                widgets::badge(ui, "NOT ENOUGH YET", theme::MUTED, theme::WIDGET);
            }
            None => {}
        }
    });
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.spacing_mut().interact_size.y = FLAG_ROW;
    for event in &tally.events {
        let what = event.name.rsplit('_').next().unwrap_or(&event.name);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            dot(ui, event.done);
            let text = egui::RichText::new(spaced(what)).font(theme::body(14.0));
            ui.label(if event.done {
                text.color(theme::TEXT)
            } else {
                text.color(theme::MUTED)
            })
            .on_hover_text(&event.name);
            if !event.done {
                ui.label(
                    egui::RichText::new("not yet")
                        .font(theme::body(12.0))
                        .color(theme::MUTED),
                );
            }
        });
    }
}

fn counters(ui: &mut Ui, state: &State) {
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &format!("Counters ({})", state.counters.len()));
        ui.add_space(2.0);
        if state.counters.is_empty() {
            ui.label(egui::RichText::new("The story has none yet.").color(theme::MUTED));
        }
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().interact_size.y = FLAG_ROW;
        for (name, value) in &state.counters {
            ui.horizontal(|ui| {
                ui.set_min_height(FLAG_ROW);
                ui.label(egui::RichText::new(name).font(theme::body(14.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let color = if *value == 0 {
                        theme::MUTED
                    } else {
                        theme::TEXT
                    };
                    ui.label(
                        egui::RichText::new(widgets::signed(*value))
                            .font(theme::semibold(15.0))
                            .color(color),
                    );
                });
            });
        }
    });
}

/// Whether a flag matches the words typed: every word, in its name or its identifier.
fn matches(flag: &Flag, words: &[String]) -> bool {
    let name = flag.name.as_deref().unwrap_or("").to_lowercase();
    let id = flag.id.to_string();
    words.iter().all(|w| name.contains(w) || id.contains(w))
}

fn flags(ui: &mut Ui, state: &State) {
    let search_id = Id::new("story-flag-search");
    let set_id = Id::new("story-flag-set-only");
    let mut search = ui
        .data(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    let mut only_set = ui.data(|d| d.get_temp::<bool>(set_id)).unwrap_or(false);

    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        let set = state.flags.iter().filter(|f| f.set).count();
        widgets::section_title(ui, &format!("Flags ({})", state.flags.len()));
        ui.label(
            egui::RichText::new(format!(
                "The flags the game has touched in this playthrough: {set} are set, the rest \
                 were set once and cleared. A flag that isn't listed has never been set."
            ))
            .font(theme::body(12.5))
            .color(theme::MUTED),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut search)
                    .hint_text("Filter flags, for example: Shadowheart")
                    .desired_width(320.0)
                    .margin(Margin::symmetric(10.0, 7.0)),
            );
            ui.add_space(8.0);
            ui.checkbox(&mut only_set, "Only flags that are set");
        });
        ui.add_space(6.0);

        let words: Vec<String> = search
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();
        let shown: Vec<&Flag> = state
            .flags
            .iter()
            .filter(|f| (!only_set || f.set) && matches(f, &words))
            .collect();
        if shown.is_empty() {
            ui.label(egui::RichText::new("None match.").color(theme::MUTED));
            return;
        }
        if shown.len() != state.flags.len() {
            ui.label(
                egui::RichText::new(format!("{} match.", shown.len()))
                    .font(theme::body(12.5))
                    .color(theme::MUTED),
            );
        }
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().interact_size.y = FLAG_ROW;
        egui::ScrollArea::vertical()
            .id_salt("story-flags")
            .max_height(FLAGS_HEIGHT)
            .min_scrolled_height(FLAGS_HEIGHT)
            .auto_shrink([false, true])
            .show_rows(ui, FLAG_ROW, shown.len(), |ui, rows| {
                for flag in &shown[rows] {
                    ui.horizontal(|ui| {
                        ui.set_min_height(FLAG_ROW);
                        dot(ui, flag.set);
                        let color = if flag.set { theme::FINE } else { theme::MUTED };
                        let (text, named) = match &flag.name {
                            Some(name) => (name.clone(), true),
                            None => (flag.id.to_string(), false),
                        };
                        let text = egui::RichText::new(text).font(theme::body(14.0));
                        ui.label(if named && flag.set {
                            text.color(theme::TEXT)
                        } else {
                            text.color(theme::MUTED)
                        });
                        if !named {
                            ui.label(
                                egui::RichText::new("no name in this save")
                                    .font(theme::body(12.0))
                                    .color(theme::MUTED),
                            );
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(12.0);
                            let state = if flag.set { "set" } else { "cleared" };
                            ui.label(
                                egui::RichText::new(state)
                                    .font(theme::body(12.5))
                                    .color(color),
                            );
                        });
                    });
                }
            });
    });

    ui.data_mut(|d| {
        d.insert_temp(search_id, search);
        d.insert_temp(set_id, only_set);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_titles() {
        assert_eq!(spaced("GaveNightOrchid"), "Gave Night Orchid");
        assert_eq!(spaced("Romance2Night"), "Romance 2 Night");
        assert_eq!(
            title("ORI_Shadowheart_NightsongPoints"),
            "Shadowheart · Nightsong Points"
        );
        assert_eq!(title("Camp_Rest_Count"), "Camp · Rest · Count");
        assert_eq!(title("GLO"), "GLO");
    }
}
