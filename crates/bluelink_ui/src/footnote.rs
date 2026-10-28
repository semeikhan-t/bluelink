//! Footnote definitions `[^id]: …` with optional indented continuation lines.
//!
//! Enter inside the live field inserts a paragraph (`\n` in the body). Escape
//! or leaving the line commits. Continuation lines in the file use four spaces.

use egui::{text::CCursor, text::CCursorRange, Sense, TextEdit};
use egui_phosphor::regular;

use crate::preview::{advance_fence, FenceRole};
use crate::theme;

/// Opaque footnote label (`1`, `bignote`, …).
pub type FootnoteId = String;

/// Indent for continuation lines after `[^id]:`.
pub const FOOTNOTE_CONT: &str = "    ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootnotePlace {
    None,
    Head {
        end: usize,
    },
    Tail {
        start: usize,
        end: usize,
    },
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// First line of a definition: `[^id]: body…`.
pub fn parse_def(line: &str) -> Option<(FootnoteId, String)> {
    let rest = line.strip_prefix("[^")?;
    let (id, after) = rest.split_once("]:")?;
    if !valid_id(id) {
        return None;
    }
    let body = after.strip_prefix(' ').unwrap_or(after);
    Some((id.to_string(), body.to_string()))
}

/// Single-line form (no continuations).
pub fn format_def(id: &str, body: &str) -> String {
    format!("[^{id}]: {body}")
}

/// Full block: body newlines become indented continuation lines.
pub fn format_block(id: &str, body: &str) -> String {
    let mut parts = body.split('\n');
    let first = parts.next().unwrap_or("");
    let mut out = format_def(id, first);
    for line in parts {
        out.push('\n');
        out.push_str(FOOTNOTE_CONT);
        out.push_str(line);
    }
    out
}

/// Head line plus indented tails → `(id, body with \\n)`.
pub fn parse_block(lines: &[String]) -> Option<(FootnoteId, String)> {
    let (id, mut body) = parse_def(lines.first()?)?;
    for line in lines.iter().skip(1) {
        let cont = strip_cont_indent(line)?;
        body.push('\n');
        body.push_str(cont);
    }
    Some((id, body))
}

fn strip_cont_indent(line: &str) -> Option<&str> {
    if line.starts_with(FOOTNOTE_CONT) {
        Some(&line[FOOTNOTE_CONT.len()..])
    } else if let Some(rest) = line.strip_prefix('\t') {
        Some(rest)
    } else if line.chars().next().is_some_and(|c| c.is_whitespace()) {
        Some(line.trim_start())
    } else {
        None
    }
}

fn is_cont_line(line: &str) -> bool {
    !line.is_empty() && line.chars().next().is_some_and(|c| c.is_whitespace())
}

