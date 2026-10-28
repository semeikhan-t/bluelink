//! Frontmatter properties strip above the live note body.
//!
//! Pretty = typed key/value rows; Raw = YAML interior between `---` fences.
//! Fold is session-only and does not rewrite the note buffer.
//!
//! `ponytail:` date-like values stay plain strings until a date widget exists.

use bluelink_core::{
    frontmatter_raw_yaml, parse_frontmatter, splice_frontmatter_yaml, write_frontmatter,
};
use egui::{RichText, TextEdit, Ui};
use egui_phosphor::regular;

use crate::theme;

/// How the properties block renders field values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropertiesView {
    #[default]
    Pretty,
    Raw,
}

impl PropertiesView {
    pub fn toggle(self) -> Self {
        match self {
            Self::Pretty => Self::Raw,
            Self::Raw => Self::Pretty,
        }
    }
}

#[derive(Default)]
pub struct PropertiesAction {
    /// Full buffer rewritten (frontmatter only changed).
    pub rewritten: Option<String>,
    /// True while a property key/value field has focus (for undo burst grouping).
    pub focused: bool,
}

/// Display-layer classification for Pretty mode (not separate editor commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyValueKind {
    Bool,
    Number,
    List,
    /// Includes date-like strings — no dedicated date widget yet.
    String,
}

pub fn classify_property_value(value: &str) -> PropertyValueKind {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
        return PropertyValueKind::Bool;
    }
    if value.contains('\n') {
        return PropertyValueKind::List;
    }
    if is_yaml_number(trimmed) {
        return PropertyValueKind::Number;
    }
    PropertyValueKind::String
}

fn is_yaml_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Reject date-like `YYYY-MM-DD` so it stays a string (`ponytail:`).
    if s.len() >= 8 && s.as_bytes().get(4) == Some(&b'-') {
        return false;
    }
    let mut chars = s.chars().peekable();
    if matches!(chars.peek(), Some('+' | '-')) {
        chars.next();
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    for c in chars {
        match c {
            '0'..='9' => saw_digit = true,
            '.' if !saw_dot => saw_dot = true,
            _ => return false,
        }
    }
    saw_digit
}

