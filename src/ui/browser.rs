//! The save browser: the saves folder, its campaigns, and a card for each save.

use super::thumbs::Thumbnails;
use super::widgets::{self, Kind};
use super::{theme, Action};
use crate::save::Support;
use crate::saves::{self, Campaign, Listed, SaveKind, SaveSummary, Unreadable};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Frame, Layout, Margin, Pos2, Rect, Rounding, Sense,
    Ui, Vec2,
};
use std::path::PathBuf;

const CARD_WIDTH: f32 = 264.0;
const CARD_GAP: f32 = 16.0;
const PICTURE_HEIGHT: f32 = 148.0;
const CARD_HEIGHT: f32 = PICTURE_HEIGHT + 128.0;
const CAMPAIGNS_WIDTH: f32 = 248.0;

pub struct BrowserState {
    pub dir: Option<PathBuf>,
    /// Whether `dir` was found by looking, not chosen.
    pub detected: bool,
    pub listed: Vec<Listed>,
    pub campaigns: Vec<Campaign>,
    /// The campaign whose saves are showing.
    pub selected: Option<String>,
    pub search: String,
    /// Why the folder could not be read.
    pub error: Option<String>,
}

impl BrowserState {
    pub fn new(dir: Option<PathBuf>, detected: bool) -> BrowserState {
        let mut state = BrowserState {
            dir,
            detected,
            listed: Vec::new(),
            campaigns: Vec::new(),
            selected: None,
            search: String::new(),
            error: None,
        };
        state.scan();
        state
    }

    /// Reads the folder again.
    pub fn scan(&mut self) {
        self.listed.clear();
        self.error = None;
        if let Some(dir) = &self.dir {
            match saves::scan(dir) {
                Ok(listed) => self.listed = listed,
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
        self.campaigns = saves::campaigns(&self.listed);
        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|id| self.campaigns.iter().any(|c| &c.id == id));
        if !still_there {
            self.selected = self.campaigns.first().map(|c| c.id.clone());
        }
    }

    pub fn set_dir(&mut self, dir: PathBuf) {
        self.dir = Some(dir);
        self.detected = false;
        self.selected = None;
        self.search.clear();
        self.scan();
    }

    fn unreadable(&self) -> impl Iterator<Item = &Unreadable> {
        self.listed.iter().filter_map(|l| match l {
            Listed::Unreadable(u) => Some(u),
            Listed::Save(_) => None,
        })
    }

    fn save_count(&self) -> usize {
        self.campaigns.iter().map(|c| c.saves.len()).sum()
    }
}

/// A name for the level a save is in. "Wilderness" and "High Hall" are what the game itself
/// calls those two in save names; the rest are the areas the level codes stand for. A code
/// that is not listed is shown as it is.
pub fn level_name(code: &str) -> &str {
    match code {
        "TUT_Avernus_C" => "Nautiloid",
        "WLD_Main_A" => "Wilderness",
        "CRE_Main_A" => "Mountain Pass",
        "SCL_Main_A" => "Shadow-Cursed Lands",
        "BGO_Main_A" => "Rivington",
        "CTY_Main_A" => "Lower City",
        "IRN_Main_A" => "Iron Throne",
        "END_Main" => "High Hall",
        other => other,
    }
}

/// "Level 5 Sorcerer / Warlock" for the first character the save lists.
pub fn party_line(summary: &SaveSummary) -> String {
    let Some(first) = summary.party.first() else {
        return String::new();
    };
    let classes: Vec<&str> = first.classes.iter().map(|c| c.main.as_str()).collect();
    if classes.is_empty() {
        format!("Level {}", first.level)
    } else {
        format!("Level {} {}", first.level, classes.join(" / "))
    }
}

pub fn show(
    ui: &mut Ui,
    state: &mut BrowserState,
    thumbs: &mut Thumbnails,
    now: u64,
    hovering_files: bool,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width().min(1320.0);
    let side = ((ui.available_width() - width) / 2.0).max(0.0);
    Frame::none()
        .inner_margin(Margin {
            left: side + 32.0,
            right: side + 32.0,
            top: 26.0,
            bottom: 0.0,
        })
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Choose a save").font(theme::display(34.0)));
            let subtitle = match (state.save_count(), state.campaigns.len()) {
                (0, _) => "Edits go into a new save, so your saves are never changed.".to_owned(),
                (saves, campaigns) => format!(
                    "{saves} save{} in {campaigns} campaign{}. Edits go into a new save, so your \
                     saves are never changed.",
                    if saves == 1 { "" } else { "s" },
                    if campaigns == 1 { "" } else { "s" },
                ),
            };
            ui.label(egui::RichText::new(subtitle).color(theme::MUTED));
            ui.add_space(8.0);
            folder_bar(ui, state, actions);
            ui.add_space(8.0);

            if let Some(error) = &state.error {
                notice(ui, "This folder could not be read", error);
            } else if state.dir.is_none() {
                drop_zone(
                    ui,
                    "Your saves folder was not found",
                    "Drop a save folder or your Savegames\\Story folder here, or choose the folder.",
                    hovering_files,
                    actions,
                );
            } else if state.campaigns.is_empty() && state.unreadable().next().is_none() {
                drop_zone(
                    ui,
                    "No saves in this folder",
                    "Drop a save folder here, or choose another folder.",
                    hovering_files,
                    actions,
                );
            } else {
                columns(ui, state, thumbs, now, actions);
            }
        });
}