/// Map each line to a footnote head/tail. Fence lines are never footnotes.
pub fn places(lines: &[String]) -> Vec<FootnotePlace> {
    let mut out = vec![FootnotePlace::None; lines.len()];
    let mut i = 0;
    let mut fence = None;
    while i < lines.len() {
        let in_fence = !matches!(advance_fence(&lines[i], &mut fence), FenceRole::Outside);
        if !in_fence && parse_def(&lines[i]).is_some() {
            let mut end = i + 1;
            while end < lines.len() {
                let _ = advance_fence(&lines[end], &mut fence);
                if is_cont_line(&lines[end]) {
                    end += 1;
                } else {
                    break;
                }
            }
            out[i] = FootnotePlace::Head { end };
            for j in (i + 1)..end {
                out[j] = FootnotePlace::Tail { start: i, end };
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

pub struct FieldOut {
    pub changed: bool,
    /// Escape finishes the field so the line can render as normal Markdown.
    pub commit: bool,
    /// Backspace at the start of the field removes the footnote.
    pub erase: bool,
    /// Click on the `[^{id}]` label — jump to the first in-note marker.
    pub jump_to_marker: bool,
}

/// Multiline definition field. Enter inserts a paragraph; Escape commits.
pub fn show_field(
    ui: &mut egui::Ui,
    id: &str,
    body: &mut String,
    want_focus: bool,
    editable: bool,
) -> FieldOut {
    let mut changed = false;
    let mut commit = false;
    let mut erase = false;
    let mut jump_to_marker = false;

    let label = ui
        .add(
            egui::Label::new(
                egui::RichText::new(format!("[^{id}]"))
                    .size(13.0)
                    .color(theme::WEAK),
            )
            .sense(Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Jump to first reference");
    if label.clicked() {
        jump_to_marker = true;
    }

    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 28.0).max(40.0);
                let edit_id = ui.id().with(("footnote_body", id));
                let out = ui
                    .add_enabled_ui(editable, |ui| {
                        TextEdit::multiline(body)
                            .id(edit_id)
                            .desired_rows(3)
                            .desired_width(width)
                            .frame(false)
                            .show(ui)
                    })
                    .inner;
                if want_focus {
                    out.response.request_focus();
                    let end = body.chars().count();
                    let mut state = out.state;
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(end))));
                    state.store(ui.ctx(), out.response.id);
                }
                if out.response.changed() {
                    changed = true;
                }
                let editing = out.response.has_focus() || out.response.lost_focus();
                if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
                let _ = ui.add(egui::Button::new(regular::ARTICLE).frame(false));
            });
        });

    FieldOut {
        changed,
        commit,
        erase,
        jump_to_marker,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewHit {
    None,
    /// Clicked the `[^{id}]` label — jump to the first marker.
    Marker,
    /// Clicked the body — enter edit.
    Body,
}

/// Paint a read-only preview of a definition block.
pub fn paint_preview(ui: &mut egui::Ui, id: &str, body: &str) -> PreviewHit {
    let mut hit = PreviewHit::None;
    let frame = egui::Frame::NONE
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let resp = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("[^{id}]"))
                            .size(12.0)
                            .strong()
                            .color(theme::WEAK),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                hit = PreviewHit::Marker;
            }
            for para in body.split('\n') {
                let shown = if para.is_empty() { " " } else { para };
                let body_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(shown)
                            .size(theme::EDITOR_SIZE)
                            .color(theme::TEXT),
                    )
                    .sense(Sense::click()),
                );
                if body_resp.clicked() && hit == PreviewHit::None {
                    hit = PreviewHit::Body;
                }
            }
        });
    if hit == PreviewHit::None && frame.response.clicked() {
        hit = PreviewHit::Body;
    }
    hit
}

