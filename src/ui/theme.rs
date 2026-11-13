//! Colours, fonts and the base look of every widget.

use eframe::egui::{
    self, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke,
    TextStyle, Visuals,
};

// Surfaces, darkest first.
pub const SUNKEN: Color32 = Color32::from_rgb(10, 8, 15);
pub const BG: Color32 = Color32::from_rgb(15, 12, 22);
pub const BG_HEADER: Color32 = Color32::from_rgb(19, 15, 28);
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(17, 14, 25);
pub const PANEL: Color32 = Color32::from_rgb(25, 21, 37);
pub const PANEL_ALT: Color32 = Color32::from_rgb(21, 17, 31);
pub const DIALOG: Color32 = Color32::from_rgb(27, 22, 40);
pub const WIDGET: Color32 = Color32::from_rgb(38, 32, 56);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(50, 42, 73);
pub const WIDGET_PRESS: Color32 = Color32::from_rgb(62, 52, 90);
pub const LINE: Color32 = Color32::from_rgb(43, 36, 62);
pub const LINE_STRONG: Color32 = Color32::from_rgb(60, 51, 86);

pub const TEXT: Color32 = Color32::from_rgb(236, 231, 245);
pub const TEXT_SOFT: Color32 = Color32::from_rgb(205, 198, 220);
pub const MUTED: Color32 = Color32::from_rgb(150, 141, 172);

/// The accent: links, the selected tab, primary buttons.
pub const VIOLET: Color32 = Color32::from_rgb(176, 140, 255);
pub const VIOLET_HOVER: Color32 = Color32::from_rgb(198, 170, 255);
pub const VIOLET_BG: Color32 = Color32::from_rgb(46, 34, 79);
pub const VIOLET_LINE: Color32 = Color32::from_rgb(92, 70, 150);
/// Anything changed and not yet exported.
pub const GOLD: Color32 = Color32::from_rgb(232, 196, 108);
pub const GOLD_BG: Color32 = Color32::from_rgb(44, 37, 24);
pub const GOLD_LINE: Color32 = Color32::from_rgb(112, 93, 49);
/// Warnings and things that cannot be done.
pub const CORAL: Color32 = Color32::from_rgb(255, 150, 128);
pub const CORAL_BG: Color32 = Color32::from_rgb(52, 26, 26);
pub const CORAL_LINE: Color32 = Color32::from_rgb(120, 62, 56);
/// Things that are fine.
pub const TEAL: Color32 = Color32::from_rgb(110, 226, 196);
/// Text on a filled accent button.
pub const ON_ACCENT: Color32 = Color32::from_rgb(22, 14, 40);

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
    visuals.selection.bg_fill = VIOLET_BG;
    visuals.selection.stroke = stroke(1.0, VIOLET);
    visuals.hyperlink_color = VIOLET;
    let rounding = Rounding::same(8.0);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = stroke(1.0, LINE);
    widgets.noninteractive.fg_stroke = stroke(1.0, TEXT_SOFT);
    widgets.noninteractive.rounding = rounding;
    for (state, fill, line) in [
        (&mut widgets.inactive, WIDGET, LINE_STRONG),
        (&mut widgets.hovered, WIDGET_HOVER, VIOLET_LINE),
        (&mut widgets.active, WIDGET_PRESS, VIOLET),
        (&mut widgets.open, WIDGET_HOVER, VIOLET_LINE),
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
