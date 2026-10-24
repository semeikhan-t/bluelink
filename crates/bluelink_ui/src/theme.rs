//! BlueLink visual scale: larger type, roomier spacing, ink + signal-blue palette.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, Style,
    TextStyle, Visuals,
};

/// Deep ink / signal blue — readable dark workspace, not purple-glow defaults.
pub const BG: Color32 = Color32::from_rgb(14, 18, 28);
pub const PANEL: Color32 = Color32::from_rgb(22, 28, 42);
pub const FAINT: Color32 = Color32::from_rgb(30, 38, 56);
pub const STROKE: Color32 = Color32::from_rgb(48, 58, 78);
pub const TEXT: Color32 = Color32::from_rgb(232, 236, 244);
pub const WEAK: Color32 = Color32::from_rgb(148, 160, 184);
pub const ACCENT: Color32 = Color32::from_rgb(56, 140, 220);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(40, 96, 160);
pub const WARN: Color32 = Color32::from_rgb(220, 160, 72);
pub const CODE_BG: Color32 = Color32::from_rgb(18, 24, 36);

pub const BODY_SIZE: f32 = 16.0;
pub const EDITOR_SIZE: f32 = 17.0;
pub const HEADING_SIZE: f32 = 28.0;
pub const SIDEBAR_WIDTH: f32 = 300.0;
/// Readable note column — centered in the central panel.
pub const CONTENT_MAX_WIDTH: f32 = 720.0;

/// Named families: fewer `#` → heavier weight (`H1` black … `H4+` regular).
const FACE_BLACK: &str = "bl_black";
const FACE_BOLD: &str = "bl_bold";
const FACE_MEDIUM: &str = "bl_medium";
const FACE_REGULAR: &str = "bl_regular";

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    load_face(
        &mut fonts,
        FACE_BLACK,
        &[
            "/usr/share/fonts/noto/NotoSans-Black.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_BOLD,
        &[
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_MEDIUM,
        &[
            "/usr/share/fonts/noto/NotoSans-Medium.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_REGULAR,
        &[
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
        ],
    );

    // Prefer real Regular for body text when available.
    if fonts.font_data.contains_key(FACE_REGULAR) {
        if let Some(stack) = fonts.families.get_mut(&FontFamily::Proportional) {
            stack.insert(0, FACE_REGULAR.to_owned());
        }
    }

    // Always register heading families (fallback to Ubuntu-Light / Regular).
    register_named(
        &mut fonts,
        "H1",
        &[FACE_BLACK, FACE_BOLD, FACE_MEDIUM, FACE_REGULAR],
    );
    register_named(
        &mut fonts,
        "H2",
        &[FACE_BOLD, FACE_BLACK, FACE_MEDIUM, FACE_REGULAR],
    );
    register_named(
        &mut fonts,
        "H3",
        &[FACE_MEDIUM, FACE_REGULAR, FACE_BOLD],
    );

    ctx.set_fonts(fonts);
}

pub fn heading_size(level: u8) -> f32 {
    match level {
        1 => 28.0,
        2 => 24.0,
        3 => 20.0,
        _ => 18.0,
    }
}

pub fn heading_family(level: u8) -> FontFamily {
    match level {
        1 => FontFamily::Name("H1".into()),
        2 => FontFamily::Name("H2".into()),
        3 => FontFamily::Name("H3".into()),
        _ => FontFamily::Proportional,
    }
}

/// Heading style: size + weight. `#` → heaviest, more `#` → lighter.
pub fn heading_rich(text: &str, level: u8) -> RichText {
    RichText::new(text)
        .size(heading_size(level))
        .family(heading_family(level))
}

fn load_face(fonts: &mut FontDefinitions, key: &str, paths: &[&str]) {
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
            return;
        }
    }
}

fn register_named(fonts: &mut FontDefinitions, name: &str, preferred: &[&str]) {
    let mut stack = Vec::new();
    for key in preferred {
        if fonts.font_data.contains_key(*key) {
            stack.push((*key).to_owned());
        }
    }
    stack.push("Ubuntu-Light".to_owned());
    stack.push("NotoEmoji-Regular".to_owned());
    stack.push("emoji-icon-font".to_owned());
    fonts
        .families
        .insert(FontFamily::Name(name.into()), stack);
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    scale_text(&mut style);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(14);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(48.0, 32.0);
    style.spacing.indent = 22.0;
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 10.0,
        ..egui::style::ScrollStyle::solid()
    };
    style.visuals = dark_visuals();
    ctx.set_style(style);
    ctx.set_visuals(dark_visuals());
}

fn scale_text(style: &mut Style) {
    use FontFamily::{Monospace, Proportional};
    style.text_styles = [
        (TextStyle::Small, FontId::new(13.0, Proportional)),
        (TextStyle::Body, FontId::new(BODY_SIZE, Proportional)),
        (TextStyle::Button, FontId::new(15.0, Proportional)),
        (TextStyle::Heading, FontId::new(HEADING_SIZE, Proportional)),
        (TextStyle::Monospace, FontId::new(EDITOR_SIZE, Monospace)),
    ]
    .into();
}

fn dark_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.bg_fill = FAINT;
    v.widgets.hovered.bg_fill = Color32::from_rgb(36, 48, 72);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.open.bg_fill = FAINT;

    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.bg_stroke = Stroke::new(1.5_f32, ACCENT);

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(56, 140, 220, 80);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = FAINT;
    v.code_bg_color = CODE_BG;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    let _ = WEAK; // used via RichText::weak() / style elsewhere

    v.window_stroke = Stroke::new(1.0_f32, STROKE);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    v.widgets.inactive.corner_radius = CornerRadius::same(6);
    v.widgets.hovered.corner_radius = CornerRadius::same(6);
    v.widgets.active.corner_radius = CornerRadius::same(6);

    v
}
