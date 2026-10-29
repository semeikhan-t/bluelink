//! Notion-like right-click menu for the live Markdown editor.
//!
//! Single `Area` popup (not per-line egui `context_menu`) to avoid freezing the UI.
//! Submenus are our own side panels — not egui `menu_button`.

use egui::{Id, Pos2, Rect, Ui, Vec2, ViewportCommand};
use egui_phosphor::regular;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    WikiLink,
    ExternalLink,
    /// Insert `#` at the caret, or `#` before the selection (word → `#tag`).
    InsertTag,
    /// Insert `![[]]` with the caret between the brackets (wiki embed).
    InsertEmbed,
    Wrap {
        before: &'static str,
        after: &'static str,
    },
    SetLinePrefix(&'static str),
    InsertSnippet(&'static str),
    /// Host opens a file picker and inserts `![[attachments/…]]`.
    AttachFile,
    /// `[^N]` at the caret and `[^N]: ` at the end of the note.
    Footnote,
    /// Wrap / unwrap `%%…%%` on the selection or current line.
    ToggleComments,
    /// Add one list indent level (`  `) on list lines in the selection.
    IndentList,
    /// Remove one list indent level on list lines in the selection.
    UnindentList,
    /// Collapse the innermost open fold at the caret (callout, then heading/indent).
    FoldMore,
    /// Open the innermost closed fold at the caret.
    FoldLess,
    FoldAll,
    UnfoldAll,
    /// Collapse / expand the Properties strip (session-only; YAML unchanged).
    ToggleFoldProperties,
    /// Host toggles [`crate::editor::ViewMode::Reading`].
    ToggleReading,
    /// Host toggles [`crate::editor::ViewMode::Source`].
    ToggleSource,
    Cut,
    Copy,
    Paste,
    /// Text paste only — never image→embed (unlike [`EditorAction::Paste`]).
    PastePlain,
    /// Strip inline `** * \` ~~ ==` markers from the selection.
    ClearFormatting,
    /// Wrap / unwrap `$…$` (not block `$$`).
    ToggleInlineMath,
    SelectAll,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubMenu {
    Format,
    Paragraph,
    Insert,
    Fold,
    View,
}

/// Where the popup was opened and how to apply the chosen action.
#[derive(Clone)]
pub struct CtxMenu {
    pub pos: Pos2,
    /// Live-mode line index; `None` = full-document edit buffer.
    pub line: Option<usize>,
    pub primary: usize,
    pub secondary: usize,
    submenu: Option<SubMenu>,
}

const CTX_KEY: &str = "bluelink_editor_ctx_menu";

pub fn load_menu(ctx: &egui::Context) -> Option<CtxMenu> {
    ctx.data_mut(|d| d.get_temp::<Option<CtxMenu>>(Id::new(CTX_KEY)).flatten())
}

pub fn store_menu(ctx: &egui::Context, menu: Option<CtxMenu>) {
    ctx.data_mut(|d| d.insert_temp(Id::new(CTX_KEY), menu));
}

pub fn open_at(
    ctx: &egui::Context,
    pos: Pos2,
    line: Option<usize>,
    primary: usize,
    secondary: usize,
) {
    crate::table::close(ctx);
    store_menu(
        ctx,
        Some(CtxMenu {
            pos,
            line,
            primary,
            secondary,
            submenu: None,
        }),
    );
}

pub fn close(ctx: &egui::Context) {
    store_menu(ctx, None);
}

/// Draw the floating menu if open. Returns the chosen action (and closes).
pub fn show_popup(ctx: &egui::Context) -> Option<(CtxMenu, EditorAction)> {
    let Some(mut menu) = load_menu(ctx) else {
        return None;
    };

    let has_selection = menu.primary != menu.secondary;
    let mut action = None;
    let mut hit_rects: Vec<Rect> = Vec::new();

    let main = egui::Area::new(Id::new("bluelink_editor_ctx_popup"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.pos)
        .sense(egui::Sense::hover())
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(210.0);
                ui.spacing_mut().item_spacing.y = 2.0;

                if row(ui, regular::LINK, "Add link", false) {
                    action = Some(EditorAction::WikiLink);
                }
                if row(ui, regular::ARROW_SQUARE_OUT, "Add external link", false) {
                    action = Some(EditorAction::ExternalLink);
                }

                ui.separator();

                if row(ui, regular::PAINT_BRUSH, "Format", true) {
                    menu.submenu = Some(SubMenu::Format);
                }
                if row(ui, regular::PARAGRAPH, "Paragraph", true) {
                    menu.submenu = Some(SubMenu::Paragraph);
                }
                if row(ui, regular::LIST_PLUS, "Insert", true) {
                    menu.submenu = Some(SubMenu::Insert);
                }
                if row(ui, regular::CARET_DOWN, "Fold", true) {
                    menu.submenu = Some(SubMenu::Fold);
                }
                if row(ui, regular::EYE, "View", true) {
                    menu.submenu = Some(SubMenu::View);
                }

                ui.separator();

                ui.add_enabled_ui(has_selection, |ui| {
                    if row(ui, regular::SCISSORS, "Cut", false) {
                        action = Some(EditorAction::Cut);
                    }
                    if row(ui, regular::COPY, "Copy", false) {
                        action = Some(EditorAction::Copy);
                    }
                });
                if row(ui, regular::CLIPBOARD, "Paste", false) {
                    action = Some(EditorAction::Paste);
                }
                if row(ui, regular::CLIPBOARD_TEXT, "Paste as plain text", false) {
                    action = Some(EditorAction::PastePlain);
                }

                ui.separator();

                if row(ui, regular::SELECTION_ALL, "Select all", false) {
                    action = Some(EditorAction::SelectAll);
                }
            });
        });
    hit_rects.push(main.response.rect);

    if let Some(sub) = menu.submenu {
        let sub_pos = Pos2::new(main.response.rect.right() + 4.0, main.response.rect.top());
        let sub_area = egui::Area::new(Id::new("bluelink_editor_ctx_sub"))
            .order(egui::Order::Foreground)
            .fixed_pos(sub_pos)
            .sense(egui::Sense::hover())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match sub {
                        SubMenu::Format => {
                            if row(ui, regular::TEXT_B, "Bold", false) {
                                action = Some(EditorAction::Wrap {
                                    before: "**",
                                    after: "**",
                                });
                            }
                            if row(ui, regular::TEXT_ITALIC, "Italic", false) {
                                action = Some(EditorAction::Wrap {
                                    before: "*",
                                    after: "*",
                                });
                            }
                            if row(ui, regular::CODE, "Code", false) {
                                action = Some(EditorAction::Wrap {
                                    before: "`",
                                    after: "`",
                                });
                            }
                            if row(ui, regular::TEXT_STRIKETHROUGH, "Strikethrough", false) {
                                action = Some(EditorAction::Wrap {
                                    before: "~~",
                                    after: "~~",
                                });
                            }
                            if row(ui, regular::HIGHLIGHTER, "Highlight", false) {
                                action = Some(EditorAction::Wrap {
                                    before: "==",
                                    after: "==",
                                });
                            }
                            if row(ui, regular::CHAT_CIRCLE, "Toggle comments", false) {
                                action = Some(EditorAction::ToggleComments);
                            }
                            if row(ui, regular::MATH_OPERATIONS, "Inline math", false) {
                                action = Some(EditorAction::ToggleInlineMath);
                            }
                            ui.add_enabled_ui(has_selection, |ui| {
                                if row(ui, regular::ERASER, "Clear formatting", false) {
                                    action = Some(EditorAction::ClearFormatting);
                                }
                            });
                        }
                        SubMenu::Paragraph => {
                            if row(ui, regular::TEXT_T, "Plain text", false) {
                                action = Some(EditorAction::SetLinePrefix(""));
                            }
                            if row(ui, regular::TEXT_H_ONE, "Heading 1", false) {
                                action = Some(EditorAction::SetLinePrefix("# "));
                            }
                            if row(ui, regular::TEXT_H_TWO, "Heading 2", false) {
                                action = Some(EditorAction::SetLinePrefix("## "));
                            }
                            if row(ui, regular::TEXT_H_THREE, "Heading 3", false) {
                                action = Some(EditorAction::SetLinePrefix("### "));
                            }
                            if row(ui, regular::TEXT_H_FOUR, "Heading 4", false) {
                                action = Some(EditorAction::SetLinePrefix("#### "));
                            }
                            if row(ui, regular::TEXT_H_FIVE, "Heading 5", false) {
                                action = Some(EditorAction::SetLinePrefix("##### "));
                            }
                            if row(ui, regular::TEXT_H_SIX, "Heading 6", false) {
                                action = Some(EditorAction::SetLinePrefix("###### "));
                            }
                            if row(ui, regular::TEXT_AA, "Quote", false) {
                                action = Some(EditorAction::SetLinePrefix("> "));
                            }
                            if row(ui, regular::LIST_BULLETS, "Bullet list", false) {
                                action = Some(EditorAction::SetLinePrefix("- "));
                            }
                            if row(ui, regular::LIST_NUMBERS, "Numbered list", false) {
                                action = Some(EditorAction::SetLinePrefix("1. "));
                            }
                            if row(ui, regular::TEXT_INDENT, "Indent list", false) {
                                action = Some(EditorAction::IndentList);
                            }
                            if row(ui, regular::TEXT_OUTDENT, "Unindent list", false) {
                                action = Some(EditorAction::UnindentList);
                            }
                        }
                        SubMenu::Insert => {
                            if row(ui, regular::HASH, "Tag", false) {
                                action = Some(EditorAction::InsertTag);
                            }
                            if row(ui, regular::SQUARE, "Embed", false) {
                                action = Some(EditorAction::InsertEmbed);
                            }
                            if row(ui, regular::ARTICLE, "Footnote", false) {
                                action = Some(EditorAction::Footnote);
                            }
                            if row(ui, regular::TABLE, "Table", false) {
                                action = Some(EditorAction::InsertSnippet(crate::table::SNIPPET));
                            }
                            if row(ui, regular::QUOTES, "Callout", false) {
                                action =
                                    Some(EditorAction::InsertSnippet(crate::preview::CALLOUT_SNIPPET));
                            }
                            if row(ui, regular::MINUS, "Horizontal rule", false) {
                                action = Some(EditorAction::InsertSnippet("---"));
                            }
                            if row(ui, regular::CODE_BLOCK, "Code block", false) {
                                action = Some(EditorAction::InsertSnippet("```\n\n```"));
                            }
                            if row(ui, regular::MATH_OPERATIONS, "Math block", false) {
                                action =
                                    Some(EditorAction::InsertSnippet(crate::math_block::SNIPPET));
                            }
                            if row(ui, regular::PAPERCLIP, "Attach file…", false) {
                                action = Some(EditorAction::AttachFile);
                            }
                        }
                        SubMenu::Fold => {
                            if row(ui, regular::CARET_RIGHT, "Fold", false) {
                                action = Some(EditorAction::FoldMore);
                            }
                            if row(ui, regular::CARET_DOWN, "Unfold", false) {
                                action = Some(EditorAction::FoldLess);
                            }
                            ui.separator();
                            if row(ui, regular::CARET_DOUBLE_UP, "Fold all", false) {
                                action = Some(EditorAction::FoldAll);
                            }
                            if row(ui, regular::CARET_DOUBLE_DOWN, "Unfold all", false) {
                                action = Some(EditorAction::UnfoldAll);
                            }
                            ui.separator();
                            if row(ui, regular::SIDEBAR, "Fold properties", false) {
                                action = Some(EditorAction::ToggleFoldProperties);
                            }
                        }
                        SubMenu::View => {
                            if row(ui, regular::BOOK_OPEN_TEXT, "Toggle reading view", false) {
                                action = Some(EditorAction::ToggleReading);
                            }
                            if row(ui, regular::CODE, "Toggle source", false) {
                                action = Some(EditorAction::ToggleSource);
                            }
                        }
                    }
                });
            });
        hit_rects.push(sub_area.response.rect);
    }

    // Persist submenu open state when no action yet.
    if action.is_none() {
        store_menu(ctx, Some(menu.clone()));
    }

    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let click_outside = ctx.input(|i| {
        i.pointer.primary_clicked()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| hit_rects.iter().all(|r| !r.contains(p)))
    });

    // Never discard a chosen action — click_outside can race with button clicks.
    if action.is_some() {
        close(ctx);
        return action.map(|a| (menu, a));
    }

    if escape || click_outside {
        close(ctx);
    }

    None
}

