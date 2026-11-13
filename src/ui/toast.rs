//! Short notices at the bottom of the window that go away on their own.

use super::{theme, widgets, Action};
use eframe::egui::{self, Align2, Context, Frame, Id, Margin, Order, Rounding};

/// How long a notice stays, in seconds.
const LIFETIME: f64 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Problem,
}

pub struct Toast {
    pub text: String,
    pub tone: Tone,
    /// A button on the notice: its label and what it does.
    pub action: Option<(String, Action)>,
    expires: Option<f64>,
}

impl Toast {
    pub fn new(text: impl Into<String>, tone: Tone) -> Toast {
        Toast {
            text: text.into(),
            tone,
            action: None,
            expires: None,
        }
    }

    pub fn with_action(mut self, label: &str, action: Action) -> Toast {
        self.action = Some((label.to_owned(), action));
        self
    }
}

#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
}

impl Toasts {
    pub fn push(&mut self, toast: Toast) {
        // Newest replaces oldest; more than three at once is noise.
        if self.list.len() >= 3 {
            self.list.remove(0);
        }
        self.list.push(toast);
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The text of each notice showing, oldest first.
    pub fn texts(&self) -> Vec<&str> {
        self.list.iter().map(|t| t.text.as_str()).collect()
    }

    pub fn show(&mut self, ctx: &Context, actions: &mut Vec<Action>) {
        let now = ctx.input(|i| i.time);
        for toast in &mut self.list {
            toast.expires.get_or_insert(now + LIFETIME);
        }
        self.list.retain(|t| t.expires.is_none_or(|at| at > now));
        if self.list.is_empty() {
            return;
        }
        // Come back when the next one is due to go.
        if let Some(next) = self.list.iter().filter_map(|t| t.expires).reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.05)));
        }
        let mut dismissed = None;
        egui::Area::new(Id::new("toasts"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -20.0])
            .show(ctx, |ui| {
                for (i, toast) in self.list.iter().enumerate() {
                    let (line, mark, mark_color) = match toast.tone {
                        Tone::Info => (theme::LINE_STRONG, "•", theme::ACCENT),
                        Tone::Success => (theme::LINE_STRONG, "✔", theme::FINE),
                        Tone::Problem => (theme::CORAL_LINE, "⚠", theme::CORAL),
                    };
                    Frame::none()
                        .fill(theme::DIALOG)
                        .stroke(theme::stroke(1.0, line))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(16.0, 10.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(mark).color(mark_color));
                                ui.label(
                                    egui::RichText::new(&toast.text).font(theme::medium(15.0)),
                                );
                                if let Some((label, action)) = &toast.action {
                                    if widgets::button(ui, label, widgets::Kind::Quiet).clicked() {
                                        actions.push(action.clone());
                                        dismissed = Some(i);
                                    }
                                }
                                if widgets::icon_button(ui, "✖", "Dismiss").clicked() {
                                    dismissed = Some(i);
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismissed {
            self.list.remove(i);
        }
    }
}
