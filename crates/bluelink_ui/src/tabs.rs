use egui::{Color32, CornerRadius, CursorIcon, FontId, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular;

use crate::theme;

#[derive(Default)]
pub struct TabAction {
    pub select: Option<String>,
    pub close: Option<String>,
}

const TAB_H: f32 = 32.0;
const TITLE_MAX: usize = 24;

/// Browser-style tab strip for open notes.
pub fn show(ui: &mut Ui, tabs: &[String], active: Option<&str>, dirty: bool) -> TabAction {
    let mut action = TabAction::default();
    if tabs.is_empty() {
        return action;
    }

    let bar_w = ui.available_width();
    let (_, bar_rect) = ui.allocate_space(egui::vec2(bar_w, TAB_H + 4.0));
    let rule_y = bar_rect.bottom() - 1.0;

    // Slightly deeper than the editor so the active tab can "lift" into the content.
    let strip = Color32::from_rgb(10, 13, 22);
    ui.painter()
        .rect_filled(bar_rect, CornerRadius::ZERO, strip);
    ui.painter().hline(
        bar_rect.x_range(),
        rule_y,
        Stroke::new(1.0_f32, theme::STROKE),
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar_rect.shrink2(egui::vec2(4.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::BOTTOM)),
    );

    egui::ScrollArea::horizontal()
        .id_salt("note_tabs")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show(&mut child, |ui| {
            ui.set_min_height(TAB_H + 2.0);
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.add_space(2.0);

            for path in tabs {
                let is_active = active == Some(path.as_str());
                let title = tab_title(path);
                tab_chip(
                    ui,
                    path,
                    &title,
                    is_active,
                    dirty && is_active,
                    rule_y,
                    &mut action,
                );
            }
            ui.add_space(4.0);
        });

    action
}

fn tab_chip(
    ui: &mut Ui,
    path: &str,
    title: &str,
    active: bool,
    dirty: bool,
    rule_y: f32,
    action: &mut TabAction,
) {
    let close_id = ui.id().with(("note_tab_close", path));

    let pad_x = 10.0_f32;
    let icon_w = 14.0_f32;
    let close_w = 16.0_f32;
    let gaps = 6.0_f32 + if dirty { 10.0 } else { 0.0 } + 6.0;
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(title.to_owned(), FontId::proportional(13.5), Color32::WHITE)
            .size()
            .x
    });
    let width = pad_x + icon_w + gaps + text_w + close_w + pad_x;
    let size = Vec2::new(width.max(72.0), TAB_H);

    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();

    let rounding = CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };

    if active {
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rule_y + 1.0),
        );
        ui.painter().rect_filled(body, rounding, theme::BG);
        ui.painter().hline(
            body.x_range().shrink(10.0),
            body.top() + 2.0,
            Stroke::new(2.0_f32, theme::ACCENT),
        );
        // Cover the strip rule so the active tab merges into the editor.
        ui.painter().hline(
            body.x_range(),
            rule_y,
            Stroke::new(2.5_f32, theme::BG),
        );
    } else if hovered {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(0.0, 4.0)),
            CornerRadius::same(6),
            Color32::from_rgb(24, 32, 48),
        );
    }

    let text_color = if active || hovered {
        theme::TEXT
    } else {
        theme::WEAK
    };
    let icon_color = if active {
        theme::ACCENT
    } else if hovered {
        theme::TEXT
    } else {
        theme::WEAK
    };

    let mut x = rect.left() + pad_x;
    let cy = rect.center().y;

    paint_glyph(ui, egui::pos2(x, cy), regular::NOTE, 13.0, icon_color, AlignX::Left);
    x += icon_w + 6.0;

    if dirty {
        ui.painter()
            .circle_filled(egui::pos2(x + 3.0, cy), 3.0, theme::WARN);
        x += 10.0;
    }

    paint_glyph(ui, egui::pos2(x, cy), title, 13.5, text_color, AlignX::Left);
    x += text_w + 6.0;

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - pad_x - close_w * 0.5, cy),
        Vec2::splat(close_w),
    );
    let show_close = active || hovered;
    let mut closed = false;
    if show_close {
        let close = ui.interact(close_rect, close_id, Sense::click());
        let ch = close.hovered();
        if ch {
            ui.painter().rect_filled(
                close_rect.expand(1.5),
                CornerRadius::same(4),
                Color32::from_rgba_unmultiplied(220, 90, 90, 55),
            );
        }
        let xc = if ch {
            Color32::from_rgb(255, 140, 140)
        } else {
            theme::WEAK
        };
        paint_glyph(ui, close_rect.center(), regular::X, 11.0, xc, AlignX::Center);
        close.clone().on_hover_text("Close");
        if close.clicked() {
            action.close = Some(path.to_string());
            closed = true;
        }
    }

    if !closed {
        if response.clicked() {
            action.select = Some(path.to_string());
        }
        if response.middle_clicked() {
            action.close = Some(path.to_string());
        }
    }

    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    let _ = x;
}

enum AlignX {
    Left,
    Center,
}

fn paint_glyph(ui: &Ui, anchor: egui::Pos2, text: &str, size: f32, color: Color32, align: AlignX) {
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), FontId::proportional(size), color));
    let pos = match align {
        AlignX::Left => egui::pos2(anchor.x, anchor.y - galley.size().y * 0.5),
        AlignX::Center => egui::pos2(
            anchor.x - galley.size().x * 0.5,
            anchor.y - galley.size().y * 0.5,
        ),
    };
    ui.painter().galley(pos, galley, color);
}

fn tab_title(path: &str) -> String {
    let raw = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md");
    if raw.chars().count() <= TITLE_MAX {
        return raw.to_string();
    }
    let truncated: String = raw.chars().take(TITLE_MAX.saturating_sub(1)).collect();
    format!("{truncated}…")
}