fn row(ui: &mut Ui, icon: &str, label: &str, has_sub: bool) -> bool {
    let text = if has_sub {
        format!("{icon}  {label}          ›")
    } else {
        format!("{icon}  {label}")
    };
    ui.add_sized(
        Vec2::new(ui.available_width(), ui.spacing().interact_size.y),
        egui::Button::new(text),
    )
    .clicked()
}

pub fn dispatch_clipboard(ui: &Ui, action: &EditorAction) {
    let cmd = match action {
        EditorAction::Cut => Some(ViewportCommand::RequestCut),
        EditorAction::Copy => Some(ViewportCommand::RequestCopy),
        // Paste and PastePlain both request OS text paste; image→embed is only on Paste.
        EditorAction::Paste | EditorAction::PastePlain => Some(ViewportCommand::RequestPaste),
        _ => None,
    };
    if let Some(cmd) = cmd {
        ui.ctx().send_viewport_cmd(cmd);
    }
}

pub fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

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

/// If `s` is wrapped in outer `%%…%%`, return the inner body.
fn strip_outer_comment(s: &str) -> Option<&str> {
    if s.len() >= 4 && s.starts_with("%%") && s.ends_with("%%") {
        Some(&s[2..s.len() - 2])
    } else {
        None
    }
}

/// Strip inline emphasis/code/strike/highlight markers from `s` (not headings/lists/wiki/`$`).
pub fn strip_inline_markers(s: &str) -> String {
    let mut result = s.to_string();
    // Longest first so `**` wins over `*`.
    const DELIMS: &[&str] = &["**", "~~", "==", "`", "*"];
    loop {
        let before = result.clone();
        for d in DELIMS {
            if let Some(inner) = strip_outer_delim(&result, d) {
                result = inner.to_string();
            }
        }
        for d in DELIMS {
            result = strip_paired_delims(&result, d);
        }
        if result == before {
            break;
        }
    }
    result
}

