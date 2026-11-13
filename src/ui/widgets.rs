//! The pieces every screen is built from: buttons, cards, badges and the number stepper.

use super::theme;
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Frame, Layout, Margin, Response, Rounding,
    Sense, Stroke, Ui, Vec2,
};
use std::ops::RangeInclusive;

/// How a button looks, which says how much it matters.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one thing the screen is for.
    Primary,
    /// An ordinary action.
    Secondary,
    /// A minor action: text only until hovered.
    Quiet,
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE))
        .size()
        .x
}

/// A button with a text label. Disabled when the surrounding `Ui` is.
pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    let font = theme::semibold(15.0);
    let size = Vec2::new(text_width(ui, text, &font) + 30.0, 36.0);
    sized_button(ui, text, kind, size, font)
}

/// A button that fills the width available.
pub fn wide_button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    let size = Vec2::new(ui.available_width(), 42.0);
    sized_button(ui, text, kind, size, theme::semibold(16.0))
}

fn sized_button(ui: &mut Ui, text: &str, kind: Kind, size: Vec2, font: FontId) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hot = enabled && response.hovered();
        let down = enabled && response.is_pointer_button_down_on();
        let (fill, line, mut color) = match kind {
            Kind::Primary => (
                if down || hot {
                    theme::ACCENT_HOVER
                } else {
                    theme::ACCENT
                },
                Color32::TRANSPARENT,
                theme::ON_ACCENT,
            ),
            Kind::Secondary => (
                if down {
                    theme::WIDGET_PRESS
                } else if hot {
                    theme::WIDGET_HOVER
                } else {
                    theme::WIDGET
                },
                if hot {
                    theme::ACCENT_LINE
                } else {
                    theme::LINE_STRONG
                },
                theme::TEXT,
            ),
            Kind::Quiet => (
                if hot {
                    theme::WIDGET
                } else {
                    Color32::TRANSPARENT
                },
                Color32::TRANSPARENT,
                if hot { theme::TEXT } else { theme::TEXT_SOFT },
            ),
        };
        let (fill, line) = if enabled {
            (fill, line)
        } else {
            match kind {
                Kind::Primary => (theme::WIDGET, Color32::TRANSPARENT),
                _ => (fill.gamma_multiply(0.5), line.gamma_multiply(0.5)),
            }
        };
        if !enabled {
            color = theme::MUTED.gamma_multiply(0.7);
        }
        ui.painter()
            .rect(rect, Rounding::same(8.0), fill, theme::stroke(1.0, line));
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }
    if ui.is_enabled() {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A small square button holding one symbol, with a tooltip saying what it does.
pub fn icon_button(ui: &mut Ui, symbol: &str, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = ui.is_enabled() && response.hovered();
        if hot {
            ui.painter()
                .rect_filled(rect, Rounding::same(6.0), theme::WIDGET_HOVER);
        }
        let color = if !ui.is_enabled() {
            theme::MUTED.gamma_multiply(0.5)
        } else if hot {
            theme::TEXT
        } else {
            theme::MUTED
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            symbol,
            theme::body(17.0),
            color,
        );
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// A rounded panel that groups related things.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::none()
        .fill(theme::PANEL)
        .stroke(theme::stroke(1.0, theme::LINE))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(18.0))
        .show(ui, add)
        .inner
}

/// The small capital heading at the top of a card.
pub fn section_title(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(theme::bold(12.5))
            .color(theme::ACCENT)
            .extra_letter_spacing(1.4),
    );
}

/// A pill of text: a save's kind, a count, a warning.
pub fn badge(ui: &mut Ui, text: &str, color: Color32, fill: Color32) -> Response {
    let font = theme::bold(11.0);
    let size = Vec2::new(text_width(ui, text, &font) + 16.0, 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, Rounding::same(10.0), fill);
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }
    response
}

/// Paints a badge at a fixed place, over a picture.
pub fn badge_at(ui: &Ui, pos: egui::Pos2, text: &str, color: Color32, fill: Color32) {
    let font = theme::bold(11.0);
    let size = Vec2::new(text_width(ui, text, &font) + 16.0, 20.0);
    let rect = egui::Rect::from_min_size(pos, size);
    ui.painter().rect_filled(rect, Rounding::same(6.0), fill);
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, text, font, color);
}