fn folder_bar(ui: &mut Ui, state: &BrowserState, actions: &mut Vec<Action>) {
    Frame::none()
        .fill(theme::PANEL_ALT)
        .stroke(theme::stroke(1.0, theme::LINE))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(18.0, 12.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let label = if state.detected {
                        "Saves folder (found automatically)"
                    } else {
                        "Saves folder"
                    };
                    ui.label(
                        egui::RichText::new(label)
                            .font(theme::body(12.5))
                            .color(theme::MUTED),
                    );
                    let path = match &state.dir {
                        Some(dir) => dir.display().to_string(),
                        None => "Not set".to_owned(),
                    };
                    ui.set_max_width(ui.available_width() - 300.0);
                    widgets::clipped(ui, &path, theme::mono(13.5), theme::TEXT_SOFT)
                        .on_hover_text(&path);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, "Change folder…", Kind::Secondary).clicked() {
                        actions.push(Action::PickFolder);
                    }
                    ui.add_enabled_ui(state.dir.is_some(), |ui| {
                        if widgets::button(ui, "Refresh", Kind::Quiet).clicked() {
                            actions.push(Action::Refresh);
                        }
                    });
                });
            });
        });
}

fn notice(ui: &mut Ui, title: &str, detail: &str) {
    Frame::none()
        .fill(theme::CORAL_BG)
        .stroke(theme::stroke(1.0, theme::CORAL_LINE))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .font(theme::semibold(16.0))
                    .color(theme::CORAL),
            );
            ui.label(egui::RichText::new(detail).color(theme::TEXT_SOFT));
        });
}

fn drop_zone(ui: &mut Ui, title: &str, detail: &str, hovering: bool, actions: &mut Vec<Action>) {
    let height = (ui.available_height() - 32.0).clamp(200.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let (fill, line) = if hovering {
        (theme::ACCENT_BG, theme::ACCENT)
    } else {
        (theme::PANEL_ALT, theme::LINE_STRONG)
    };
    ui.painter()
        .rect(rect, Rounding::same(14.0), fill, theme::stroke(1.5, line));
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(24.0))
            .layout(Layout::top_down(Align::Center)),
    );
    content.add_space((height - 150.0).max(0.0) / 2.0);
    content.label(egui::RichText::new(title).font(theme::semibold(20.0)));
    content.label(egui::RichText::new(detail).color(theme::MUTED));
    content.add_space(10.0);
    if widgets::button(&mut content, "Choose folder…", Kind::Primary).clicked() {
        actions.push(Action::PickFolder);
    }
}

fn columns(
    ui: &mut Ui,
    state: &mut BrowserState,
    thumbs: &mut Thumbnails,
    now: u64,
    actions: &mut Vec<Action>,
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.allocate_ui_with_layout(
            Vec2::new(CAMPAIGNS_WIDTH, height),
            Layout::top_down(Align::Min),
            |ui| campaign_list(ui, state, now, actions),
        );
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), height),
            Layout::top_down(Align::Min),
            |ui| save_grid(ui, state, thumbs, now, actions),
        );
    });
}

