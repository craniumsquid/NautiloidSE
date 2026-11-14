//! Colours, fonts and the base look of every widget.

use eframe::egui::{
    self, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke,
    TextStyle, Visuals,
};

// The palette is Faerûn by candlelight: dark leather and old wood for surfaces, parchment
// for text, and gilt for anything that invites a click, as on the game's own menus.
// Violet, the colour of the Weave and of the nautiloid, is kept for one thing only: marking
// what has been changed.

// Surfaces, darkest first.
pub const SUNKEN: Color32 = Color32::from_rgb(11, 9, 7);
pub const BG: Color32 = Color32::from_rgb(18, 15, 12);
pub const BG_HEADER: Color32 = Color32::from_rgb(23, 19, 14);
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(21, 17, 13);
pub const PANEL: Color32 = Color32::from_rgb(30, 25, 19);
pub const PANEL_ALT: Color32 = Color32::from_rgb(25, 21, 16);
pub const DIALOG: Color32 = Color32::from_rgb(34, 28, 21);
pub const WIDGET: Color32 = Color32::from_rgb(47, 39, 29);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(61, 51, 38);
pub const WIDGET_PRESS: Color32 = Color32::from_rgb(75, 63, 46);
pub const LINE: Color32 = Color32::from_rgb(52, 43, 31);
pub const LINE_STRONG: Color32 = Color32::from_rgb(76, 63, 45);

// Parchment.
pub const TEXT: Color32 = Color32::from_rgb(240, 231, 212);
pub const TEXT_SOFT: Color32 = Color32::from_rgb(215, 203, 180);
pub const MUTED: Color32 = Color32::from_rgb(164, 151, 124);

/// Gilt: headings, the selected tab, primary buttons.
pub const ACCENT: Color32 = Color32::from_rgb(218, 182, 108);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(237, 207, 140);
pub const ACCENT_BG: Color32 = Color32::from_rgb(58, 46, 22);
pub const ACCENT_LINE: Color32 = Color32::from_rgb(125, 101, 53);
/// Anything changed and not yet exported.
pub const CHANGED: Color32 = Color32::from_rgb(190, 168, 255);
pub const CHANGED_BG: Color32 = Color32::from_rgb(40, 33, 62);
pub const CHANGED_LINE: Color32 = Color32::from_rgb(104, 87, 160);
/// Warnings and things that cannot be done: sealing-wax red.
pub const CORAL: Color32 = Color32::from_rgb(238, 132, 110);
pub const CORAL_BG: Color32 = Color32::from_rgb(58, 27, 21);
pub const CORAL_LINE: Color32 = Color32::from_rgb(124, 61, 49);
/// Things that are fine.
pub const FINE: Color32 = Color32::from_rgb(146, 204, 150);
pub const FINE_BG: Color32 = Color32::from_rgb(28, 44, 30);
/// Text on a filled accent button.
pub const ON_ACCENT: Color32 = Color32::from_rgb(31, 22, 8);

/// A line of the given width. `Stroke::new` takes `impl Into<f32>`, and a bare float literal
/// there relies on a type fallback the compiler is phasing out.
pub fn stroke(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

pub const HEADER_HEIGHT: f32 = 64.0;
pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const CHANGES_WIDTH: f32 = 316.0;

const MEDIUM: &str = "barlow-medium";
const SEMIBOLD: &str = "barlow-semibold";
const BOLD: &str = "barlow-bold";
const DISPLAY: &str = "cinzel-bold";

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}
/// The titling face, for the logo and page headings.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn install_fonts(ctx: &Context) {
    // egui's own fonts stay as fallbacks: they supply the symbols and any script Barlow lacks.
    let mut fonts = FontDefinitions::default();
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let faces: [(&str, &'static [u8]); 5] = [
        (
            "barlow",
            include_bytes!("../../assets/fonts/Barlow-Regular.ttf"),
        ),
        (
            MEDIUM,
            include_bytes!("../../assets/fonts/Barlow-Medium.ttf"),
        ),
        (
            SEMIBOLD,
            include_bytes!("../../assets/fonts/Barlow-SemiBold.ttf"),
        ),
        (BOLD, include_bytes!("../../assets/fonts/Barlow-Bold.ttf")),
        (
            DISPLAY,
            include_bytes!("../../assets/fonts/Cinzel-Bold.ttf"),
        ),
    ];
    for (name, bytes) in faces {
        fonts
            .font_data
            .insert(name.to_owned(), FontData::from_static(bytes));
        if name != "barlow" {
            let mut chain = vec![name.to_owned()];
            chain.extend(fallbacks.iter().cloned());
            fonts.families.insert(FontFamily::Name(name.into()), chain);
        }
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "barlow".to_owned());
    ctx.set_fonts(fonts);
}

/// Applies the theme to `ctx`. Called once, when the app starts.
pub fn apply(ctx: &Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Small, body(12.5)),
        (TextStyle::Body, body(15.0)),
        (TextStyle::Button, medium(15.0)),
        (TextStyle::Heading, display(26.0)),
        (TextStyle::Monospace, mono(13.5)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = false;
    // The handle in the widget colours, not the text colour.
    style.spacing.scroll.foreground_color = false;

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG;
    visuals.window_fill = DIALOG;
    visuals.extreme_bg_color = SUNKEN;
    visuals.faint_bg_color = PANEL_ALT;
    visuals.window_rounding = Rounding::same(12.0);
    visuals.window_stroke = stroke(1.0, LINE_STRONG);
    visuals.selection.bg_fill = ACCENT_BG;
    visuals.selection.stroke = stroke(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    let rounding = Rounding::same(8.0);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = stroke(1.0, LINE);
    widgets.noninteractive.fg_stroke = stroke(1.0, TEXT_SOFT);
    widgets.noninteractive.rounding = rounding;
    for (state, fill, line) in [
        (&mut widgets.inactive, WIDGET, LINE_STRONG),
        (&mut widgets.hovered, WIDGET_HOVER, ACCENT_LINE),
        (&mut widgets.active, WIDGET_PRESS, ACCENT),
        (&mut widgets.open, WIDGET_HOVER, ACCENT_LINE),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke(1.0, line);
        state.fg_stroke = stroke(1.0, TEXT);
        state.rounding = rounding;
        state.expansion = 0.0;
    }
    style.visuals = visuals;
    ctx.set_style(style);
}
