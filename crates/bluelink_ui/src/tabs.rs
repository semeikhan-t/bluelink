use egui::{RichText, Sense, Ui};
use egui_phosphor::regular;

use crate::theme;

#[derive(Default)]
pub struct TabAction {
    pub select: Option<String>,
    pub close: Option<String>,
}

/// Browser-style tab strip for open notes.
pub fn show(ui: &mut Ui, tabs: &[String], active: Option<&str>, dirty: bool) -> TabAction {
    let mut action = TabAction::default();
    if tabs.is_empty() {
        return action;
    }

    egui::ScrollArea::horizontal()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for path in tabs {
                    let is_active = active == Some(path.as_str());
                    let title = tab_title(path);
                    let fill = if is_active {
                        theme::FAINT
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let stroke = if is_active {
                        egui::Stroke::new(1.0_f32, theme::ACCENT)
                    } else {
                        egui::Stroke::new(1.0_f32, theme::STROKE)
                    };

                    let inner = egui::Frame::new()
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if dirty && is_active {
                                    ui.label(
                                        RichText::new(regular::CIRCLE)
                                            .size(10.0)
                                            .color(theme::WARN),
                                    );
                                }
                                let label = if is_active {
                                    RichText::new(&title).size(14.0).strong().color(theme::ACCENT)
                                } else {
                                    RichText::new(&title).size(14.0)
                                };
                                let tab = ui.add(egui::Label::new(label).sense(Sense::click()));
                                if tab.clicked() {
                                    action.select = Some(path.clone());
                                }
                                if tab.middle_clicked() {
                                    action.close = Some(path.clone());
                                }
                                let close = ui
                                    .add(
                                        egui::Button::new(RichText::new(regular::X).size(12.0))
                                            .frame(false)
                                            .min_size(egui::vec2(16.0, 16.0)),
                                    )
                                    .on_hover_text("Close");
                                if close.clicked() {
                                    action.close = Some(path.clone());
                                }
                            });
                        });

                    // Click on tab chrome (padding) also selects.
                    if inner.response.clicked() && action.select.is_none() && action.close.is_none()
                    {
                        action.select = Some(path.clone());
                    }
                }
            });
        });

    action
}

fn tab_title(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}
