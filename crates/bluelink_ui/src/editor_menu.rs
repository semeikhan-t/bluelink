//! Notion-like right-click menu for the live Markdown editor.

use egui::{Ui, ViewportCommand};
use egui_phosphor::regular;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    WikiLink,
    ExternalLink,
    /// Wrap selection (or insert markers at cursor) with `before` / `after`.
    Wrap {
        before: &'static str,
        after: &'static str,
    },
    /// Replace Markdown block prefix on the current line (`""` = plain).
    SetLinePrefix(&'static str),
    InsertSnippet(&'static str),
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// Draw the context-menu contents. Returns the chosen action, if any.
pub fn menu_ui(ui: &mut Ui, has_selection: bool) -> Option<EditorAction> {
    let mut action = None;

    if ui
        .button(format!("{}  Add link", regular::LINK))
        .clicked()
    {
        action = Some(EditorAction::WikiLink);
        ui.close_menu();
    }
    if ui
        .button(format!("{}  Add external link", regular::ARROW_SQUARE_OUT))
        .clicked()
    {
        action = Some(EditorAction::ExternalLink);
        ui.close_menu();
    }

    ui.separator();

    ui.menu_button(format!("{}  Format", regular::PAINT_BRUSH), |ui| {
        if ui
            .button(format!("{}  Bold", regular::TEXT_B))
            .clicked()
        {
            action = Some(EditorAction::Wrap {
                before: "**",
                after: "**",
            });
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Italic", regular::TEXT_ITALIC))
            .clicked()
        {
            action = Some(EditorAction::Wrap {
                before: "*",
                after: "*",
            });
            ui.close_menu();
        }
        if ui.button(format!("{}  Code", regular::CODE)).clicked() {
            action = Some(EditorAction::Wrap {
                before: "`",
                after: "`",
            });
            ui.close_menu();
        }
    });

    ui.menu_button(format!("{}  Paragraph", regular::PARAGRAPH), |ui| {
        if ui
            .button(format!("{}  Plain text", regular::TEXT_T))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix(""));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Heading 1", regular::TEXT_H_ONE))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("# "));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Heading 2", regular::TEXT_H_TWO))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("## "));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Heading 3", regular::TEXT_H_THREE))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("### "));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Quote", regular::TEXT_AA))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("> "));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Bullet list", regular::LIST_BULLETS))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("- "));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Numbered list", regular::LIST_NUMBERS))
            .clicked()
        {
            action = Some(EditorAction::SetLinePrefix("1. "));
            ui.close_menu();
        }
    });

    ui.menu_button(format!("{}  Insert", regular::LIST_PLUS), |ui| {
        if ui
            .button(format!("{}  Code block", regular::CODE_BLOCK))
            .clicked()
        {
            action = Some(EditorAction::InsertSnippet("```\n\n```"));
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Divider", regular::MINUS))
            .clicked()
        {
            action = Some(EditorAction::InsertSnippet("---"));
            ui.close_menu();
        }
    });

    ui.separator();

    ui.add_enabled_ui(has_selection, |ui| {
        if ui
            .button(format!("{}  Cut", regular::SCISSORS))
            .clicked()
        {
            action = Some(EditorAction::Cut);
            ui.close_menu();
        }
        if ui.button(format!("{}  Copy", regular::COPY)).clicked() {
            action = Some(EditorAction::Copy);
            ui.close_menu();
        }
    });
    if ui
        .button(format!("{}  Paste", regular::CLIPBOARD))
        .clicked()
    {
        action = Some(EditorAction::Paste);
        ui.close_menu();
    }
    if ui
        .button(format!("{}  Paste as plain text", regular::CLIPBOARD_TEXT))
        .clicked()
    {
        action = Some(EditorAction::Paste);
        ui.close_menu();
    }

    ui.separator();

    if ui
        .button(format!("{}  Select all", regular::SELECTION_ALL))
        .clicked()
    {
        action = Some(EditorAction::SelectAll);
        ui.close_menu();
    }

    action
}

/// Dispatch clipboard viewport commands (TextEdit must be focused).
pub fn dispatch_clipboard(ui: &Ui, action: &EditorAction) {
    let cmd = match action {
        EditorAction::Cut => Some(ViewportCommand::RequestCut),
        EditorAction::Copy => Some(ViewportCommand::RequestCopy),
        EditorAction::Paste => Some(ViewportCommand::RequestPaste),
        _ => None,
    };
    if let Some(cmd) = cmd {
        ui.ctx().send_viewport_cmd(cmd);
    }
}

/// Char index → byte index.
pub fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// Wrap selection (or insert markers at caret). Returns new caret char index.
pub fn wrap_selection(
    text: &mut String,
    primary: usize,
    secondary: usize,
    before: &str,
    after: &str,
) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    let byte_lo = char_to_byte(text, lo);
    let byte_hi = char_to_byte(text, hi);
    let selected = text[byte_lo..byte_hi].to_string();
    let replacement = format!("{before}{selected}{after}");
    text.replace_range(byte_lo..byte_hi, &replacement);
    lo + before.chars().count() + selected.chars().count()
}

/// Strip ATX heading / quote / list prefixes from a single line.
pub fn strip_line_prefix(line: &str) -> String {
    let s = line.trim_start();
    let s = if let Some(rest) = s.strip_prefix('>') {
        rest.trim_start()
    } else {
        s
    };
    let hash = s.bytes().take_while(|&b| b == b'#').count();
    let s = if (1..=6).contains(&hash) {
        let rest = &s[hash..];
        rest.strip_prefix(' ').unwrap_or(rest)
    } else {
        s
    };
    let s = if let Some(rest) = s.strip_prefix("- [ ] ") {
        rest
    } else if let Some(rest) = s.strip_prefix("- [x] ") {
        rest
    } else if let Some(rest) = s.strip_prefix("- [X] ") {
        rest
    } else if let Some(rest) = s.strip_prefix("- ") {
        rest
    } else if let Some(rest) = s.strip_prefix("* ") {
        rest
    } else if let Some(rest) = numbered_list_rest(s) {
        rest
    } else {
        s
    };
    s.to_string()
}

fn numbered_list_rest(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = &s[digits..];
    rest.strip_prefix(". ")
}

/// Replace the Markdown block prefix on a single line.
pub fn set_line_prefix(line: &str, prefix: &str) -> String {
    let body = strip_line_prefix(line);
    if prefix.is_empty() {
        body
    } else {
        format!("{prefix}{body}")
    }
}

/// Apply a line-prefix change at the caret's line inside a multi-line buffer.
/// Returns new caret char index (start of body after prefix).
pub fn set_prefix_at_char(text: &mut String, char_idx: usize, prefix: &str) -> usize {
    let byte = char_to_byte(text, char_idx);
    let line_start = text[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[byte..]
        .find('\n')
        .map(|i| byte + i)
        .unwrap_or(text.len());
    let line = text[line_start..line_end].to_string();
    let new_line = set_line_prefix(&line, prefix);
    text.replace_range(line_start..line_end, &new_line);
    let start_chars = text[..line_start].chars().count();
    start_chars + prefix.chars().count()
}

/// Insert (or replace selection) with `snippet`. Returns caret after snippet.
pub fn insert_at(text: &mut String, primary: usize, secondary: usize, snippet: &str) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    let byte_lo = char_to_byte(text, lo);
    let byte_hi = char_to_byte(text, hi);
    text.replace_range(byte_lo..byte_hi, snippet);
    lo + snippet.chars().count()
}

/// Apply text-mutating actions. Returns new primary caret char index when text changed.
pub fn apply_text_action(
    text: &mut String,
    primary: usize,
    secondary: usize,
    action: &EditorAction,
) -> Option<usize> {
    match action {
        EditorAction::WikiLink => {
            let lo = primary.min(secondary);
            let hi = primary.max(secondary);
            if lo == hi {
                Some(insert_at(text, primary, secondary, "[["))
            } else {
                Some(wrap_selection(text, primary, secondary, "[[", "]]"))
            }
        }
        EditorAction::ExternalLink => {
            let lo = primary.min(secondary);
            let hi = primary.max(secondary);
            if lo == hi {
                // Cursor lands inside the URL placeholder.
                let caret = insert_at(text, primary, secondary, "[](https://)");
                Some(caret - ")".chars().count())
            } else {
                let byte_lo = char_to_byte(text, lo);
                let byte_hi = char_to_byte(text, hi);
                let selected = text[byte_lo..byte_hi].to_string();
                let replacement = format!("[{selected}](https://)");
                text.replace_range(byte_lo..byte_hi, &replacement);
                // After `](https://` — before closing `)`.
                Some(lo + selected.chars().count() + "](https://".chars().count())
            }
        }
        EditorAction::Wrap { before, after } => {
            Some(wrap_selection(text, primary, secondary, before, after))
        }
        EditorAction::SetLinePrefix(prefix) => {
            Some(set_prefix_at_char(text, primary, prefix))
        }
        EditorAction::InsertSnippet(snippet) => {
            Some(insert_at(text, primary, secondary, snippet))
        }
        EditorAction::Cut
        | EditorAction::Copy
        | EditorAction::Paste
        | EditorAction::SelectAll => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_inserts_markers_around_selection() {
        let mut s = "hello world".to_string();
        let caret = wrap_selection(&mut s, 0, 5, "**", "**");
        assert_eq!(s, "**hello** world");
        assert_eq!(caret, 7); // after "**hello"
    }

    #[test]
    fn wrap_at_caret_inserts_empty_pair() {
        let mut s = "ab".to_string();
        let caret = wrap_selection(&mut s, 1, 1, "`", "`");
        assert_eq!(s, "a``b");
        assert_eq!(caret, 2);
    }

    #[test]
    fn strip_and_set_prefix() {
        assert_eq!(strip_line_prefix("## Hello"), "Hello");
        assert_eq!(strip_line_prefix("> quoted"), "quoted");
        assert_eq!(strip_line_prefix("- item"), "item");
        assert_eq!(strip_line_prefix("1. item"), "item");
        assert_eq!(set_line_prefix("## Hello", "- "), "- Hello");
        assert_eq!(set_line_prefix("- Hello", ""), "Hello");
        assert_eq!(set_line_prefix("- Hello", "# "), "# Hello");
    }

    #[test]
    fn set_prefix_in_multiline() {
        let mut s = "a\n## Hello\nc".to_string();
        // caret on 'H' of Hello
        let caret = set_prefix_at_char(&mut s, 5, "- ");
        assert_eq!(s, "a\n- Hello\nc");
        assert_eq!(&s[char_to_byte(&s, caret)..], "Hello\nc");
    }

    #[test]
    fn wiki_and_external_helpers() {
        let mut s = "note".to_string();
        let caret = apply_text_action(&mut s, 0, 4, &EditorAction::WikiLink).unwrap();
        assert_eq!(s, "[[note]]");
        assert_eq!(caret, 6);

        let mut s = String::new();
        let caret = apply_text_action(&mut s, 0, 0, &EditorAction::ExternalLink).unwrap();
        assert_eq!(s, "[](https://)");
        assert_eq!(&s[..caret], "[](https://");
    }
}
