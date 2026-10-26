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
    Wrap {
        before: &'static str,
        after: &'static str,
    },
    SetLinePrefix(&'static str),
    InsertSnippet(&'static str),
    /// Host opens a file picker and inserts `![[attachments/…]]`.
    AttachFile,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubMenu {
    Format,
    Paragraph,
    Insert,
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
                    action = Some(EditorAction::Paste);
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
                            if row(ui, regular::TEXT_AA, "Quote", false) {
                                action = Some(EditorAction::SetLinePrefix("> "));
                            }
                            if row(ui, regular::LIST_BULLETS, "Bullet list", false) {
                                action = Some(EditorAction::SetLinePrefix("- "));
                            }
                            if row(ui, regular::LIST_NUMBERS, "Numbered list", false) {
                                action = Some(EditorAction::SetLinePrefix("1. "));
                            }
                        }
                        SubMenu::Insert => {
                            if row(ui, regular::PAPERCLIP, "Attach file…", false) {
                                action = Some(EditorAction::AttachFile);
                            }
                            if row(ui, regular::CODE_BLOCK, "Code block", false) {
                                action = Some(EditorAction::InsertSnippet("```\n\n```"));
                            }
                            if row(ui, regular::MINUS, "Divider", false) {
                                action = Some(EditorAction::InsertSnippet("---"));
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
        EditorAction::Paste => Some(ViewportCommand::RequestPaste),
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
        EditorAction::SetLinePrefix(prefix) => Some(set_prefix_at_char(text, primary, prefix)),
        EditorAction::InsertSnippet(snippet) => {
            Some(insert_at(text, primary, secondary, snippet))
        }
        EditorAction::Cut
        | EditorAction::Copy
        | EditorAction::Paste
        | EditorAction::SelectAll
        | EditorAction::AttachFile => None,
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
}