/// Editable properties block. Empty fields → compact add affordance.
pub fn show(
    ui: &mut Ui,
    buffer: &str,
    readonly: bool,
    folded: &mut bool,
    view: &mut PropertiesView,
) -> PropertiesAction {
    let mut action = PropertiesAction::default();
    let (mut fields, body) = parse_frontmatter(buffer);
    let body = body.to_string();
    let key_count = fields.len();

    let mut changed = false;
    let mut remove: Option<usize> = None;
    let mut add = false;
    let mut raw_changed: Option<String> = None;

    egui::Frame::NONE
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round_sm())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let chevron = if *folded {
                    regular::CARET_RIGHT
                } else {
                    regular::CARET_DOWN
                };
                let fold_label = if *folded && key_count > 0 {
                    format!("{chevron} Properties ({key_count})")
                } else {
                    format!("{chevron} Properties")
                };
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(fold_label)
                                .size(12.0)
                                .family(theme::display_family())
                                .color(theme::WEAK),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    *folded = !*folded;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !*folded {
                        if !readonly
                            && *view == PropertiesView::Pretty
                            && ui
                                .small_button(format!("{} Add", regular::PLUS))
                                .on_hover_text("Add property")
                                .clicked()
                        {
                            add = true;
                        }
                        let (icon, tip) = match *view {
                            PropertiesView::Pretty => (regular::CODE, "Show raw YAML"),
                            PropertiesView::Raw => (regular::LIST_BULLETS, "Show fields"),
                        };
                        if ui
                            .small_button(icon.to_string())
                            .on_hover_text(tip)
                            .clicked()
                        {
                            *view = view.toggle();
                        }
                    }
                });
            });

            if *folded {
                return;
            }

            match *view {
                PropertiesView::Raw => {
                    let mut yaml = frontmatter_raw_yaml(buffer)
                        .unwrap_or("")
                        .trim_end_matches('\n')
                        .to_string();
                    let rows = 4.max(yaml.lines().count().max(1));
                    let edit = ui.add_enabled(
                        !readonly,
                        TextEdit::multiline(&mut yaml)
                            .desired_width(ui.available_width())
                            .desired_rows(rows)
                            .code_editor()
                            .font(egui::TextStyle::Monospace),
                    );
                    if edit.has_focus() {
                        action.focused = true;
                    }
                    if edit.changed() {
                        raw_changed = Some(yaml);
                    }
                }
                PropertiesView::Pretty => {
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

                            let kind = classify_property_value(value);
                            let val_focused = match kind {
                                PropertyValueKind::Bool => {
                                    let mut checked = value.trim().eq_ignore_ascii_case("true");
                                    let resp = ui.add_enabled(
                                        !readonly,
                                        egui::Checkbox::new(&mut checked, ""),
                                    );
                                    if resp.changed() {
                                        *value = if checked {
                                            "true".into()
                                        } else {
                                            "false".into()
                                        };
                                        changed = true;
                                    }
                                    resp.has_focus()
                                }
                                PropertyValueKind::Number => {
                                    let mut draft = value.clone();
                                    let val_edit = ui.add_enabled(
                                        !readonly,
                                        TextEdit::singleline(&mut draft)
                                            .desired_width(ui.available_width() - 28.0)
                                            .hint_text("number")
                                            .font(egui::TextStyle::Small),
                                    );
                                    if val_edit.changed() {
                                        // Keep string field if the user leaves a non-number.
                                        *value = draft;
                                        changed = true;
                                    }
                                    val_edit.has_focus()
                                }
                                PropertyValueKind::List => {
                                    let mut draft = value.replace('\n', ", ");
                                    let val_edit = ui.add_enabled(
                                        !readonly,
                                        TextEdit::singleline(&mut draft)
                                            .desired_width(ui.available_width() - 28.0)
                                            .hint_text("a, b, c")
                                            .font(egui::TextStyle::Small),
                                    );
                                    if val_edit.changed() {
                                        *value = draft
                                            .split(',')
                                            .map(str::trim)
                                            .filter(|s| !s.is_empty())
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        changed = true;
                                    }
                                    val_edit.has_focus()
                                }
                                PropertyValueKind::String => {
                                    // ponytail: date-like values use the same string field.
                                    let val_edit = ui.add_enabled(
                                        !readonly,
                                        TextEdit::singleline(value)
                                            .desired_width(ui.available_width() - 28.0)
                                            .hint_text("value")
                                            .font(egui::TextStyle::Small),
                                    );
                                    if val_edit.changed() {
                                        changed = true;
                                    }
                                    val_edit.has_focus()
                                }
                            };

                            if key_edit.changed() {
                                changed = true;
                            }
                            if key_edit.has_focus() || val_focused {
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
                }
            }
        });

    if let Some(yaml) = raw_changed {
        if !readonly {
            action.rewritten = Some(splice_frontmatter_yaml(&yaml, &body));
        }
        return action;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bool_number_list_string() {
        assert_eq!(classify_property_value("true"), PropertyValueKind::Bool);
        assert_eq!(classify_property_value("FALSE"), PropertyValueKind::Bool);
        assert_eq!(classify_property_value("42"), PropertyValueKind::Number);
        assert_eq!(classify_property_value("-3.5"), PropertyValueKind::Number);
        assert_eq!(
            classify_property_value("a\nb"),
            PropertyValueKind::List
        );
        assert_eq!(
            classify_property_value("2024-01-15"),
            PropertyValueKind::String
        );
        assert_eq!(classify_property_value("hello"), PropertyValueKind::String);
    }

    #[test]
    fn view_toggle_symmetric() {
        assert_eq!(PropertiesView::Pretty.toggle(), PropertiesView::Raw);
        assert_eq!(PropertiesView::Raw.toggle(), PropertiesView::Pretty);
    }
}