fn strip_outer_delim<'a>(s: &'a str, delim: &str) -> Option<&'a str> {
    if s.len() >= delim.len() * 2 && s.starts_with(delim) && s.ends_with(delim) {
        // Avoid treating `**x**` as outer `*` wrap.
        if delim == "*" && (s.starts_with("**") || s.ends_with("**")) {
            return None;
        }
        Some(&s[delim.len()..s.len() - delim.len()])
    } else {
        None
    }
}

fn strip_paired_delims(s: &str, delim: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = find_delim_start(rest, delim) {
        out.push_str(&rest[..i]);
        rest = &rest[i + delim.len()..];
        if let Some(j) = find_delim_start(rest, delim) {
            out.push_str(&rest[..j]);
            rest = &rest[j + delim.len()..];
        } else {
            out.push_str(delim);
            break;
        }
    }
    out.push_str(rest);
    out
}

fn find_delim_start(s: &str, delim: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = s[start..].find(delim) {
        let i = start + rel;
        if delim == "*" && s[i..].starts_with("**") {
            start = i + 2;
            continue;
        }
        return Some(i);
    }
    None
}

/// Clear inline markers in the selection. No-op if selection is empty.
pub fn clear_formatting(text: &mut String, primary: usize, secondary: usize) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    if lo == hi {
        return primary;
    }
    let byte_lo = char_to_byte(text, lo);
    let byte_hi = char_to_byte(text, hi);
    let cleaned = strip_inline_markers(&text[byte_lo..byte_hi]);
    text.replace_range(byte_lo..byte_hi, &cleaned);
    lo + cleaned.chars().count()
}

