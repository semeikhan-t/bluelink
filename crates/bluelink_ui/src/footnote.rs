//! Visual editor for a definition line `[^N]: text`.
//! The marker in the paragraph stays plain text. The definition is this field.

use egui::{text::CCursor, text::CCursorRange, TextEdit};
use egui_phosphor::regular;

use crate::theme;

pub fn parse_def(line: &str) -> Option<(u32, String)> {
    let rest = line.strip_prefix("[^")?;
    let (num, after) = rest.split_once("]:")?;
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = num.parse().ok()?;
    let body = after.strip_prefix(' ').unwrap_or(after);
    Some((n, body.to_string()))
}

pub fn format_def(n: u32, body: &str) -> String {
    format!("[^{n}]: {body}")
}

/// One-line field (taller when expanded). Newlines are stripped so the
/// definition stays a single source line. Returns true when `body` changed.
pub fn show_field(
    ui: &mut egui::Ui,
    n: u32,
    body: &mut String,
    want_focus: bool,
    editable: bool,
) -> bool {
    let expand_key = egui::Id::new(("footnote_expand", n));
    let mut expanded = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(expand_key).unwrap_or(false));

    ui.label(
        egui::RichText::new(format!("[^{n}]"))
            .size(13.0)
            .color(theme::WEAK),
    );

    let mut changed = false;
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 28.0).max(40.0);
                let id = ui.id().with(("footnote_body", n));
                let edit = if expanded {
                    TextEdit::multiline(body).desired_rows(4)
                } else {
                    TextEdit::singleline(body)
                };
                let out = ui
                    .add_enabled_ui(editable, |ui| {
                        edit.id(id)
                            .desired_width(width)
                            .frame(false)
                            .show(ui)
                    })
                    .inner;
                if want_focus {
                    out.response.request_focus();
                    let end = body.chars().count();
                    let mut state = out.state;
                    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(end))));
                    state.store(ui.ctx(), out.response.id);
                }
                if out.response.changed() {
                    changed = true;
                }
                if ui
                    .add(egui::Button::new(regular::ARROWS_OUT).frame(false))
                    .clicked()
                {
                    expanded = !expanded;
                }
            });
        });

    if body.contains('\n') {
        body.retain(|c| c != '\n');
        changed = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(expand_key, expanded));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_definition() {
        assert_eq!(parse_def("[^1]: hello"), Some((1, "hello".into())));
        assert_eq!(parse_def("[^12]:"), Some((12, "".into())));
        assert_eq!(parse_def("[^1]:no-space"), Some((1, "no-space".into())));
        assert_eq!(parse_def("see [^1]"), None);
        assert_eq!(format_def(2, "note"), "[^2]: note");
    }
}