/// Removes the definition block and one `[^id]` marker. Caret lands where the
/// marker was. A blank line directly above a trailing definition goes with it.
pub fn erase(text: &mut String, id: &str, line_start: usize, line_end: usize) -> usize {
    let (from, to) = definition_span(text, line_start, line_end);
    let marker_before = find_marker(&text[..from], id);
    let marker_after = find_marker(&text[to..], id).map(|pos| pos + to);
    text.replace_range(from..to, "");
    let removed = to - from;
    let marker_at = marker_before.or_else(|| marker_after.map(|pos| pos - removed));
    if let Some(at) = marker_at {
        return remove_marker(text, id, at);
    }
    if let Some(at) = find_marker(text, id) {
        return remove_marker(text, id, at);
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

pub fn find_marker(text: &str, id: &str) -> Option<usize> {
    let marker = format!("[^{id}]");
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

fn remove_marker(text: &mut String, id: &str, at: usize) -> usize {
    let marker = format!("[^{id}]");
    let caret = text[..at].chars().count();
    text.replace_range(at..at + marker.len(), "");
    caret
}

/// Line index of the definition head for `id`.
pub fn def_line(text: &str, id: &str) -> Option<usize> {
    for (i, line) in text.split('\n').enumerate() {
        if parse_def(line).is_some_and(|(d, _)| d == id) {
            return Some(i);
        }
    }
    None
}

/// Line index of the first in-note `[^id]` marker (not a definition).
pub fn first_marker_line(text: &str, id: &str) -> Option<usize> {
    let at = find_marker(text, id)?;
    Some(text[..at].bytes().filter(|&b| b == b'\n').count())
}

fn find_inline_footnote(rest: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = rest[start..].find("^[") {
        let i = start + rel;
        if let Some(close) = rest[i + 2..].find(']') {
            if close > 0 {
                return Some(i);
            }
            start = i + 2;
        } else {
            break;
        }
    }
    None
}

fn find_ref_marker(rest: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = rest[start..].find("[^") {
        let i = start + rel;
        let after = &rest[i + 2..];
        if let Some(close) = after.find(']') {
            let id = &after[..close];
            let end = i + 2 + close + 1;
            if valid_id(id) && !rest[end..].starts_with(':') {
                return Some(i);
            }
            start = end;
        } else {
            break;
        }
    }
    None
}

/// Split plain text for inline `^[…]` and reference `[^id]` (not followed by `:`).
pub fn split_inline_footnotes(text: &str) -> Vec<InlinePiece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let inline_at = find_inline_footnote(rest);
        let ref_at = find_ref_marker(rest);

        let next = match (inline_at, ref_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        let Some(at) = next else {
            out.push(InlinePiece::Text(rest));
            break;
        };
        if at > 0 {
            out.push(InlinePiece::Text(&rest[..at]));
        }
        if rest[at..].starts_with("^[") {
            let inner_start = at + 2;
            let close = rest[inner_start..].find(']').unwrap();
            let inner = &rest[inner_start..inner_start + close];
            out.push(InlinePiece::Inline { text: inner });
            rest = &rest[inner_start + close + 1..];
        } else {
            let id_start = at + 2;
            let close = rest[id_start..].find(']').unwrap();
            let id = &rest[id_start..id_start + close];
            out.push(InlinePiece::Ref { id });
            rest = &rest[id_start + close + 1..];
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlinePiece<'a> {
    Text(&'a str),
    Inline { text: &'a str },
    Ref { id: &'a str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_named_and_numeric() {
        assert_eq!(
            parse_def("[^1]: hello"),
            Some(("1".into(), "hello".into()))
        );
        assert_eq!(
            parse_def("[^bignote]: x"),
            Some(("bignote".into(), "x".into()))
        );
        assert_eq!(parse_def("[^12]:"), Some(("12".into(), "".into())));
        assert_eq!(parse_def("[^bad id]: x"), None);
        assert_eq!(parse_def("see [^1]"), None);
        assert_eq!(format_def("2", "note"), "[^2]: note");
    }

    #[test]
    fn multiline_block_round_trip() {
        let body = "first\nsecond";
        let block = format_block("note", body);
        assert_eq!(block, "[^note]: first\n    second");
        let lines: Vec<String> = block.lines().map(str::to_string).collect();
        let (id, parsed) = parse_block(&lines).unwrap();
        assert_eq!(id, "note");
        assert_eq!(parsed, body);

        let places = places(&lines);
        assert_eq!(places[0], FootnotePlace::Head { end: 2 });
        assert_eq!(places[1], FootnotePlace::Tail { start: 0, end: 2 });
    }

    #[test]
    fn backspace_at_start_erases_the_footnote() {
        let mut note = "[^1]\n\n[^1]: ".to_string();
        let start = note.rfind("[^1]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, "1", start, end);
        assert_eq!(note, "");
        assert_eq!(caret, 0);

        let mut note = "see [^1] here\n\n[^1]: note".to_string();
        let start = note.rfind("[^1]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, "1", start, end);
        assert_eq!(note, "see  here");
        assert_eq!(caret, 4);

        let mut note = "[^2][^1]\n\n[^1]: \n\n[^2]: ".to_string();
        let start = note.rfind("[^2]:").unwrap();
        let end = note.len();
        let caret = erase(&mut note, "2", start, end);
        assert_eq!(note, "[^1]\n\n[^1]: ");
        assert_eq!(caret, 0);
    }

    #[test]
    fn inline_split_and_nav_helpers() {
        let pieces = split_inline_footnotes("a ^[hi] b [^x] c");
        assert_eq!(
            pieces,
            vec![
                InlinePiece::Text("a "),
                InlinePiece::Inline { text: "hi" },
                InlinePiece::Text(" b "),
                InlinePiece::Ref { id: "x" },
                InlinePiece::Text(" c"),
            ]
        );
        let text = "see [^bignote] here\n\n[^bignote]: body\n    more";
        assert_eq!(first_marker_line(text, "bignote"), Some(0));
        assert_eq!(def_line(text, "bignote"), Some(2));

        // A def-shaped token must not hide a later real marker.
        let pieces = split_inline_footnotes("x [^skip]: y [^ok] z");
        assert!(pieces.iter().any(|p| matches!(p, InlinePiece::Ref { id: "ok" })));
    }
}