/// A number with a button either side. Returns the new value when it changes.
///
/// The number itself can be dragged or clicked and typed over. `changed` marks a value that
/// differs from the save's.
pub fn stepper(
    ui: &mut Ui,
    value: u32,
    range: RangeInclusive<u32>,
    step: u32,
    width: f32,
    changed: bool,
) -> Option<u32> {
    let mut result = None;
    let outline = if changed {
        theme::CHANGED_LINE
    } else {
        theme::LINE_STRONG
    };
    Frame::none()
        .fill(theme::SUNKEN)
        .stroke(theme::stroke(1.0, outline))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(2.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Always minus, number, plus, whichever way the row around it runs, and no
            // wider than those three.
            let size = Vec2::new(28.0 + 2.0 + width + 2.0 + 28.0, 28.0);
            ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                let step_button = |ui: &mut Ui, symbol: &str, enabled: bool| {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    let hot = enabled && response.hovered();
                    if ui.is_rect_visible(rect) {
                        let fill = if hot {
                            theme::WIDGET_HOVER
                        } else {
                            theme::WIDGET
                        };
                        ui.painter().rect_filled(rect, Rounding::same(6.0), fill);
                        let color = if enabled {
                            theme::TEXT
                        } else {
                            theme::MUTED.gamma_multiply(0.5)
                        };
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            symbol,
                            theme::semibold(17.0),
                            color,
                        );
                    }
                    enabled && response.on_hover_cursor(CursorIcon::PointingHand).clicked()
                };
                if step_button(ui, "−", value > *range.start()) {
                    result = Some(value.saturating_sub(step).max(*range.start()));
                }
                let mut edited = value;
                let color = if changed { theme::CHANGED } else { theme::TEXT };
                ui.scope(|ui| {
                    // The number sits in the frame, with no box of its own.
                    let visuals = ui.visuals_mut();
                    visuals.override_text_color = Some(color);
                    for state in [
                        &mut visuals.widgets.inactive,
                        &mut visuals.widgets.hovered,
                        &mut visuals.widgets.active,
                    ] {
                        state.bg_fill = Color32::TRANSPARENT;
                        state.weak_bg_fill = Color32::TRANSPARENT;
                        state.bg_stroke = Stroke::NONE;
                    }
                    ui.style_mut().override_font_id = Some(theme::semibold(16.0));
                    ui.spacing_mut().button_padding = Vec2::new(2.0, 4.0);
                    ui.add_sized(
                        [width, 28.0],
                        egui::DragValue::new(&mut edited)
                            .range(range.clone())
                            .speed((step as f64 * 0.2).max(0.1)),
                    );
                });
                if edited != value {
                    result = Some(edited);
                }
                if step_button(ui, "+", value < *range.end()) {
                    result = Some(value.saturating_add(step).min(*range.end()));
                }
            });
        });
    result
}

/// Width kept clear on the right of a [`row`] for its control: the widest stepper and its
/// revert button.
const CONTROL_ROOM: f32 = 216.0;

/// A line with a label on the left and whatever `right` adds on the right.
pub fn row(ui: &mut Ui, label: &str, hint: Option<&str>, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        // The words wrap in what the control on the right leaves them.
        let room = (ui.available_width() - CONTROL_ROOM).max(120.0);
        ui.vertical(|ui| {
            ui.set_max_width(room);
            ui.spacing_mut().item_spacing.y = 0.0;
            if hint.is_none() {
                ui.add_space(8.0);
            }
            ui.label(egui::RichText::new(label).font(theme::medium(15.0)));
            if let Some(hint) = hint {
                ui.label(
                    egui::RichText::new(hint)
                        .font(theme::body(12.5))
                        .color(theme::MUTED),
                );
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

/// Text that is clipped with an ellipsis instead of wrapping.
pub fn clipped(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(color)).truncate())
}

/// 1234567 as "1,234,567".
pub fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A rectangle filled with part of a texture, with rounded corners.
pub fn textured(
    rect: egui::Rect,
    rounding: Rounding,
    tint: Color32,
    texture: egui::TextureId,
    uv: egui::Rect,
) -> egui::epaint::RectShape {
    let mut shape = egui::epaint::RectShape::filled(rect, rounding, tint);
    shape.fill_texture_id = texture;
    shape.uv = uv;
    shape
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn groups_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(3_385), "3,385");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