/// If `s` is a single `$…$` (not `$$`), return the inner body.
fn strip_outer_inline_math(s: &str) -> Option<&str> {
    if s.starts_with("$$") || s.ends_with("$$") {
        return None;
    }
    if s.len() >= 2 && s.starts_with('$') && s.ends_with('$') {
        let inner = &s[1..s.len() - 1];
        if !inner.contains('$') {
            return Some(inner);
        }
    }
    None
}

fn word_char_bounds(text: &str, caret: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let i = caret.min(chars.len());
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut lo = i;
    while lo > 0 && is_word(chars[lo - 1]) {
        lo -= 1;
    }
    let mut hi = i;
    while hi < chars.len() && is_word(chars[hi]) {
        hi += 1;
    }
    (lo, hi)
}

/// Toggle `$…$` on the selection, word under caret, or insert an empty pair.
pub fn toggle_inline_math(text: &mut String, primary: usize, secondary: usize) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    if lo != hi {
        let byte_lo = char_to_byte(text, lo);
        let byte_hi = char_to_byte(text, hi);
        let selected = text[byte_lo..byte_hi].to_string();
        if selected.starts_with("$$") {
            return hi;
        }
        if let Some(inner) = strip_outer_inline_math(&selected) {
            text.replace_range(byte_lo..byte_hi, inner);
            lo + inner.chars().count()
        } else {
            let replacement = format!("${selected}$");
            text.replace_range(byte_lo..byte_hi, &replacement);
            lo + 1 + selected.chars().count()
        }
    } else {
        let (wlo, whi) = word_char_bounds(text, primary);
        if wlo < whi {
            toggle_inline_math(text, wlo, whi)
        } else {
            wrap_selection(text, primary, primary, "$", "$")
        }
    }
}