fn campaign_list(ui: &mut Ui, state: &BrowserState, now: u64, actions: &mut Vec<Action>) {
    widgets::section_title(ui, "Campaigns");
    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .id_salt("campaigns")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for campaign in &state.campaigns {
                let selected = state.selected.as_ref() == Some(&campaign.id);
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width() - 10.0, 58.0),
                    Sense::click(),
                );
                if ui.is_rect_visible(rect) {
                    let (fill, line) = if selected {
                        (theme::ACCENT_BG, theme::ACCENT_LINE)
                    } else if response.hovered() {
                        (theme::WIDGET, theme::LINE_STRONG)
                    } else {
                        (theme::PANEL_ALT, theme::LINE)
                    };
                    ui.painter()
                        .rect(rect, Rounding::same(10.0), fill, theme::stroke(1.0, line));
                    let left = rect.left() + 14.0;
                    let painter = ui
                        .painter()
                        .with_clip_rect(rect.shrink2(Vec2::new(12.0, 0.0)));
                    painter.text(
                        Pos2::new(left, rect.top() + 19.0),
                        Align2::LEFT_CENTER,
                        &campaign.leader,
                        theme::semibold(16.0),
                        theme::TEXT,
                    );
                    let newest = campaign.saves.first().map_or(0, |s| s.saved_at);
                    let count = campaign.saves.len();
                    painter.text(
                        Pos2::new(left, rect.top() + 40.0),
                        Align2::LEFT_CENTER,
                        format!(
                            "{count} save{} · {}",
                            if count == 1 { "" } else { "s" },
                            saves::ago(now.saturating_sub(newest))
                        ),
                        theme::body(12.5),
                        theme::MUTED,
                    );
                    if campaign.honour_mode() {
                        widgets::badge_at(
                            ui,
                            Pos2::new(rect.right() - 74.0, rect.top() + 9.0),
                            "HONOUR",
                            theme::ACCENT,
                            theme::ACCENT_BG,
                        );
                    }
                }
                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    actions.push(Action::SelectCampaign(campaign.id.clone()));
                }
            }
            let unreadable = state.unreadable().count();
            if unreadable > 0 {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{unreadable} folder{} could not be read",
                        if unreadable == 1 { "" } else { "s" }
                    ))
                    .font(theme::body(12.5))
                    .color(theme::CORAL),
                )
                .on_hover_ui(|ui| {
                    for entry in state.unreadable() {
                        ui.label(format!("{}: {}", entry.name, entry.error));
                    }
                });
            }
        });
}

fn save_grid(
    ui: &mut Ui,
    state: &mut BrowserState,
    thumbs: &mut Thumbnails,
    now: u64,
    actions: &mut Vec<Action>,
) {
    let Some(campaign) = state
        .campaigns
        .iter()
        .find(|c| Some(&c.id) == state.selected.as_ref())
    else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&campaign.leader).font(theme::semibold(20.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search saves")
                    .margin(Margin::symmetric(10.0, 7.0))
                    .desired_width(220.0),
            );
        });
    });
    let needle = state.search.trim().to_lowercase();
    let shown: Vec<&SaveSummary> = campaign
        .saves
        .iter()
        .filter(|s| {
            needle.is_empty()
                || s.name.to_lowercase().contains(&needle)
                || level_name(&s.level).to_lowercase().contains(&needle)
        })
        .collect();
    if shown.is_empty() {
        ui.add_space(20.0);
        ui.label(egui::RichText::new("No saves match the search.").color(theme::MUTED));
        return;
    }

    let available = ui.available_width() - 10.0;
    let per_row = (((available + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).floor() as usize).max(1);
    let rows = shown.len().div_ceil(per_row);
    ui.spacing_mut().item_spacing.y = CARD_GAP;
    egui::ScrollArea::vertical()
        .id_salt("saves")
        .auto_shrink([false, false])
        .show_rows(ui, CARD_HEIGHT, rows, |ui, visible| {
            for row in visible {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for summary in shown.iter().skip(row * per_row).take(per_row) {
                        save_card(ui, summary, thumbs, now, actions);
                    }
                });
            }
            ui.add_space(8.0);
        });
}

fn kind_colors(kind: SaveKind) -> (Color32, Color32) {
    match kind {
        SaveKind::Honour => (theme::ACCENT, Color32::from_black_alpha(215)),
        _ => (theme::TEXT, Color32::from_black_alpha(200)),
    }
}

