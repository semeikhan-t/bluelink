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

pub struct FieldOut {
    pub changed: bool,
    /// Enter finishes the field so the line can render as normal Markdown.
    pub commit: bool,
    /// Backspace at the start of the field removes the footnote.
    pub erase: bool,
}

/// One-line field (taller when expanded). Enter commits; a newline never stays
/// in the source line.
pub fn show_field(
    ui: &mut egui::Ui,
    n: u32,
    body: &mut String,
    want_focus: bool,
    editable: bool,
) -> FieldOut {
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
    let mut commit = false;
    let mut erase = false;
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
                let editing = out.response.has_focus() || out.response.lost_focus();
                if editing && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                let at_start = out.cursor_range.is_some_and(|range| {
                    range.primary.ccursor.index == 0 && range.secondary.ccursor.index == 0
                });
                let backspace = ui.input(|i| {
                    i.key_pressed(egui::Key::Backspace)
                        && !i.modifiers.command
                        && !i.modifiers.alt
                        && !i.modifiers.shift
                });
                if editing && at_start && backspace {
                    erase = true;
                }
                if ui
                    .add(egui::Button::new(regular::ARROWS_OUT).frame(false))
                    .clicked()
                {
                    expanded = !expanded;
                }
            });
        });

    if commit_body(body) {
        changed = true;
        commit = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(expand_key, expanded));
    FieldOut {
        changed,
        commit,
        erase,
    }
}

/// Removes the definition line and the `[^n]` marker. The caret is where the
/// marker was. A blank line directly above a trailing definition goes with it.
pub fn erase(text: &mut String, n: u32, line_start: usize, line_end: usize) -> usize {
    let (from, to) = definition_span(text, line_start, line_end);
    let marker_before = find_marker(&text[..from], n);
    let marker_after = find_marker(&text[to..], n).map(|pos| pos + to);
    text.replace_range(from..to, "");
    let removed = to - from;
    let marker_at = marker_before.or_else(|| marker_after.map(|pos| pos - removed));
    if let Some(at) = marker_at {
        return remove_marker(text, n, at);
    }
    if let Some(at) = find_marker(text, n) {
        return remove_marker(text, n, at);
    }
    text[..from.min(text.len())].chars().count()
}

fn definition_span(text: &str, line_start: usize, line_end: usize) -> (usize, usize) {
    let mut from = line_start.min(text.len());
    let mut to = line_end.min(text.len()).max(from);
    let bytes = text.as_bytes();
    if to < bytes.len() && bytes[to] == b'\n' {
        to += 1;
    } else if from > 0 && bytes[from - 1] == b'\n' {
        from -= 1;
        if from > 0 && bytes[from - 1] == b'\n' {
            from -= 1;
        }
    }
    (from, to)
}

fn find_marker(text: &str, n: u32) -> Option<usize> {
    let marker = format!("[^{n}]");
    let mut start = 0;
    while let Some(rel) = text[start..].find(&marker) {
        let at = start + rel;
        let after = at + marker.len();
        if !text[after..].starts_with(':') {
            return Some(at);
        }
        start = after;
    }
    None
}

fn remove_marker(text: &mut String, n: u32, at: usize) -> usize {
    let marker = format!("[^{n}]");
    let caret = text[..at].chars().count();
    text.replace_range(at..at + marker.len(), "");
    caret
}

/// Enter ends the field. The definition stays one source line.
fn commit_body(body: &mut String) -> bool {
    if !body.contains('\n') {
        return false;
    }
    body.retain(|c| c != '\n');
    true
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

    #[test]
    fn enter_strips_the_newline_and_commits() {
        let mut body = "hello\n**there**".to_string();
        assert!(commit_body(&mut body));
        assert_eq!(body, "hello**there**");
        assert!(!commit_body(&mut body));
    }

    #[test]
    fn backspace_at_start_erases_the_footnote() {
        let mut note = "[^1]\n\n[^1]: ".to_string();
        let start = note.rfind("[^1]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, 1, start, end);
        assert_eq!(note, "");
        assert_eq!(caret, 0);

        let mut note = "see [^1] here\n\n[^1]: note".to_string();
        let start = note.rfind("[^1]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, 1, start, end);
        assert_eq!(note, "see  here");
        assert_eq!(caret, 4);

        let mut note = "[^2][^1]\n\n[^1]: \n\n[^2]: ".to_string();
        let start = note.rfind("[^2]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, 2, start, end);
        assert_eq!(note, "[^1]\n\n[^1]: ");
        assert_eq!(caret, 0);
    }
}
