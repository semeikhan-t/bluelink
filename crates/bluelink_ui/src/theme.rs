//! Obsidian 1.14.5 dark tokens, adapted to native egui widgets.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, Style,
    TextStyle, Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1c);
pub const BG_ALT: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);
pub const PANEL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
pub const FAINT: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);
pub const STROKE: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const STROKE_HOVER: Color32 = Color32::from_rgb(0x3f, 0x3f, 0x3f);
pub const TEXT: Color32 = Color32::from_rgb(0xda, 0xda, 0xda);
pub const WEAK: Color32 = Color32::from_rgb(0xb3, 0xb3, 0xb3);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x66, 0x66, 0x66);
/// Obsidian default `hsl(258 88% 66%)`, rounded to sRGB (`#8a5cf5`).
pub const ACCENT: Color32 = Color32::from_rgb(0x8a, 0x5c, 0xf5);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xa8, 0x82, 0xff);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x6c, 0x5c, 0xc5);
pub const DESTRUCTIVE: Color32 = Color32::from_rgb(0xfb, 0x46, 0x4c);
pub const WARN: Color32 = Color32::from_rgb(0xe9, 0x97, 0x3f);
pub const SUCCESS: Color32 = Color32::from_rgb(0x44, 0xcf, 0x6e);
pub const CODE_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);

pub const BODY_SIZE: f32 = 13.0;
pub const EDITOR_SIZE: f32 = 16.0;
pub const HEADING_SIZE: f32 = 24.0;
pub const TITLEBAR_HEIGHT: f32 = 30.0;
pub const HEADER_HEIGHT: f32 = 40.0;
pub const RIBBON_WIDTH: f32 = 44.0;
pub const STATUS_HEIGHT: f32 = 24.0;
pub const SIDEBAR_WIDTH: f32 = 280.0;
/// Readable note column — centered in the central panel.
pub const CONTENT_MAX_WIDTH: f32 = 700.0;

pub const ROUND_SM: u8 = 4;
pub const ROUND: u8 = 4;
pub const ROUND_LG: u8 = 8;
pub const ROUND_XL: u8 = 12;
pub const ROUND_PILL: u8 = 24;
pub const SPACING_SCALE: [f32; 7] = [2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0];

pub const FAMILY_DISPLAY: &str = "UI Display";
pub const FAMILY_READING: &str = "UI Reading";

const PHOSPHOR: &str = "phosphor";

pub fn round_sm() -> CornerRadius {
    CornerRadius::same(ROUND_SM)
}
pub fn round() -> CornerRadius {
    CornerRadius::same(ROUND)
}
pub fn round_xl() -> CornerRadius {
    CornerRadius::same(ROUND_XL)
}

pub fn display_family() -> FontFamily {
    FontFamily::Name(FAMILY_DISPLAY.into())
}

pub fn reading_family() -> FontFamily {
    FontFamily::Name(FAMILY_READING.into())
}

/// Give glyph-only controls a screen-reader label and matching tooltip.
pub fn accessible_icon(response: egui::Response, name: &str) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    response.on_hover_text(name)
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
    register_named(&mut fonts, "H3", &[FACE_MEDIUM, FACE_BOLD, FACE_REGULAR]);

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
    fonts.families.insert(FontFamily::Name(name.into()), stack);
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    scale_text(&mut style);
    style.spacing.item_spacing = egui::vec2(SPACING_SCALE[2], SPACING_SCALE[1]);
    style.spacing.window_margin = egui::Margin::same(SPACING_SCALE[3] as i8);
    style.spacing.button_padding = egui::vec2(SPACING_SCALE[2], SPACING_SCALE[1]);
    style.spacing.interact_size = egui::vec2(30.0, 30.0);
    style.spacing.indent = 20.0;
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
        (TextStyle::Button, FontId::new(13.0, display.clone())),
        (TextStyle::Heading, FontId::new(HEADING_SIZE, display)),
        (TextStyle::Monospace, FontId::new(EDITOR_SIZE, Monospace)),
    ]
    .into();
}

fn promotor_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = None;

    // Obsidian uses quiet controls that emerge on hover instead of persistent cards.
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.bg_fill = FAINT;
    v.widgets.hovered.bg_fill = STROKE;
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.open.bg_fill = FAINT;

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, STROKE_HOVER);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, STROKE);

    // Active (pressed) primary: white text on blue.
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0x8a, 0x5c, 0xf5, 84);
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
    v.window_corner_radius = round_xl();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_dark_semantic_tokens_are_exact() {
        assert_eq!(BG, Color32::from_rgb(0x1c, 0x1c, 0x1c));
        assert_eq!(BG_ALT, Color32::from_rgb(0x23, 0x23, 0x23));
        assert_eq!(PANEL, Color32::from_rgb(0x28, 0x28, 0x28));
        assert_eq!(STROKE, Color32::from_rgb(0x33, 0x33, 0x33));
        assert_eq!(STROKE_HOVER, Color32::from_rgb(0x3f, 0x3f, 0x3f));
        assert_eq!(TEXT, Color32::from_rgb(0xda, 0xda, 0xda));
        assert_eq!(WEAK, Color32::from_rgb(0xb3, 0xb3, 0xb3));
        assert_eq!(TEXT_FAINT, Color32::from_rgb(0x66, 0x66, 0x66));
        assert_eq!(ACCENT, Color32::from_rgb(0x8a, 0x5c, 0xf5));
        assert_eq!(DESTRUCTIVE, Color32::from_rgb(0xfb, 0x46, 0x4c));
        assert_eq!(WARN, Color32::from_rgb(0xe9, 0x97, 0x3f));
        assert_eq!(SUCCESS, Color32::from_rgb(0x44, 0xcf, 0x6e));
    }

    #[test]
    fn obsidian_workspace_geometry_is_exact() {
        assert_eq!(TITLEBAR_HEIGHT, 30.0);
        assert_eq!(HEADER_HEIGHT, 40.0);
        assert_eq!(RIBBON_WIDTH, 44.0);
        assert_eq!(STATUS_HEIGHT, 24.0);
        assert_eq!(SIDEBAR_WIDTH, 280.0);
        assert_eq!(CONTENT_MAX_WIDTH, 700.0);
        assert_eq!([ROUND_SM, ROUND_LG, ROUND_XL, ROUND_PILL], [4, 8, 12, 24]);
        assert_eq!(SPACING_SCALE, [2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0]);
    }
}