fn save_card(
    ui: &mut Ui,
    summary: &SaveSummary,
    thumbs: &mut Thumbnails,
    now: u64,
    actions: &mut Vec<Action>,
) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(CARD_WIDTH, CARD_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let editable = summary.support == Support::Editable;
    let hot = response.hovered();
    let rounding = Rounding::same(12.0);
    let line = if hot && editable {
        theme::ACCENT_LINE
    } else {
        theme::LINE
    };
    ui.painter()
        .rect(rect, rounding, theme::PANEL, theme::stroke(1.0, line));

    // The screenshot, or a placeholder while it loads or if there is none.
    let picture = Rect::from_min_size(rect.min, Vec2::new(CARD_WIDTH, PICTURE_HEIGHT)).shrink(1.0);
    let top = Rounding {
        nw: 11.0,
        ne: 11.0,
        sw: 0.0,
        se: 0.0,
    };
    let texture = summary
        .screenshot
        .as_ref()
        .and_then(|path| thumbs.get(path))
        .map(|t| (t.id(), t.size_vec2()));
    match texture {
        Some((id, size)) => {
            // Fill the frame, cropping whichever way overflows.
            let (frame, image) = (picture.aspect_ratio(), size.x / size.y.max(1.0));
            let uv = if image > frame {
                let keep = frame / image;
                Rect::from_min_max(
                    Pos2::new((1.0 - keep) / 2.0, 0.0),
                    Pos2::new((1.0 + keep) / 2.0, 1.0),
                )
            } else {
                let keep = image / frame;
                Rect::from_min_max(
                    Pos2::new(0.0, (1.0 - keep) / 2.0),
                    Pos2::new(1.0, (1.0 + keep) / 2.0),
                )
            };
            let tint = if editable {
                Color32::WHITE
            } else {
                Color32::from_gray(120)
            };
            ui.painter()
                .add(widgets::textured(picture, top, tint, id, uv));
        }
        None => {
            ui.painter().rect_filled(picture, top, theme::SUNKEN);
            ui.painter().text(
                picture.center(),
                Align2::CENTER_CENTER,
                if summary.screenshot.is_some() {
                    "Loading…"
                } else {
                    "No screenshot"
                },
                theme::body(13.0),
                theme::MUTED.gamma_multiply(0.7),
            );
        }
    }
    let (kind_color, kind_fill) = kind_colors(summary.kind);
    widgets::badge_at(
        ui,
        picture.min + Vec2::new(10.0, 10.0),
        &summary.kind.label().to_uppercase(),
        kind_color,
        kind_fill,
    );

    let text_color = if editable { theme::TEXT } else { theme::MUTED };
    let left = rect.left() + 16.0;
    let mut y = picture.bottom() + 20.0;
    let painter = ui
        .painter()
        .with_clip_rect(rect.shrink2(Vec2::new(14.0, 0.0)));
    painter.text(
        Pos2::new(left, y),
        Align2::LEFT_CENTER,
        &summary.name,
        theme::semibold(16.0),
        text_color,
    );
    y += 22.0;
    painter.text(
        Pos2::new(left, y),
        Align2::LEFT_CENTER,
        level_name(&summary.level),
        theme::body(13.5),
        theme::MUTED,
    );
    y += 20.0;
    painter.text(
        Pos2::new(left, y),
        Align2::LEFT_CENTER,
        party_line(summary),
        theme::body(13.5),
        theme::TEXT_SOFT,
    );
    y += 22.0;
    let mods = summary.mods.len();
    if mods > 0 {
        painter.text(
            Pos2::new(left, y),
            Align2::LEFT_CENTER,
            format!("{mods} mod{}", if mods == 1 { "" } else { "s" }),
            theme::body(12.5),
            theme::MUTED,
        );
    }

    let foot = rect.bottom() - 16.0;
    painter.text(
        Pos2::new(left, foot),
        Align2::LEFT_CENTER,
        saves::ago(now.saturating_sub(summary.saved_at)),
        theme::body(12.5),
        theme::MUTED,
    );
    let (status, status_color) = match &summary.support {
        Support::Editable => ("✔ Patch 8".to_owned(), theme::FINE),
        Support::ReadOnly(_) => ("⚠ Can't be edited".to_owned(), theme::CORAL),
    };
    painter.text(
        Pos2::new(rect.right() - 16.0, foot),
        Align2::RIGHT_CENTER,
        status,
        theme::semibold(12.5),
        status_color,
    );

    let response = match &summary.support {
        Support::Editable => response.on_hover_cursor(CursorIcon::PointingHand),
        Support::ReadOnly(why) => response.on_hover_text(why),
    };
    if response.clicked() {
        actions.push(Action::Open(summary.folder.clone()));
    }
}
