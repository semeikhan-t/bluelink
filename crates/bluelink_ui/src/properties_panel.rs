//! Frontmatter properties strip above the live note body.

use bluelink_core::{parse_frontmatter, write_frontmatter};
use egui::{RichText, TextEdit, Ui};
use egui_phosphor::regular;

use crate::theme;

#[derive(Default)]
pub struct PropertiesAction {
    /// Full buffer rewritten (frontmatter only changed).
    pub rewritten: Option<String>,
    /// True while a property key/value field has focus (for undo burst grouping).
    pub focused: bool,
}

/// Editable key/value rows for YAML frontmatter. Empty fields → compact add affordance.
pub fn show(ui: &mut Ui, buffer: &str, readonly: bool) -> PropertiesAction {
    let mut action = PropertiesAction::default();
    let (mut fields, body) = parse_frontmatter(buffer);
    let body = body.to_string();

    let mut changed = false;
    let mut remove: Option<usize> = None;
    let mut add = false;

    egui::Frame::NONE
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round_sm())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Properties")
                        .size(12.0)
                        .family(theme::display_family())
                        .color(theme::WEAK),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !readonly
                        && ui
                            .small_button(format!("{} Add", regular::PLUS))
                            .on_hover_text("Add property")
                            .clicked()
                    {
                        add = true;
                    }
                });
            });

            if fields.is_empty() && readonly {
                ui.label(RichText::new("No properties").size(13.0).color(theme::WEAK));
                return;
            }

            for (i, (key, value)) in fields.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let key_edit = ui.add_enabled(
                        !readonly,
                        TextEdit::singleline(key)
                            .desired_width(120.0)
                            .hint_text("key")
                            .font(egui::TextStyle::Small),
                    );
                    ui.label(RichText::new(":").color(theme::WEAK));
                    let val_edit = ui.add_enabled(
                        !readonly,
                        TextEdit::singleline(value)
                            .desired_width(ui.available_width() - 28.0)
                            .hint_text("value")
                            .font(egui::TextStyle::Small),
                    );
                    if key_edit.changed() || val_edit.changed() {
                        changed = true;
                    }
                    if key_edit.has_focus() || val_edit.has_focus() {
                        action.focused = true;
                    }
                    if !readonly
                        && ui
                            .small_button(regular::X.to_string())
                            .on_hover_text("Remove")
                            .clicked()
                    {
                        remove = Some(i);
                    }
                });
            }
        });

    if add {
        let mut n = 1_u32;
        let key = loop {
            let candidate = if n == 1 {
                "property".to_string()
            } else {
                format!("property{n}")
            };
            if !fields.iter().any(|(k, _)| k == &candidate) {
                break candidate;
            }
            n += 1;
        };
        fields.push((key, String::new()));
        changed = true;
    }
    if let Some(i) = remove {
        fields.remove(i);
        changed = true;
    }

    if changed && !readonly {
        fields.retain(|(k, _)| !k.trim().is_empty());
        action.rewritten = Some(write_frontmatter(&fields, &body));
    }

    action
}