/// Toggle `%%…%%` on the selection, or on the caret's line when the selection is empty.
pub fn toggle_comments(text: &mut String, primary: usize, secondary: usize) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    if lo != hi {
        let byte_lo = char_to_byte(text, lo);
        let byte_hi = char_to_byte(text, hi);
        let selected = text[byte_lo..byte_hi].to_string();
        if let Some(inner) = strip_outer_comment(&selected) {
            text.replace_range(byte_lo..byte_hi, inner);
            lo + inner.chars().count()
        } else {
            let replacement = format!("%%{selected}%%");
            text.replace_range(byte_lo..byte_hi, &replacement);
            lo + "%%".chars().count() + selected.chars().count()
        }
    } else {
        let byte = char_to_byte(text, primary);
        let line_start = text[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = text[byte..]
            .find('\n')
            .map(|i| byte + i)
            .unwrap_or(text.len());
        let line = text[line_start..line_end].to_string();
        let trimmed = line.trim();
        let lead = line.len() - line.trim_start().len();
        let trail = line.len() - line.trim_end().len();
        let new_line = if let Some(inner) = strip_outer_comment(trimmed) {
            format!("{}{}{}", &line[..lead], inner, &line[line.len() - trail..])
        } else {
            format!("%%{line}%%")
        };
        text.replace_range(line_start..line_end, &new_line);
        let start_chars = text[..line_start].chars().count();
        start_chars + new_line.chars().count()
    }
}

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

pub fn set_line_prefix(line: &str, prefix: &str) -> String {
    let body = strip_line_prefix(line);
    if prefix.is_empty() {
        body
    } else {
        format!("{prefix}{body}")
    }
}

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

