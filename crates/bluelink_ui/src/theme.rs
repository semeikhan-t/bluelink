//! ProMotor / shadcn dark: cool near-black canvas, blue primary, medium radius, sans UI type.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, Style,
    TextStyle, Visuals,
};

/// Cool zinc canvas / blue primary — shadcn-like dark tool chrome.
pub const BG: Color32 = Color32::from_rgb(0x09, 0x09, 0x0b);
pub const PANEL: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0e);
pub const FAINT: Color32 = Color32::from_rgb(0x18, 0x18, 0x1b);
pub const STROKE: Color32 = Color32::from_rgb(0x27, 0x27, 0x2a);
pub const TEXT: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfa);
pub const WEAK: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xaa);
pub const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);
pub const DESTRUCTIVE: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
pub const CODE_BG: Color32 = Color32::from_rgb(0x05, 0x05, 0x05);

pub const BODY_SIZE: f32 = 16.0;
pub const EDITOR_SIZE: f32 = 16.0;
pub const HEADING_SIZE: f32 = 24.0;
pub const SIDEBAR_WIDTH: f32 = 300.0;
/// Readable note column — centered in the central panel.
pub const CONTENT_MAX_WIDTH: f32 = 720.0;

pub const ROUND_SM: u8 = 4;
pub const ROUND: u8 = 6;
pub const ROUND_LG: u8 = 8;

pub const FAMILY_DISPLAY: &str = "UI Display";
pub const FAMILY_READING: &str = "UI Reading";

const PHOSPHOR: &str = "phosphor";

pub fn round_sm() -> CornerRadius {
    CornerRadius::same(ROUND_SM)
}
pub fn round() -> CornerRadius {
    CornerRadius::same(ROUND)
}
pub fn round_lg() -> CornerRadius {
    CornerRadius::same(ROUND_LG)
}

pub fn display_family() -> FontFamily {
    FontFamily::Name(FAMILY_DISPLAY.into())
}

pub fn reading_family() -> FontFamily {
    FontFamily::Name(FAMILY_READING.into())
}

/// Sans faces: Display = titles/buttons; Reading = body.
const FACE_BLACK: &str = "bl_black";
const FACE_BOLD: &str = "bl_bold";
const FACE_MEDIUM: &str = "bl_medium";
const FACE_REGULAR: &str = "bl_regular";

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    // Prefer Inter when present; else Noto Sans / Liberation / DejaVu.
    load_face(
        &mut fonts,
        FACE_BLACK,
        &[
            "/usr/share/fonts/inter/Inter-Black.ttf",
            "/usr/share/fonts/TTF/Inter-Black.ttf",
            "/usr/share/fonts/opentype/inter/Inter-Black.otf",
            "/usr/share/fonts/noto/NotoSans-Black.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_BOLD,
        &[
            "/usr/share/fonts/inter/Inter-Bold.ttf",
            "/usr/share/fonts/TTF/Inter-Bold.ttf",
            "/usr/share/fonts/opentype/inter/Inter-Bold.otf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_MEDIUM,
        &[
            "/usr/share/fonts/inter/Inter-SemiBold.ttf",
            "/usr/share/fonts/inter/Inter-Medium.ttf",
            "/usr/share/fonts/TTF/Inter-SemiBold.ttf",
            "/usr/share/fonts/TTF/Inter-Medium.ttf",
            "/usr/share/fonts/noto/NotoSans-Medium.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        ],
    );
    load_face(
        &mut fonts,
        FACE_REGULAR,
        &[
            "/usr/share/fonts/inter/Inter-Regular.ttf",
            "/usr/share/fonts/TTF/Inter-Regular.ttf",
            "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ],
    );

    if let Some(stack) = fonts.families.get_mut(&FontFamily::Proportional) {
        stack.retain(|k| k != PHOSPHOR);
        if fonts.font_data.contains_key(FACE_REGULAR) {
            stack.insert(0, FACE_REGULAR.to_owned());
        }
        stack.insert(1, PHOSPHOR.to_owned());
    }

    register_named(
        &mut fonts,
        FAMILY_DISPLAY,
        &[FACE_BLACK, FACE_BOLD, FACE_MEDIUM, FACE_REGULAR],
    );
    register_named(
        &mut fonts,
        FAMILY_READING,
        &[FACE_REGULAR, FACE_MEDIUM, FACE_BOLD],
    );

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
        &[FACE_MEDIUM, FACE_BOLD, FACE_REGULAR],
    );

    ctx.set_fonts(fonts);
}

pub fn heading_size(level: u8) -> f32 {
    match level {
        1 => 24.0,
        2 => 20.0,
        3 => 18.0,
        _ => 16.0,
    }
}

pub fn heading_family(level: u8) -> FontFamily {
    match level {
        1 => FontFamily::Name("H1".into()),
        2 => FontFamily::Name("H2".into()),
        3 => FontFamily::Name("H3".into()),
        _ => reading_family(),
    }
}

/// Heading style: size + weight. `#` → heaviest, more `#` → lighter.
pub fn heading_rich(text: &str, level: u8) -> RichText {
    RichText::new(text)
        .size(heading_size(level))
        .family(heading_family(level))
        .color(TEXT)
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
    // Icons: phosphor right after the primary face (Button/Body use these families).
    if fonts.font_data.contains_key(PHOSPHOR) {
        let idx = if stack.is_empty() { 0 } else { 1 };
        stack.insert(idx, PHOSPHOR.to_owned());
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
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(14);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size = egui::vec2(44.0, 32.0);
    style.spacing.indent = 22.0;
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 10.0,
        ..egui::style::ScrollStyle::solid()
    };
    style.visuals = promotor_visuals();
    ctx.set_style(style);
    ctx.set_visuals(promotor_visuals());
}

fn scale_text(style: &mut Style) {
    use FontFamily::Monospace;
    let reading = reading_family();
    let display = display_family();
    style.text_styles = [
        (TextStyle::Small, FontId::new(12.0, reading.clone())),
        (TextStyle::Body, FontId::new(BODY_SIZE, reading.clone())),
        (TextStyle::Button, FontId::new(14.0, display.clone())),
        (TextStyle::Heading, FontId::new(HEADING_SIZE, display)),
        (TextStyle::Monospace, FontId::new(EDITOR_SIZE, Monospace)),
    ]
    .into();
}

fn promotor_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = None;

    // Idle: dark fill + light text + subtle border. Hover: muted fill (not invert).
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.bg_fill = FAINT;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x27, 0x27, 0x2a);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.open.bg_fill = FAINT;

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x3f, 0x3f, 0x46));
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, STROKE);

    // Active (pressed) primary: white text on blue.
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 56);
    v.selection.stroke = Stroke::new(2.0_f32, ACCENT);

    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = CODE_BG;
    v.faint_bg_color = FAINT;
    v.code_bg_color = CODE_BG;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = DESTRUCTIVE;

    v.window_stroke = Stroke::new(1.0_f32, STROKE);
    v.window_corner_radius = round_lg();
    v.menu_corner_radius = round();
    v.widgets.noninteractive.corner_radius = round();
    v.widgets.inactive.corner_radius = round();
    v.widgets.hovered.corner_radius = round();
    v.widgets.active.corner_radius = round();
    v.widgets.open.corner_radius = round();
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;

    v
}
