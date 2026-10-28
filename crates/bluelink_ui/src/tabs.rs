use egui::{Color32, CursorIcon, FontId, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular;

use crate::theme;

#[derive(Default)]
pub struct TabAction {
    pub select: Option<usize>,
    pub close: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Note,
    Graph,
    Devices,
    Sync,
}

pub struct TabView {
    pub title: String,
    pub kind: TabKind,
}

const TAB_H: f32 = 35.0;
const BAR_PAD_Y: f32 = 5.0;
/// Full tab strip height — use to vertically center toolbar icons.
pub const BAR_HEIGHT: f32 = 40.0;
const TITLE_MAX: usize = 28;
const TITLE_SIZE: f32 = 14.0;
const ICON_SIZE: f32 = 14.0;

/// Obsidian-like tabs: quiet inactive labels and a raised active leaf.
pub fn show(ui: &mut Ui, tabs: &[TabView], active: Option<usize>, dirty: bool) -> TabAction {
    let mut action = TabAction::default();
    if tabs.is_empty() {
        return action;
    }

    let bar_w = ui.available_width();
    let (_, bar_rect) = ui.allocate_space(egui::vec2(bar_w, BAR_HEIGHT));

    ui.painter()
        .rect_filled(bar_rect, egui::CornerRadius::ZERO, theme::PANEL);
    ui.painter().hline(
        bar_rect.x_range(),
        bar_rect.bottom() - 0.5,
        Stroke::new(1.0_f32, theme::STROKE),
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                bar_rect.min + egui::vec2(8.0, BAR_PAD_Y),
                bar_rect.max - egui::vec2(8.0, 0.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );

    egui::ScrollArea::horizontal()
        .id_salt("note_tabs")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, true])
        .show(&mut child, |ui| {
            ui.set_min_height(TAB_H);
            ui.spacing_mut().item_spacing.x = 1.0;

            for (index, tab) in tabs.iter().enumerate() {
                let is_active = active == Some(index);
                let title = truncate_title(&tab.title);
                let show_dirty = dirty && is_active && tab.kind == TabKind::Note;
                tab_chip(
                    ui,
                    index,
                    tab.kind,
                    &title,
                    is_active,
                    show_dirty,
                    &mut action,
                );
            }
        });

    action
}

fn tab_chip(
    ui: &mut Ui,
    index: usize,
    kind: TabKind,
    title: &str,
    active: bool,
    dirty: bool,
    action: &mut TabAction,
) {
    let close_id = ui.id().with(("note_tab_close", index));

    let pad_x = 8.0_f32;
    let icon_w = if kind == TabKind::Note { 0.0 } else { 16.0 };
    let close_w = 18.0_f32;
    let gap_icon = if kind == TabKind::Note { 0.0 } else { 6.0 };
    let gap_close = 4.0_f32;
    let dirty_w = if dirty { 10.0 } else { 0.0 };
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(
            title.to_owned(),
            FontId::proportional(TITLE_SIZE),
            theme::TEXT,
        )
        .size()
        .x
    });
    let width = pad_x + icon_w + gap_icon + dirty_w + text_w + gap_close + close_w + pad_x;
    let size = Vec2::new(width.clamp(96.0, 320.0), TAB_H);

    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Tab {title}"))
    });
    let hovered = response.hovered();

    let fill = if active {
        theme::BG
    } else if hovered {
        Color32::from_rgba_unmultiplied(255, 255, 255, 17)
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if active {
        Stroke::new(1.0_f32, theme::STROKE)
    } else if hovered {
        Stroke::new(1.0_f32, theme::STROKE_HOVER)
    } else {
        Stroke::NONE
    };

    let radius = if active {
        egui::CornerRadius {
            nw: 10,
            ne: 10,
            sw: 0,
            se: 0,
        }
    } else {
        theme::round_sm()
    };
    ui.painter()
        .rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            radius,
            Stroke::new(2.0_f32, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }

    let text_color = if active || hovered {
        theme::TEXT
    } else {
        theme::WEAK
    };
    let icon_color = if active || hovered {
        theme::TEXT
    } else {
        theme::WEAK
    };

    let mut x = rect.left() + pad_x;
    let cy = rect.center().y;

    let icon = match kind {
        TabKind::Note => "",
        TabKind::Graph => regular::SHARE_NETWORK,
        TabKind::Devices => regular::DEVICES,
        TabKind::Sync => regular::ARROWS_CLOCKWISE,
    };
    if !icon.is_empty() {
        paint_glyph(
            ui,
            egui::pos2(x, cy),
            icon,
            ICON_SIZE,
            icon_color,
            AlignX::Left,
        );
        x += icon_w + gap_icon;
    }

    if dirty {
        ui.painter()
            .circle_filled(egui::pos2(x + 2.5, cy), 2.5, theme::WARN);
        x += dirty_w;
    }

    paint_glyph(
        ui,
        egui::pos2(x, cy),
        title,
        TITLE_SIZE,
        text_color,
        AlignX::Left,
    );
    x += text_w + gap_close;

    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - pad_x - close_w * 0.5, cy),
        Vec2::splat(close_w),
    );
    let show_close = active || hovered;
    let mut closed = false;
    if show_close {
        let close = ui.interact(close_rect, close_id, Sense::click());
        close.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}"))
        });
        if close.has_focus() {
            ui.painter().rect_stroke(
                close_rect.expand(1.0),
                theme::round_sm(),
                Stroke::new(2.0_f32, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let ch = close.hovered();
        if ch {
            ui.painter().rect_filled(
                close_rect.expand(1.0),
                theme::round_sm(),
                Color32::from_rgba_unmultiplied(
                    theme::DESTRUCTIVE.r(),
                    theme::DESTRUCTIVE.g(),
                    theme::DESTRUCTIVE.b(),
                    48,
                ),
            );
        }
        let xc = if ch { theme::DESTRUCTIVE } else { theme::WEAK };
        paint_glyph(
            ui,
            close_rect.center(),
            regular::X,
            12.0,
            xc,
            AlignX::Center,
        );
        close.clone().on_hover_text("Close");
        if close.clicked() {
            action.close = Some(index);
            closed = true;
        }
    }

    if !closed {
        if response.clicked() {
            action.select = Some(index);
        }
        if response.middle_clicked() {
            action.close = Some(index);
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

fn truncate_title(raw: &str) -> String {
    if raw.chars().count() <= TITLE_MAX {
        return raw.to_string();
    }
    let truncated: String = raw.chars().take(TITLE_MAX.saturating_sub(1)).collect();
    format!("{truncated}…")
}