/// Next `[^N]` is one past the highest marker or definition already in `text`.
fn next_footnote_number(text: &str) -> u32 {
    let bytes = text.as_bytes();
    let mut max_n = 0u32;
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'^' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let closed = j > start && j < bytes.len() && bytes[j] == b']';
            if closed {
                if let Ok(n) = text[start..j].parse::<u32>() {
                    max_n = max_n.max(n);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    max_n.saturating_add(1)
}

/// Insert `[^N]` in place of the selection and a blank definition at the end.
/// Caret sits after `[^N]: ` so the note text can be typed immediately.
fn insert_footnote(text: &mut String, primary: usize, secondary: usize) -> usize {
    let n = next_footnote_number(text);
    let marker = format!("[^{n}]");
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    let byte_lo = char_to_byte(text, lo);
    let byte_hi = char_to_byte(text, hi);
    text.replace_range(byte_lo..byte_hi, &marker);

    if !text.ends_with('\n') {
        text.push('\n');
    }
    // A blank line separates the body from the definition.
    if !text.ends_with("\n\n") {
        text.push('\n');
    }
    text.push_str(&format!("[^{n}]: "));
    text.chars().count()
}

pub fn insert_at(text: &mut String, primary: usize, secondary: usize, snippet: &str) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    let byte_lo = char_to_byte(text, lo);
    let byte_hi = char_to_byte(text, hi);
    text.replace_range(byte_lo..byte_hi, snippet);
    lo + snippet.chars().count()
}

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
        EditorAction::InsertTag => {
            let lo = primary.min(secondary);
            let hi = primary.max(secondary);
            if lo == hi {
                Some(insert_at(text, primary, secondary, "#"))
            } else {
                let byte_lo = char_to_byte(text, lo);
                text.insert(byte_lo, '#');
                Some(hi + 1)
            }
        }
        EditorAction::InsertEmbed => {
            let caret = insert_at(text, primary, secondary, "![[]]");
            Some(caret - "]]".chars().count())
        }
        EditorAction::ExternalLink => {
            let lo = primary.min(secondary);
            let hi = primary.max(secondary);
            if lo == hi {
                let caret = insert_at(text, primary, secondary, "[](https://)");
                Some(caret - ")".chars().count())
            } else {
                let byte_lo = char_to_byte(text, lo);
                let byte_hi = char_to_byte(text, hi);
                let selected = text[byte_lo..byte_hi].to_string();
                let replacement = format!("[{selected}](https://)");
                text.replace_range(byte_lo..byte_hi, &replacement);
                Some(lo + selected.chars().count() + "](https://".chars().count())
            }
        }
        EditorAction::Wrap { before, after } => {
            Some(wrap_selection(text, primary, secondary, before, after))
        }
        EditorAction::ToggleComments => Some(toggle_comments(text, primary, secondary)),
        EditorAction::ClearFormatting => Some(clear_formatting(text, primary, secondary)),
        EditorAction::ToggleInlineMath => Some(toggle_inline_math(text, primary, secondary)),
        EditorAction::IndentList | EditorAction::UnindentList => {
            let deeper = matches!(action, EditorAction::IndentList);
            let (next, new_p, new_s) =
                crate::preview::adjust_list_indent_in_range(text, primary, secondary, deeper)?;
            *text = next;
            Some(if primary <= secondary { new_p } else { new_s })
        }
        EditorAction::SetLinePrefix(prefix) => Some(set_prefix_at_char(text, primary, prefix)),
        EditorAction::InsertSnippet(snippet) => Some(insert_at(text, primary, secondary, snippet)),
        EditorAction::Footnote => Some(insert_footnote(text, primary, secondary)),
        EditorAction::Cut
        | EditorAction::Copy
        | EditorAction::Paste
        | EditorAction::PastePlain
        | EditorAction::SelectAll
        | EditorAction::AttachFile
        | EditorAction::FoldMore
        | EditorAction::FoldLess
        | EditorAction::FoldAll
        | EditorAction::UnfoldAll
        | EditorAction::ToggleFoldProperties
        | EditorAction::ToggleReading
        | EditorAction::ToggleSource => None,
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
        assert_eq!(caret, 7);
    }

    #[test]
    fn wrap_at_caret_inserts_empty_pair() {
        let mut s = "ab".to_string();
        let caret = wrap_selection(&mut s, 1, 1, "`", "`");
        assert_eq!(s, "a``b");
        assert_eq!(caret, 2);
    }

    #[test]
    fn toggle_comments_wraps_and_unwraps_selection() {
        let mut s = "hello world".to_string();
        let caret = toggle_comments(&mut s, 0, 5);
        assert_eq!(s, "%%hello%% world");
        assert_eq!(caret, 7);
        let caret = toggle_comments(&mut s, 0, 9);
        assert_eq!(s, "hello world");
        assert_eq!(caret, 5);
    }

    #[test]
    fn toggle_comments_wraps_and_unwraps_line() {
        let mut s = "a\nsecret\nc".to_string();
        // caret on "secret"
        let caret = toggle_comments(&mut s, 3, 3);
        assert_eq!(s, "a\n%%secret%%\nc");
        assert_eq!(&s[char_to_byte(&s, caret)..], "\nc");
        let caret = toggle_comments(&mut s, 5, 5);
        assert_eq!(s, "a\nsecret\nc");
        assert_eq!(&s[char_to_byte(&s, caret)..], "\nc");
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

    #[test]
    fn footnotes_number_in_order() {
        let mut s = String::new();
        let caret = apply_text_action(&mut s, 0, 0, &EditorAction::Footnote).unwrap();
        assert_eq!(s, "[^1]\n\n[^1]: ");
        assert_eq!(caret, s.chars().count());

        let caret = apply_text_action(&mut s, 0, 0, &EditorAction::Footnote).unwrap();
        assert_eq!(s, "[^2][^1]\n\n[^1]: \n\n[^2]: ");
        assert_eq!(caret, s.chars().count());
    }

    #[test]
    fn insert_tag_at_caret_or_before_selection() {
        let mut s = "hello".to_string();
        let caret = apply_text_action(&mut s, 0, 0, &EditorAction::InsertTag).unwrap();
        assert_eq!(s, "#hello");
        assert_eq!(caret, 1);

        let mut s = "hello world".to_string();
        let caret = apply_text_action(&mut s, 0, 5, &EditorAction::InsertTag).unwrap();
        assert_eq!(s, "#hello world");
        assert_eq!(caret, 6);
        assert_eq!(&s[..caret], "#hello");
    }

    #[test]
    fn insert_embed_places_caret_inside() {
        let mut s = String::new();
        let caret = apply_text_action(&mut s, 0, 0, &EditorAction::InsertEmbed).unwrap();
        assert_eq!(s, "![[]]");
        assert_eq!(&s[..caret], "![[");
        assert_eq!(&s[caret..], "]]");
    }

    #[test]
    fn clear_formatting_strips_inline_markers() {
        let mut s = "**bold** and more".to_string();
        let caret = clear_formatting(&mut s, 0, 8);
        assert_eq!(s, "bold and more");
        assert_eq!(caret, 4);

        let mut s = "`code` and ~~x~~".to_string();
        let end = s.chars().count();
        let caret = clear_formatting(&mut s, 0, end);
        assert_eq!(s, "code and x");
        assert_eq!(caret, s.chars().count());
    }

    #[test]
    fn toggle_inline_math_wraps_and_unwraps() {
        let mut s = "a^2".to_string();
        let caret = toggle_inline_math(&mut s, 0, 3);
        assert_eq!(s, "$a^2$");
        assert_eq!(caret, 4);
        let caret = toggle_inline_math(&mut s, 0, 5);
        assert_eq!(s, "a^2");
        assert_eq!(caret, 3);

        // Block $$ left alone when selected as block opener.
        let mut s = "$$\nx\n$$".to_string();
        let before = s.clone();
        let caret = toggle_inline_math(&mut s, 0, 2);
        assert_eq!(s, before);
        assert_eq!(caret, 2);
    }
}
