use bluelink_core::{
    fragment_line_range, link_fragment, parse_preview, split_wikilinks, PreviewBlock, PreviewDoc,
    Span, Vault,
};
use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, OpenUrl, RichText, Sense, Ui};
use egui_phosphor::regular;

use crate::math_block;
use crate::media;
use crate::theme;

/// Role of a source line inside a fenced code block (``` / ~~~).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceRole {
    Outside,
    Open,
    Body,
    Close,
}

/// Role of a source line inside a standalone Obsidian block comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentRole {
    Outside,
    Open,
    Body,
    Close,
}

/// Update standalone `%%` block-comment state for `line`.
///
/// Inline `before %%comment%% after` is deliberately left to the Markdown span parser.
pub fn advance_comment(line: &str, open: &mut bool) -> CommentRole {
    if *open {
        if line.trim() == "%%" {
            *open = false;
            CommentRole::Close
        } else {
            CommentRole::Body
        }
    } else if line.trim() == "%%" {
        *open = true;
        CommentRole::Open
    } else {
        CommentRole::Outside
    }
}

/// Update fence state for `line` and return how it should be painted.
pub fn advance_fence(line: &str, open: &mut Option<String>) -> FenceRole {
    let trimmed = line.trim_start();
    if let Some(marker) = open.as_deref() {
        if is_closing_fence(trimmed, marker) {
            *open = None;
            return FenceRole::Close;
        }
        return FenceRole::Body;
    }
    if let Some(marker) = opening_fence_marker(trimmed) {
        *open = Some(marker.to_string());
        return FenceRole::Open;
    }
    FenceRole::Outside
}

fn opening_fence_marker(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_closing_fence(trimmed: &str, marker: &str) -> bool {
    let ch = marker.chars().next().unwrap_or('`');
    let n = trimmed.chars().take_while(|c| *c == ch).count();
    n >= marker.chars().count() && trimmed[n..].trim().is_empty()
}

/// Font for the raw line editor — same size/face as the preview of that line,
/// so activating a row doesn't jump in height.
pub fn edit_font(line: &str) -> FontId {
    edit_font_for(line, FenceRole::Outside)
}

pub fn edit_font_for(line: &str, role: FenceRole) -> FontId {
    if !matches!(role, FenceRole::Outside) {
        return FontId::new(16.0, FontFamily::Monospace);
    }
    let doc = parse_preview(line);
    match doc.blocks.first() {
        Some(PreviewBlock::Heading { level, .. }) => {
            FontId::new(theme::heading_size(*level), theme::heading_family(*level))
        }
        Some(PreviewBlock::CodeBlock { .. }) => FontId::new(16.0, FontFamily::Monospace),
        _ => FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
    }
}

/// What a click on a preview line means.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LineClick {
    #[default]
    None,
    /// Open / create this wikilink target (raw body inside `[[...]]`).
    /// `new_tab` is set when Mod (Ctrl/Cmd) was held on click.
    Wiki { raw: String, new_tab: bool },
    /// Enter raw edit for this line.
    Edit,
    /// Toggle `- [ ]` / `- [x]` without opening the line.
    ToggleCheck,
    /// Host should open quick-switcher with this tag text (`#tag` without `#`).
    Tag { query: String },
    /// Jump to the definition of `[^id]`.
    Footnote { id: String },
    /// Drag-resize finished: rewrite `![[target|…]]` size in the source buffer.
    ResizeEmbed {
        target: String,
        width: u32,
        height: Option<u32>,
    },
}

/// Checklist marker on a source line (`- [ ]` / `* [x]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMark {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChecklistPrefix<'a> {
    Bullet(&'a str),
    Numbered { marker: &'a str },
}

impl<'a> ChecklistPrefix<'a> {
    fn numbered_label(self) -> Option<&'a str> {
        match self {
            Self::Numbered { marker, .. } => Some(marker.trim_end()),
            Self::Bullet(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChecklistLine<'a> {
    prefix: ChecklistPrefix<'a>,
    mark: CheckMark,
    body: &'a str,
    checkbox_start: usize,
}

fn parse_checklist_line(line: &str) -> Option<ChecklistLine<'_>> {
    let trimmed = line.trim_start();
    let (prefix, after_marker) = if let Some(rest) = trimmed.strip_prefix("- ") {
        (ChecklistPrefix::Bullet("- "), rest)
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        (ChecklistPrefix::Bullet("* "), rest)
    } else if let Some(rest) = trimmed.strip_prefix("+ ") {
        (ChecklistPrefix::Bullet("+ "), rest)
    } else {
        let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
        let rest = numbered_list_rest(trimmed)?;
        (
            ChecklistPrefix::Numbered {
                marker: &trimmed[..digits + 2],
            },
            rest,
        )
    };
    let checkbox_start = line.len() - after_marker.len();
    let (mark, body) = if let Some(body) = after_marker.strip_prefix("[ ]") {
        (CheckMark::Unchecked, body)
    } else if let Some(body) = after_marker
        .strip_prefix("[x]")
        .or_else(|| after_marker.strip_prefix("[X]"))
    {
        (CheckMark::Checked, body)
    } else {
        return None;
    };
    Some(ChecklistLine {
        prefix,
        mark,
        body: body.strip_prefix(' ').unwrap_or(body),
        checkbox_start,
    })
}

/// `Some((mark, body))` when the line is a task item.
pub fn parse_checklist(line: &str) -> Option<(CheckMark, &str)> {
    let parsed = parse_checklist_line(line)?;
    Some((parsed.mark, parsed.body))
}

/// Flip `[ ]` ↔ `[x]` on a checklist line. Returns `None` when not a task.
pub fn toggle_checklist_line(line: &str) -> Option<String> {
    let parsed = parse_checklist_line(line)?;
    let box_ = match parsed.mark {
        CheckMark::Unchecked => "[x]",
        CheckMark::Checked => "[ ]",
    };
    let mut next = line.to_string();
    next.replace_range(parsed.checkbox_start..parsed.checkbox_start + 3, box_);
    Some(next)
}

/// Default Insert → Callout snippet (`> [!note]` + empty body line).
pub const CALLOUT_SNIPPET: &str = "> [!note]\n> ";

/// Canonical Obsidian callout kinds (aliases resolve here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Abstract,
    Info,
    Todo,
    Tip,
    Success,
    Question,
    Warning,
    Failure,
    Danger,
    Bug,
    Example,
    Quote,
}

impl CalloutKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Abstract => "ABSTRACT",
            Self::Info => "INFO",
            Self::Todo => "TODO",
            Self::Tip => "TIP",
            Self::Success => "SUCCESS",
            Self::Question => "QUESTION",
            Self::Warning => "WARNING",
            Self::Failure => "FAILURE",
            Self::Danger => "DANGER",
            Self::Bug => "BUG",
            Self::Example => "EXAMPLE",
            Self::Quote => "QUOTE",
        }
    }
}

/// Border / fill / phosphor icon for a callout card.
#[derive(Clone, Copy, Debug)]
pub struct CalloutVisual {
    pub accent: Color32,
    pub fill: Color32,
    pub icon: &'static str,
}

fn callout_fill(accent: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha)
}

/// Map raw `[!type]` (case-insensitive) to a kind. Second value is `true` when the
/// type is unknown and should fall back to note styling while keeping the raw label.
pub fn callout_kind(raw: &str) -> (CalloutKind, bool) {
    let key = raw.to_ascii_lowercase();
    let kind = match key.as_str() {
        "note" => CalloutKind::Note,
        "abstract" | "summary" | "tldr" => CalloutKind::Abstract,
        "info" => CalloutKind::Info,
        "todo" => CalloutKind::Todo,
        "tip" | "hint" | "important" => CalloutKind::Tip,
        "success" | "check" | "done" => CalloutKind::Success,
        "question" | "help" | "faq" => CalloutKind::Question,
        "warning" | "caution" | "attention" => CalloutKind::Warning,
        "failure" | "fail" | "missing" => CalloutKind::Failure,
        "danger" | "error" => CalloutKind::Danger,
        "bug" => CalloutKind::Bug,
        "example" => CalloutKind::Example,
        "quote" | "cite" => CalloutKind::Quote,
        _ => return (CalloutKind::Note, true),
    };
    (kind, false)
}

pub fn callout_visual(kind: CalloutKind) -> CalloutVisual {
    let (accent, icon, alpha) = match kind {
        CalloutKind::Note => (theme::ACCENT, regular::NOTE, 48),
        CalloutKind::Abstract => (
            Color32::from_rgb(0x00, 0xbf, 0xa5),
            regular::BOOK_OPEN_TEXT,
            48,
        ),
        CalloutKind::Info => (theme::ACCENT, regular::INFO, 48),
        CalloutKind::Todo => (theme::ACCENT, regular::CHECK_SQUARE, 48),
        CalloutKind::Tip => (Color32::from_rgb(0x00, 0xb5, 0xe2), regular::LIGHTBULB, 48),
        CalloutKind::Success => (theme::SUCCESS, regular::CHECK_CIRCLE, 48),
        CalloutKind::Question => (theme::WARN, regular::QUESTION, 48),
        CalloutKind::Warning => (theme::WARN, regular::WARNING, 48),
        CalloutKind::Failure => (theme::DESTRUCTIVE, regular::X_CIRCLE, 48),
        CalloutKind::Danger => (theme::DESTRUCTIVE, regular::FIRE, 72),
        CalloutKind::Bug => (theme::DESTRUCTIVE, regular::BUG, 48),
        CalloutKind::Example => (theme::ACCENT_HOVER, regular::FLASK, 48),
        CalloutKind::Quote => (theme::WEAK, regular::QUOTES, 40),
    };
    CalloutVisual {
        accent,
        fill: callout_fill(accent, alpha),
        icon,
    }
}

/// Obsidian callout: `> [!type] title` plus following `>` lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutPlace {
    None,
    /// First line; `end` exclusive.
    Head {
        end: usize,
    },
    Tail {
        start: usize,
        end: usize,
    },
}

/// Map each line to callout head/tail. Lines inside a code fence are never callouts.
pub fn callout_places(lines: &[String]) -> Vec<CalloutPlace> {
    let mut out = vec![CalloutPlace::None; lines.len()];
    let mut i = 0;
    let mut fence = None;
    while i < lines.len() {
        let in_fence = !matches!(advance_fence(&lines[i], &mut fence), FenceRole::Outside);
        if !in_fence && callout_type(&lines[i]).is_some() {
            let mut end = i + 1;
            while end < lines.len() && is_quote_line(&lines[end]) {
                let _ = advance_fence(&lines[end], &mut fence);
                end += 1;
            }
            out[i] = CalloutPlace::Head { end };
            for j in (i + 1)..end {
                out[j] = CalloutPlace::Tail { start: i, end };
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// `Some(type)` for `> [!type]…` (unknown types still count).
pub fn callout_type(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let t = t.strip_prefix('>')?.trim_start();
    let t = t.strip_prefix("[!")?;
    let close = t.find(']')?;
    let ty = &t[..close];
    if ty.is_empty()
        || !ty
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(ty)
}

/// `true` when the callout header ends the type with `-` (`> [!note]-`).
pub fn callout_collapsed(line: &str) -> bool {
    marker_after_type(line) == Some('-')
}

/// Rewrite the `+`/`-` fold marker. `None` when the line is already in that state
/// or is not a callout. Collapsing inserts `-` when the marker is absent.
/// Expanding turns `-` into `+` and leaves an unmarked callout alone.
pub fn set_callout_collapsed(line: &str, collapsed: bool) -> Option<String> {
    let bracket = type_bracket(line)?;
    let want = if collapsed { '-' } else { '+' };
    let next = line[bracket + 1..].chars().next();
    match next {
        Some(marker) if marker == want && (marker == '-' || marker == '+') => None,
        Some('-') | Some('+') => {
            let mut rewritten = line.to_string();
            let replacement = if collapsed { "-" } else { "+" };
            rewritten.replace_range(bracket + 1..bracket + 2, replacement);
            Some(rewritten)
        }
        _ if collapsed => {
            let mut rewritten = line.to_string();
            rewritten.insert(bracket + 1, '-');
            Some(rewritten)
        }
        _ => None,
    }
}

/// Apply [`set_callout_collapsed`] to every callout header. `None` if nothing changed.
/// Lines inside a code fence are left alone.
pub fn set_all_callouts_collapsed(text: &str, collapsed: bool) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    let mut fence = None;
    for piece in text.split_inclusive('\n') {
        let (line, newline) = match piece.strip_suffix('\n') {
            Some(body) => (body, true),
            None => (piece, false),
        };
        let in_fence = !matches!(advance_fence(line, &mut fence), FenceRole::Outside);
        if !in_fence {
            if let Some(next) = set_callout_collapsed(line, collapsed) {
                changed = true;
                out.push_str(&next);
                if newline {
                    out.push('\n');
                }
                continue;
            }
        }
        out.push_str(line);
        if newline {
            out.push('\n');
        }
    }
    changed.then_some(out)
}

/// `(header, end)` of the callout that contains `line`, header included.
/// A callout-shaped line inside a code fence is not a callout.
pub fn callout_at(lines: &[&str], line: usize) -> Option<(usize, usize)> {
    let mut i = 0;
    let mut fence = None;
    while i < lines.len() {
        let in_fence = !matches!(advance_fence(lines[i], &mut fence), FenceRole::Outside);
        if !in_fence && callout_type(lines[i]).is_some() {
            let mut end = i + 1;
            while end < lines.len() && lines[end].trim_start().starts_with('>') {
                let _ = advance_fence(lines[end], &mut fence);
                end += 1;
            }
            if (i..end).contains(&line) {
                return Some((i, end));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

/// Body lines of callouts whose header marker is `-`.
/// Fence lines are never hidden by this.
pub fn collapsed_callout_tails(lines: &[&str]) -> Vec<bool> {
    let mut hidden = vec![false; lines.len()];
    let mut i = 0;
    let mut fence = None;
    while i < lines.len() {
        let in_fence = !matches!(advance_fence(lines[i], &mut fence), FenceRole::Outside);
        if !in_fence && callout_collapsed(lines[i]) {
            let mut end = i + 1;
            while end < lines.len() && lines[end].trim_start().starts_with('>') {
                let _ = advance_fence(lines[end], &mut fence);
                hidden[end] = true;
                end += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    hidden
}

/// Expand a collapsed callout when `line` is in its body. Header clicks stay collapsed.
pub fn expand_collapsed_callout_at(text: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let (header, _) = callout_at(&lines, line)?;
    if header == line || !callout_collapsed(lines[header]) {
        return None;
    }
    let next = set_callout_collapsed(lines[header], false)?;
    let mut owned: Vec<String> = lines.into_iter().map(str::to_string).collect();
    owned[header] = next;
    Some(owned.join("\n"))
}

fn type_bracket(line: &str) -> Option<usize> {
    if callout_type(line).is_none() {
        return None;
    }
    let bang = line.find("[!")?;
    let close = line[bang + 2..].find(']')?;
    Some(bang + 2 + close)
}

fn marker_after_type(line: &str) -> Option<char> {
    let bracket = type_bracket(line)?;
    line[bracket + 1..].chars().next()
}

fn is_quote_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

/// One list indent level (Markdown-typical two spaces).
pub const LIST_INDENT: &str = "  ";

/// Bullet, numbered, or checklist line — same markers Enter continues, excluding quotes.
pub fn is_list_line(line: &str) -> bool {
    if parse_checklist(line).is_some() {
        return true;
    }
    let trimmed = line.trim_start();
    trimmed.starts_with("- ") || trimmed.starts_with("* ") || numbered_list_rest(trimmed).is_some()
}

/// Prepend one [`LIST_INDENT`] level.
pub fn indent_list_line(line: &str) -> String {
    format!("{LIST_INDENT}{line}")
}

/// Remove one leading indent level (`\\t`, 4 spaces, or 2 spaces).
pub fn unindent_list_line(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix('\t') {
        return Some(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix("    ") {
        return Some(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix(LIST_INDENT) {
        return Some(rest.to_string());
    }
    None
}

/// Indent or unindent list lines that intersect the char range. Non-list lines
/// are left alone. `None` when nothing changes.
pub fn adjust_list_indent_in_range(
    text: &str,
    primary: usize,
    secondary: usize,
    indent: bool,
) -> Option<(String, usize, usize)> {
    let sel_lo = primary.min(secondary);
    let sel_hi = primary.max(secondary);
    let mut out = String::with_capacity(text.len() + 32);
    let mut orig_char = 0usize;
    let mut new_lo = sel_lo as i32;
    let mut new_hi = sel_hi as i32;
    let mut changed = false;

    for piece in text.split_inclusive('\n') {
        let (line, has_nl) = match piece.strip_suffix('\n') {
            Some(body) => (body, true),
            None => (piece, false),
        };
        let line_len = line.chars().count();
        let line_start = orig_char;
        let line_content_end = orig_char + line_len;
        let line_span_end = line_content_end + usize::from(has_nl);

        let on_line = if sel_lo == sel_hi {
            if has_nl {
                sel_lo >= line_start && sel_lo < line_span_end
            } else {
                sel_lo >= line_start && sel_lo <= line_content_end
            }
        } else {
            sel_lo < line_span_end && sel_hi > line_start
        };

        let next_line = if on_line && is_list_line(line) {
            if indent {
                Some(indent_list_line(line))
            } else {
                unindent_list_line(line)
            }
        } else {
            None
        };

        match next_line {
            Some(ref new_line) if new_line.as_str() != line => {
                changed = true;
                let delta = new_line.chars().count() as i32 - line_len as i32;
                // Keep an anchor at line_start so a whole-line selection still
                // covers the new leading indent; carets inside the line move.
                if sel_lo > line_start {
                    new_lo += delta;
                }
                if sel_hi > line_start {
                    new_hi += delta;
                }
                out.push_str(new_line);
            }
            _ => out.push_str(line),
        }
        if has_nl {
            out.push('\n');
        }
        orig_char = line_span_end;
    }

    if !changed {
        return None;
    }
    let new_lo = new_lo.max(0) as usize;
    let new_hi = new_hi.max(0) as usize;
    let (np, ns) = if primary <= secondary {
        (new_lo, new_hi)
    } else {
        (new_hi, new_lo)
    };
    Some((out, np, ns))
}

/// List / checklist marker to continue on Enter. Empty body → clear instead.
pub fn continue_list_marker(line: &str) -> Option<ListContinue> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];
    if let Some(parsed) = parse_checklist_line(line) {
        let empty = parsed.body.trim().is_empty();
        let next_marker = match parsed.prefix {
            ChecklistPrefix::Bullet(marker) => marker.to_string(),
            ChecklistPrefix::Numbered { marker } => increment_numbered_marker(marker),
        };
        return Some(ListContinue {
            empty_body: empty,
            next_prefix: format!("{indent}{next_marker}[ ] "),
            clear_to: format!("{indent}"),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some(ListContinue {
            empty_body: rest.trim().is_empty(),
            next_prefix: format!("{indent}- "),
            clear_to: indent.to_string(),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return Some(ListContinue {
            empty_body: rest.trim().is_empty(),
            next_prefix: format!("{indent}* "),
            clear_to: indent.to_string(),
        });
    }
    if let Some(rest) = numbered_list_rest(trimmed) {
        let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
        let next_marker = increment_numbered_marker(&trimmed[..digits + 2]);
        return Some(ListContinue {
            empty_body: rest.trim().is_empty(),
            next_prefix: format!("{indent}{next_marker}"),
            clear_to: indent.to_string(),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("> ") {
        // Don't continue callout headers specially — plain quote.
        if callout_type(line).is_some() {
            return None;
        }
        return Some(ListContinue {
            empty_body: rest.trim().is_empty(),
            next_prefix: format!("{indent}> "),
            clear_to: indent.to_string(),
        });
    }
    None
}

pub struct ListContinue {
    pub empty_body: bool,
    pub next_prefix: String,
    pub clear_to: String,
}

fn numbered_list_rest(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    s[digits..].strip_prefix(". ")
}

/// Increment the ASCII decimal part of a numbered-list marker without a fixed integer width.
fn increment_numbered_marker(marker: &str) -> String {
    let digits = marker.strip_suffix(". ").unwrap_or(marker);
    let mut next = digits.as_bytes().to_vec();
    let mut carry = true;
    for digit in next.iter_mut().rev() {
        if !carry {
            break;
        }
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            carry = false;
        }
    }
    if carry {
        next.insert(0, b'1');
    }
    format!("{}. ", String::from_utf8(next).expect("ASCII digits"))
}

/// Language on an opening fence line (` ```rust ` → `rust`).
pub fn fence_lang(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = if let Some(r) = trimmed.strip_prefix("```") {
        r
    } else if let Some(r) = trimmed.strip_prefix("~~~") {
        r
    } else {
        return None;
    };
    let lang = rest.trim();
    if lang.is_empty() {
        None
    } else {
        Some(lang.to_string())
    }
}

/// Render a line using a preview parsed by the caller.
///
/// The live editor keeps this model between frames so scrolling does not repeatedly
/// allocate and parse the same Markdown line.
pub fn render_line_with_preview(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    line: &str,
    role: FenceRole,
    comment_role: CommentRole,
    lang: Option<&str>,
    preview: Option<&PreviewDoc>,
    // Live: true (muted glyphs). Reading mode (stage 3) can pass false to hide.
    show_comments: bool,
) -> LineClick {
    if !matches!(role, FenceRole::Outside) {
        return render_fence_stripe(ui, line, role, lang);
    }
    if !matches!(comment_role, CommentRole::Outside) {
        return render_block_comment_line(ui, line, show_comments);
    }
    if let Some(checklist) = parse_checklist_line(line) {
        return render_checklist(
            ui,
            vault,
            source_path,
            checklist.prefix.numbered_label(),
            checklist.mark,
            checklist.body,
            show_comments,
        );
    }
    if line.is_empty() {
        let resp = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
        return if resp.1.clicked() {
            LineClick::Edit
        } else {
            LineClick::None
        };
    }
    let owned_preview;
    let doc = match preview {
        Some(doc) => doc,
        None => {
            owned_preview = parse_preview(line);
            &owned_preview
        }
    };
    let mut click = LineClick::None;
    for block in &doc.blocks {
        let c = render_block_compact(ui, vault, source_path, block, show_comments);
        click = merge_click(click, c);
    }
    if doc.blocks.is_empty() {
        click = merge_click(
            click,
            render_text_with_wikis(
                ui,
                vault,
                source_path,
                line,
                theme::EDITOR_SIZE,
                false,
                0,
                false,
            ),
        );
    }
    click
}

fn render_block_comment_line(ui: &mut Ui, line: &str, show_comments: bool) -> LineClick {
    if !show_comments {
        return LineClick::None;
    }
    let resp = ui.add(
        egui::Label::new(
            RichText::new(line)
                .size(theme::EDITOR_SIZE)
                .color(theme::TEXT_FAINT),
        )
        .wrap()
        .sense(Sense::click()),
    );
    if resp.clicked() {
        LineClick::Edit
    } else {
        LineClick::None
    }
}

fn render_checklist(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    number: Option<&str>,
    mark: CheckMark,
    body: &str,
    show_comments: bool,
) -> LineClick {
    let done = matches!(mark, CheckMark::Checked);
    ui.horizontal_top(|ui| {
        let side = 16.0;
        let text_row_height = ui
            .fonts(|fonts| {
                fonts.row_height(&FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional))
            })
            .max(side);
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().interact_size.y = text_row_height;
        let mut click = LineClick::None;
        if let Some(number) = number {
            let number = ui.add(
                egui::Label::new(
                    RichText::new(number)
                        .size(theme::EDITOR_SIZE)
                        .color(theme::TEXT),
                )
                .sense(Sense::click()),
            );
            if number.clicked() {
                click = LineClick::Edit;
            }
        }
        let (hit_rect, resp) =
            ui.allocate_exact_size(egui::vec2(side, text_row_height), Sense::click());
        let rect = egui::Rect::from_center_size(hit_rect.center(), egui::vec2(side, side));
        let stroke = egui::Stroke::new(1.5_f32, theme::ACCENT);
        ui.painter()
            .rect_stroke(rect, theme::round_sm(), stroke, egui::StrokeKind::Inside);
        if done {
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 3.0, rect.center().y),
                    egui::pos2(rect.center().x - 1.0, rect.bottom() - 3.0),
                ],
                egui::Stroke::new(1.8_f32, theme::ACCENT),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.center().x - 1.0, rect.bottom() - 3.0),
                    egui::pos2(rect.right() - 3.0, rect.top() + 3.0),
                ],
                egui::Stroke::new(1.8_f32, theme::ACCENT),
            );
        }
        resp.clone().on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            click = merge_click(click, LineClick::ToggleCheck);
        }
        let body_click = ui
            .vertical(|ui| {
                ui.set_width(ui.available_width().max(8.0));
                if body.is_empty() {
                    let r = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().max(8.0), 22.0),
                        Sense::click(),
                    );
                    if r.1.clicked() {
                        LineClick::Edit
                    } else {
                        LineClick::None
                    }
                } else {
                    let doc = parse_preview(body);
                    let spans = doc
                        .blocks
                        .iter()
                        .find_map(|b| match b {
                            PreviewBlock::Paragraph { spans } => Some(spans.as_slice()),
                            PreviewBlock::BulletItem { spans } => Some(spans.as_slice()),
                            _ => None,
                        })
                        .unwrap_or(&[]);
                    if spans.is_empty() {
                        render_text_with_wikis(
                            ui,
                            vault,
                            source_path,
                            body,
                            theme::EDITOR_SIZE,
                            false,
                            0,
                            done,
                        )
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            render_spans(
                                ui,
                                vault,
                                source_path,
                                spans,
                                theme::EDITOR_SIZE,
                                false,
                                0,
                                show_comments,
                                done,
                            )
                        })
                        .inner
                    }
                }
            })
            .inner;
        merge_click(click, body_click)
    })
    .inner
}

/// Checked task body: strike + ~55% alpha of [`theme::TEXT`].
fn task_done_color() -> Color32 {
    Color32::from_rgba_unmultiplied(theme::TEXT.r(), theme::TEXT.g(), theme::TEXT.b(), 140)
}

fn with_task_done(rich: RichText, done: bool) -> RichText {
    if done {
        rich.strikethrough().color(task_done_color())
    } else {
        rich
    }
}

/// One stripe of a multi-line fenced code block (open / body / close).
fn render_fence_stripe(ui: &mut Ui, line: &str, role: FenceRole, lang: Option<&str>) -> LineClick {
    let r = theme::ROUND;
    let corners = match role {
        FenceRole::Open => CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        FenceRole::Close => CornerRadius {
            nw: 0,
            ne: 0,
            sw: r,
            se: r,
        },
        FenceRole::Body => CornerRadius::ZERO,
        FenceRole::Outside => CornerRadius::ZERO,
    };
    let (top, bottom) = match role {
        FenceRole::Open => (8, 2),
        FenceRole::Close => (2, 8),
        FenceRole::Body => (2, 2),
        FenceRole::Outside => (4, 4),
    };

    let mut clicked = false;
    // No per-stripe stroke — adjacent stripes stack into one continuous block.
    egui::Frame::NONE
        .fill(theme::CODE_BG)
        .corner_radius(corners)
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top,
            bottom,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if matches!(role, FenceRole::Open) {
                if let Some(lang) = lang {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(lang)
                                .size(12.0)
                                .monospace()
                                .color(theme::WEAK),
                        );
                    });
                }
                let resp = ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .size(14.0)
                            .monospace()
                            .color(theme::WEAK),
                    )
                    .sense(Sense::click()),
                );
                clicked = resp.clicked();
            } else if matches!(role, FenceRole::Close) {
                let resp = ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .size(14.0)
                            .monospace()
                            .color(theme::WEAK),
                    )
                    .sense(Sense::click()),
                );
                clicked = resp.clicked();
            } else {
                let shown = if line.is_empty() { " " } else { line };
                let job = highlight_code_line(shown, lang);
                let resp = ui.add(egui::Label::new(job).sense(Sense::click()));
                clicked = resp.clicked();
            }
        });
    if clicked {
        LineClick::Edit
    } else {
        LineClick::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeSpanKind {
    Base,
    Keyword,
    Comment,
    String,
}

/// Lex one code line into (slice, kind) spans — keywords, comments, strings.
fn highlight_spans<'a>(line: &'a str, lang: Option<&str>) -> Vec<(&'a str, CodeSpanKind)> {
    let words = keywords_for(lang);
    let style = comment_style(lang);
    let mut out = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        if let Some((n, kind)) = take_comment_or_string(rest, style) {
            out.push((&rest[..n], kind));
            rest = &rest[n..];
            continue;
        }
        let (tok, next) = next_code_token(rest);
        rest = next;
        let kind = if is_ident(tok) && words.iter().any(|w| *w == tok) {
            CodeSpanKind::Keyword
        } else {
            CodeSpanKind::Base
        };
        out.push((tok, kind));
    }
    out
}

fn highlight_code_line(line: &str, lang: Option<&str>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mono = |color: Color32| TextFormat {
        font_id: FontId::new(16.0, FontFamily::Monospace),
        color,
        ..Default::default()
    };
    let base = mono(theme::TEXT);
    let kw = mono(theme::ACCENT);
    let comment = mono(theme::WEAK);
    let string = mono(theme::ACCENT_HOVER);
    for (tok, kind) in highlight_spans(line, lang) {
        let fmt = match kind {
            CodeSpanKind::Base => base.clone(),
            CodeSpanKind::Keyword => kw.clone(),
            CodeSpanKind::Comment => comment.clone(),
            CodeSpanKind::String => string.clone(),
        };
        job.append(tok, 0.0, fmt);
    }
    job
}

#[derive(Clone, Copy)]
struct CommentStyle {
    line: Option<&'static str>,
    block_open: Option<&'static str>,
    block_close: Option<&'static str>,
}

fn comment_style(lang: Option<&str>) -> CommentStyle {
    match lang.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("python" | "py" | "bash" | "sh" | "shell") => CommentStyle {
            line: Some("#"),
            block_open: None,
            block_close: None,
        },
        Some("json" | "mermaid") => CommentStyle {
            line: None,
            block_open: None,
            block_close: None,
        },
        // rust / js / ts / go / default C-like
        _ => CommentStyle {
            line: Some("//"),
            block_open: Some("/*"),
            block_close: Some("*/"),
        },
    }
}

fn take_comment_or_string(s: &str, style: CommentStyle) -> Option<(usize, CodeSpanKind)> {
    let bytes = s.as_bytes();
    if let Some(mark) = style.line {
        if s.starts_with(mark) {
            return Some((s.len(), CodeSpanKind::Comment));
        }
    }
    if let (Some(open), Some(close)) = (style.block_open, style.block_close) {
        if s.starts_with(open) {
            let end = s[open.len()..]
                .find(close)
                .map(|i| open.len() + i + close.len())
                .unwrap_or(s.len());
            return Some((end, CodeSpanKind::Comment));
        }
    }
    let quote = match bytes.first() {
        Some(b'"' | b'\'') => bytes[0],
        Some(b'`') if style.line == Some("//") => b'`', // js/ts template — treat as string to EOL or close
        _ => return None,
    };
    // Skip opening quote; scan for unescaped close (or EOL).
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return Some((i + 1, CodeSpanKind::String));
        }
        i += 1;
    }
    Some((s.len(), CodeSpanKind::String))
}

fn next_code_token(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    if bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_' {
        let n = bytes
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
            .count();
        return (&s[..n], &s[n..]);
    }
    (&s[..1], &s[1..])
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn keywords_for(lang: Option<&str>) -> &'static [&'static str] {
    match lang.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("rust" | "rs") => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "use", "mod", "if", "else",
            "match", "return", "for", "while", "loop", "in", "ref", "self", "Self", "crate",
            "async", "await", "const", "static", "true", "false",
        ],
        Some("js" | "javascript" | "ts" | "typescript") => &[
            "function", "const", "let", "var", "return", "if", "else", "for", "while", "class",
            "new", "this", "import", "export", "from", "async", "await", "true", "false", "null",
            "typeof", "of", "in",
        ],
        Some("python" | "py") => &[
            "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as",
            "True", "False", "None", "with", "try", "except", "async", "await", "yield", "lambda",
            "pass",
        ],
        Some("go") => &[
            "func",
            "return",
            "if",
            "else",
            "for",
            "range",
            "package",
            "import",
            "var",
            "const",
            "type",
            "struct",
            "interface",
            "go",
            "defer",
            "true",
            "false",
            "nil",
        ],
        Some("json") => &["true", "false", "null"],
        Some("bash" | "sh" | "shell") => &[
            "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
            "function", "return", "in", "export", "local", "echo", "cd", "exit",
        ],
        // mermaid / unknown: no keyword paint
        _ => &[],
    }
}

fn merge_click(a: LineClick, b: LineClick) -> LineClick {
    match (&a, &b) {
        (_, LineClick::ResizeEmbed { .. }) => b,
        (LineClick::ResizeEmbed { .. }, _) => a,
        (_, LineClick::ToggleCheck) => b,
        (LineClick::ToggleCheck, _) => a,
        (_, LineClick::Tag { .. }) => b,
        (LineClick::Tag { .. }, _) => a,
        (_, LineClick::Footnote { .. }) => b,
        (LineClick::Footnote { .. }, _) => a,
        (_, LineClick::Wiki { .. }) => b,
        (LineClick::Wiki { .. }, _) => a,
        (_, LineClick::Edit) => b,
        (LineClick::Edit, _) => a,
        _ => a,
    }
}

fn render_block_compact(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    block: &PreviewBlock,
    show_comments: bool,
) -> LineClick {
    match block {
        PreviewBlock::Heading { level, text } => render_text_with_wikis(
            ui,
            vault,
            source_path,
            text,
            theme::heading_size(*level),
            true,
            *level,
            false,
        ),
        PreviewBlock::Paragraph { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                click = render_spans(
                    ui,
                    vault,
                    source_path,
                    spans,
                    theme::EDITOR_SIZE,
                    false,
                    0,
                    show_comments,
                    false,
                );
            });
            click
        }
        PreviewBlock::BulletItem { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                click = render_spans(
                    ui,
                    vault,
                    source_path,
                    spans,
                    theme::EDITOR_SIZE,
                    false,
                    0,
                    show_comments,
                    false,
                );
            });
            click
        }
        PreviewBlock::NumberedItem { number, spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{number}."));
                click = render_spans(
                    ui,
                    vault,
                    source_path,
                    spans,
                    theme::EDITOR_SIZE,
                    false,
                    0,
                    show_comments,
                    false,
                );
            });
            click
        }
        PreviewBlock::Quote { spans } => {
            let mut click = LineClick::None;
            egui::Frame::NONE
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(theme::round())
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        click = render_spans(
                            ui,
                            vault,
                            source_path,
                            spans,
                            theme::EDITOR_SIZE,
                            false,
                            0,
                            show_comments,
                            false,
                        );
                    });
                });
            click
        }
        PreviewBlock::CodeBlock { code, lang } => {
            // Lone ``` lines parse as empty code blocks — don't draw hollow pills;
            // multi-line fences are handled via FenceRole in the editor.
            if code.trim().is_empty() {
                return render_fence_stripe(ui, "```", FenceRole::Open, lang.as_deref());
            }
            let mut clicked = false;
            egui::Frame::NONE
                .fill(theme::CODE_BG)
                .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
                .corner_radius(theme::round())
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(lang) = lang {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            ui.label(
                                RichText::new(lang)
                                    .size(12.0)
                                    .monospace()
                                    .color(theme::WEAK),
                            );
                        });
                    }
                    for line in code.trim_end_matches('\n').lines() {
                        let shown = if line.is_empty() { " " } else { line };
                        let job = highlight_code_line(shown, lang.as_deref());
                        let resp = ui.add(egui::Label::new(job).sense(Sense::click()));
                        clicked |= resp.clicked();
                    }
                });
            if clicked {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        PreviewBlock::ThematicBreak => {
            // ui.separator() uses STROKE on the near-black canvas and disappears.
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::click());
            ui.painter().hline(
                rect.x_range(),
                rect.center().y,
                egui::Stroke::new(2.0_f32, theme::WEAK),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
    }
}

fn render_text_with_wikis(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
    done: bool,
) -> LineClick {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        render_spans(
            ui,
            vault,
            source_path,
            &split_wikilinks(text),
            size,
            heading,
            level,
            true,
            done,
        )
    })
    .inner
}

fn render_spans(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    spans: &[Span],
    size: f32,
    heading: bool,
    level: u8,
    show_comments: bool,
    done: bool,
) -> LineClick {
    let mut click = LineClick::None;
    let mut at = 0;
    while at < spans.len() {
        if is_inline_text_span(&spans[at]) {
            let end = inline_text_run_end(spans, at);
            click = merge_click(
                click,
                render_inline_text_job(
                    ui,
                    vault,
                    source_path,
                    &spans[at..end],
                    size,
                    heading,
                    level,
                    show_comments,
                    done,
                ),
            );
            at = end;
        } else {
            click = merge_click(click, render_one_span(
                ui,
                vault,
                source_path,
                &spans[at],
                size,
                heading,
                level,
                show_comments,
                done,
            ));
            at += 1;
        }
    }
    click
}

fn is_inline_text_span(span: &Span) -> bool {
    !matches!(
        span,
        Span::Embed { .. } | Span::Image { .. } | Span::InlineMath { .. }
    )
}

fn inline_text_run_end(spans: &[Span], start: usize) -> usize {
    spans[start..]
        .iter()
        .position(|span| !is_inline_text_span(span))
        .map(|offset| start + offset)
        .unwrap_or(spans.len())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InlineHitAction {
    Wiki(String),
    Tag(String),
    Footnote(String),
    Open(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineHit {
    bytes: Range<usize>,
    action: InlineHitAction,
}

fn inline_font(size: f32, heading: bool, level: u8) -> FontId {
    if heading {
        FontId::new(theme::heading_size(level), theme::heading_family(level))
    } else {
        FontId::new(size, FontFamily::Proportional)
    }
}

fn inline_format(size: f32, heading: bool, level: u8, done: bool) -> TextFormat {
    let color = if done { task_done_color() } else { theme::TEXT };
    TextFormat {
        font_id: inline_font(size, heading, level),
        color,
        strikethrough: if done {
            egui::Stroke::new(1.0_f32, color)
        } else {
            egui::Stroke::NONE
        },
        ..Default::default()
    }
}

fn append_inline_run(
    job: &mut LayoutJob,
    hits: &mut Vec<InlineHit>,
    text: &str,
    format: TextFormat,
    action: Option<InlineHitAction>,
) {
    if text.is_empty() {
        return;
    }
    let start = job.text.len();
    job.append(text, 0.0, format);
    if let Some(action) = action {
        hits.push(InlineHit {
            bytes: start..job.text.len(),
            action,
        });
    }
}

fn append_tag_runs(
    job: &mut LayoutJob,
    hits: &mut Vec<InlineHit>,
    text: &str,
    format: &TextFormat,
    done: bool,
) {
    let mut rest = text;
    while !rest.is_empty() {
        let Some(hash_at) = rest.find('#') else {
            append_inline_run(job, hits, rest, format.clone(), None);
            break;
        };
        if hash_at > 0 {
            append_inline_run(job, hits, &rest[..hash_at], format.clone(), None);
            rest = &rest[hash_at..];
        }
        let after = &rest[1..];
        let tag_len = after
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, b'_' | b'-' | b'/'))
            .count();
        if tag_len == 0 {
            append_inline_run(job, hits, "#", format.clone(), None);
            rest = after;
            continue;
        }
        let tag = &after[..tag_len];
        let mut tag_format = format.clone();
        tag_format.color = if done {
            task_done_color()
        } else {
            theme::ACCENT_HOVER
        };
        tag_format.background = theme::ACCENT.gamma_multiply(if done { 0.08 } else { 0.16 });
        append_inline_run(
            job,
            hits,
            &format!("#{tag}"),
            tag_format,
            Some(InlineHitAction::Tag(tag.to_string())),
        );
        rest = &after[tag_len..];
    }
}

fn append_text_runs(
    job: &mut LayoutJob,
    hits: &mut Vec<InlineHit>,
    text: &str,
    format: &TextFormat,
    done: bool,
) {
    for piece in crate::footnote::split_inline_footnotes(text) {
        match piece {
            crate::footnote::InlinePiece::Text(chunk) => {
                append_tag_runs(job, hits, chunk, format, done);
            }
            crate::footnote::InlinePiece::Inline { .. } => {
                let mut marker = format.clone();
                marker.font_id.size *= 0.85;
                marker.color = if done { task_done_color() } else { theme::ACCENT };
                append_inline_run(job, hits, "*", marker, None);
            }
            crate::footnote::InlinePiece::Ref { id } => {
                let mut marker = format.clone();
                marker.font_id.size *= 0.85;
                marker.color = if done { task_done_color() } else { theme::ACCENT };
                append_inline_run(
                    job,
                    hits,
                    &format!("[^{id}]"),
                    marker,
                    Some(InlineHitAction::Footnote(id.to_string())),
                );
            }
        }
    }
}

fn build_inline_text_job(
    vault: &Vault,
    source_path: Option<&str>,
    spans: &[Span],
    size: f32,
    heading: bool,
    level: u8,
    show_comments: bool,
    done: bool,
) -> (LayoutJob, Vec<InlineHit>) {
    let base = inline_format(size, heading, level, done);
    let mut job = LayoutJob::default();
    let mut hits = Vec::new();

    for span in spans {
        match span {
            Span::Text(text) => append_text_runs(&mut job, &mut hits, text, &base, done),
            Span::Comment(body) if show_comments => {
                let mut format = base.clone();
                format.color = if done { task_done_color() } else { theme::TEXT_FAINT };
                append_inline_run(
                    &mut job,
                    &mut hits,
                    &format!("%%{body}%%"),
                    format,
                    None,
                );
            }
            Span::Comment(_) => {}
            Span::Strong(text) => {
                let mut format = base.clone();
                format.color = if done { task_done_color() } else { Color32::WHITE };
                append_inline_run(&mut job, &mut hits, text, format, None);
            }
            Span::Emphasis(text) => {
                let mut format = base.clone();
                format.italics = true;
                append_inline_run(&mut job, &mut hits, text, format, None);
            }
            Span::Strike(text) => {
                let mut format = base.clone();
                format.color = if done { task_done_color() } else { theme::WEAK };
                format.strikethrough = egui::Stroke::new(1.0_f32, format.color);
                append_inline_run(&mut job, &mut hits, text, format, None);
            }
            Span::Highlight(text) => {
                let mut format = base.clone();
                format.background = Color32::from_rgba_unmultiplied(
                    theme::WARN.r(),
                    theme::WARN.g(),
                    theme::WARN.b(),
                    if done { 28 } else { 56 },
                );
                append_inline_run(&mut job, &mut hits, text, format, None);
            }
            Span::Code(text) => {
                let mut format = base.clone();
                format.font_id = FontId::new(size, FontFamily::Monospace);
                format.background = theme::CODE_BG;
                append_inline_run(&mut job, &mut hits, text, format, None);
            }
            Span::Link { text, dest } => {
                let mut format = base.clone();
                format.color = if done { task_done_color() } else { theme::ACCENT };
                format.underline = egui::Stroke::new(1.0_f32, format.color);
                append_inline_run(
                    &mut job,
                    &mut hits,
                    text,
                    format,
                    Some(InlineHitAction::Open(dest.clone())),
                );
            }
            Span::WikiLink { raw, alias } => {
                let label = alias
                    .as_deref()
                    .unwrap_or_else(|| raw.split('|').next().unwrap_or(raw));
                let mut format = base.clone();
                format.color = if done {
                    task_done_color()
                } else if vault.index().resolve(raw, source_path).is_some() {
                    theme::ACCENT
                } else {
                    theme::WARN
                };
                format.underline = egui::Stroke::new(1.0_f32, format.color);
                append_inline_run(
                    &mut job,
                    &mut hits,
                    label,
                    format,
                    Some(InlineHitAction::Wiki(raw.clone())),
                );
            }
            Span::InlineMath { .. } | Span::Embed { .. } | Span::Image { .. } => {
                unreachable!("non-text spans use the widget renderer")
            }
        }
    }
    (job, hits)
}

fn hit_at_pointer<'a>(
    galley: &egui::Galley,
    origin: egui::Pos2,
    pointer: egui::Pos2,
    hits: &'a [InlineHit],
) -> Option<&'a InlineHitAction> {
    let local = pointer - origin;
    let local = egui::pos2(local.x, local.y);
    let byte_offsets: Vec<usize> = galley.text().char_indices().map(|(byte, _)| byte).collect();
    let mut char_index = 0;
    for row in &galley.rows {
        for glyph in &row.glyphs {
            if glyph.logical_rect().contains(local) {
                let byte_index = byte_offsets.get(char_index).copied()?;
                return hits
                    .iter()
                    .find(|hit| hit.bytes.contains(&byte_index))
                    .map(|hit| &hit.action);
            }
            char_index += 1;
        }
        if row.ends_with_newline {
            char_index += 1;
        }
    }
    None
}

fn render_inline_text_job(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    spans: &[Span],
    size: f32,
    heading: bool,
    level: u8,
    show_comments: bool,
    done: bool,
) -> LineClick {
    let (job, hits) = build_inline_text_job(
        vault,
        source_path,
        spans,
        size,
        heading,
        level,
        show_comments,
        done,
    );
    if job.text.is_empty() {
        return LineClick::None;
    }

    let (origin, galley, response) = egui::Label::new(job)
        .selectable(false)
        .sense(Sense::click())
        .layout_in_ui(ui);
    if ui.is_rect_visible(response.rect) {
        ui.painter().galley(origin, galley.clone(), theme::TEXT);
    }

    let hover_action = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pointer| response.rect.contains(*pointer))
        .and_then(|pointer| hit_at_pointer(&galley, origin, pointer, &hits));
    if hover_action.is_some() {
        ui.output_mut(|output| output.cursor_icon = CursorIcon::PointingHand);
    }

    if !response.clicked() {
        return LineClick::None;
    }
    let click_action = response
        .interact_pointer_pos()
        .and_then(|pointer| hit_at_pointer(&galley, origin, pointer, &hits));
    match click_action {
        Some(InlineHitAction::Wiki(raw)) => LineClick::Wiki {
            raw: raw.clone(),
            new_tab: ui.input(|input| input.modifiers.command),
        },
        Some(InlineHitAction::Tag(query)) => LineClick::Tag {
            query: query.clone(),
        },
        Some(InlineHitAction::Footnote(id)) => LineClick::Footnote { id: id.clone() },
        Some(InlineHitAction::Open(url)) => {
            ui.ctx().open_url(OpenUrl::new_tab(url.clone()));
            LineClick::None
        }
        None => LineClick::Edit,
    }
}

fn render_one_span(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    span: &Span,
    size: f32,
    heading: bool,
    level: u8,
    show_comments: bool,
    done: bool,
) -> LineClick {
    match span {
        Span::Text(t) => render_text_chunks(ui, t, size, heading, level, done),
        Span::Comment(body) => {
            if !show_comments {
                return LineClick::None;
            }
            // Paint delimiters so inactive-line width matches the buffer.
            let shown = format!("%%{body}%%");
            let resp = ui.add(
                egui::Label::new(with_task_done(
                    RichText::new(shown).size(size).color(theme::WEAK),
                    done,
                ))
                .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Strong(t) => {
            let resp = ui.add(
                egui::Label::new(with_task_done(
                    RichText::new(t).size(size).strong().color(theme::TEXT),
                    done,
                ))
                .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Emphasis(t) => {
            let resp = ui.add(
                egui::Label::new(with_task_done(
                    RichText::new(t).size(size).italics().color(theme::TEXT),
                    done,
                ))
                .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Strike(t) => {
            let resp = ui.add(
                egui::Label::new(with_task_done(
                    RichText::new(t)
                        .size(size)
                        .strikethrough()
                        .color(theme::WEAK),
                    done,
                ))
                .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Highlight(t) => {
            let mut clicked = false;
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(
                    theme::WARN.r(),
                    theme::WARN.g(),
                    theme::WARN.b(),
                    if done { 28 } else { 56 },
                ))
                .corner_radius(theme::round_sm())
                .inner_margin(egui::Margin::symmetric(3, 1))
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::Label::new(with_task_done(
                            RichText::new(t).size(size).color(theme::TEXT),
                            done,
                        ))
                        .sense(Sense::click()),
                    );
                    clicked = resp.clicked();
                });
            if clicked {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::InlineMath { formula } => {
            if math_block::show_inline(ui, formula, size) {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Code(t) => {
            // Same CODE_BG as fenced blocks — RichText::code() is too subtle here.
            let mut clicked = false;
            egui::Frame::NONE
                .fill(theme::CODE_BG)
                .corner_radius(theme::round_sm())
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::Label::new(with_task_done(
                            RichText::new(t).size(size).monospace().color(theme::TEXT),
                            done,
                        ))
                        .sense(Sense::click()),
                    );
                    clicked = resp.clicked();
                });
            if clicked {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Link { text, dest } => {
            // External / markdown links keep default hyperlink behaviour.
            ui.hyperlink_to(with_task_done(RichText::new(text).size(size), done), dest);
            LineClick::None
        }
        Span::WikiLink { raw, alias } => {
            let target = bluelink_core::normalize_link_target(raw);
            let label = alias
                .as_deref()
                .unwrap_or_else(|| raw.split('|').next().unwrap_or(raw));
            let resolved = vault.index().resolve(raw, source_path);
            let base = if done {
                task_done_color()
            } else if resolved.is_some() {
                theme::ACCENT
            } else {
                theme::WARN
            };

            let font = if heading {
                FontId::new(theme::heading_size(level), theme::heading_family(level))
            } else {
                FontId::new(size, FontFamily::Proportional)
            };
            // Measure with a placeholder color; final color depends on hover.
            let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font.clone(), base));
            let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
            let hovered = response.hovered();
            let color = if hovered && !done {
                lighten(base)
            } else {
                base
            };

            if hovered && !done {
                ui.painter().rect_filled(
                    rect.expand2(egui::vec2(4.0, 2.0)),
                    theme::round_sm(),
                    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 40),
                );
            }
            let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font, color));
            ui.painter().galley(rect.min, galley, color);
            let line_y = if done {
                rect.center().y
            } else {
                rect.bottom() - 1.0
            };
            ui.painter().hline(
                rect.left()..=rect.right(),
                line_y,
                egui::Stroke::new(
                    if hovered && !done { 1.5_f32 } else { 1.0_f32 },
                    color,
                ),
            );

            response
                .clone()
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(match &resolved {
                    Some(p) => format!("Open {p}"),
                    None => format!("Create {target}"),
                });
            if response.clicked() {
                let new_tab = ui.input(|i| i.modifiers.command);
                LineClick::Wiki {
                    raw: raw.clone(),
                    new_tab,
                }
            } else {
                LineClick::None
            }
        }
        Span::Embed {
            target,
            alias,
            width,
            height,
        } => {
            let label = alias.as_deref();
            if let Some(path) = vault.index().resolve(target, source_path) {
                if path.ends_with(".md") {
                    return render_note_embed(ui, vault, &path, target, label);
                }
            }
            render_media_embed(ui, vault, target, label, *width, *height)
        }
        Span::Image {
            dest,
            alt,
            width,
            height,
        } => {
            if dest.starts_with("http://") || dest.starts_with("https://") {
                let draw_w = embed_draw_width(ui, *width);
                let mut img = egui::Image::new(dest.as_str()).max_width(draw_w);
                if let Some(h) = *height {
                    img = img
                        .fit_to_exact_size(egui::vec2(draw_w, h as f32))
                        .maintain_aspect_ratio(false);
                } else {
                    img = img.maintain_aspect_ratio(true);
                }
                ui.add(img);
                if !alt.is_empty() {
                    ui.label(RichText::new(alt).size(13.0).color(theme::WEAK));
                }
                LineClick::None
            } else {
                let label = if alt.is_empty() {
                    None
                } else {
                    Some(alt.as_str())
                };
                render_media_embed(ui, vault, dest, label, *width, *height)
            }
        }
    }
}

fn embed_draw_width(ui: &Ui, width: Option<u32>) -> f32 {
    let avail = ui.available_width().max(40.0);
    match width {
        Some(w) => (w as f32).clamp(40.0, avail),
        None => avail.min(640.0),
    }
}

/// Split plain text on footnotes (`^[…]`, `[^id]`) then `#tags`.
fn render_text_chunks(
    ui: &mut Ui,
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
    done: bool,
) -> LineClick {
    let mut click = LineClick::None;
    for piece in crate::footnote::split_inline_footnotes(text) {
        match piece {
            crate::footnote::InlinePiece::Text(chunk) => {
                click = merge_click(
                    click,
                    render_tag_chunks(ui, chunk, size, heading, level, done),
                );
            }
            crate::footnote::InlinePiece::Inline { text: inner } => {
                let color = if done { task_done_color() } else { theme::ACCENT };
                let resp = ui
                    .add(
                        egui::Label::new(with_task_done(
                            RichText::new("*").size(size * 0.85).color(color),
                            done,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_text(inner)
                    .on_hover_cursor(CursorIcon::Help);
                if resp.clicked() {
                    click = merge_click(click, LineClick::Edit);
                }
            }
            crate::footnote::InlinePiece::Ref { id } => {
                let color = if done { task_done_color() } else { theme::ACCENT };
                let resp = ui
                    .add(
                        egui::Label::new(with_task_done(
                            RichText::new(format!("[^{id}]"))
                                .size(size * 0.85)
                                .color(color),
                            done,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(format!("Go to [^{id}]"));
                if resp.clicked() {
                    click = LineClick::Footnote { id: id.to_string() };
                }
            }
        }
    }
    click
}

/// Split plain text on `#tags` (Obsidian-style: `#` + non-space run).
fn render_tag_chunks(
    ui: &mut Ui,
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
    done: bool,
) -> LineClick {
    let mut click = LineClick::None;
    let mut rest = text;
    while !rest.is_empty() {
        let Some(hash_at) = rest.find('#') else {
            click = merge_click(click, paint_plain_text(ui, rest, size, heading, level, done));
            break;
        };
        if hash_at > 0 {
            click = merge_click(
                click,
                paint_plain_text(ui, &rest[..hash_at], size, heading, level, done),
            );
            rest = &rest[hash_at..];
        }
        let after = &rest[1..];
        let tag_len = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
            .count();
        if tag_len == 0 {
            click = merge_click(click, paint_plain_text(ui, "#", size, heading, level, done));
            rest = after;
            continue;
        }
        let tag = &after[..tag_len];
        let tag_color = if done {
            task_done_color()
        } else {
            theme::ACCENT_HOVER
        };
        let resp = egui::Frame::NONE
            .fill(if done {
                theme::ACCENT.gamma_multiply(0.08)
            } else {
                theme::ACCENT.gamma_multiply(0.16)
            })
            .corner_radius(egui::CornerRadius::same(theme::ROUND_PILL))
            .inner_margin(egui::Margin::symmetric(4, 1))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(with_task_done(
                        RichText::new(format!("#{tag}")).size(size).color(tag_color),
                        done,
                    ))
                    .sense(Sense::click()),
                )
            })
            .inner
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            click = LineClick::Tag {
                query: tag.to_string(),
            };
        }
        rest = &after[tag_len..];
    }
    click
}

fn paint_plain_text(
    ui: &mut Ui,
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
    done: bool,
) -> LineClick {
    if text.is_empty() {
        return LineClick::None;
    }
    let rich = if heading {
        theme::heading_rich(text, level)
    } else {
        RichText::new(text).size(size).color(theme::TEXT)
    };
    let resp = ui.add(egui::Label::new(with_task_done(rich, done)).sense(Sense::click()));
    if resp.clicked() {
        LineClick::Edit
    } else {
        LineClick::None
    }
}

/// `![[Note]]` / `![[Note#frag]]` card. Click opens the note (and scrolls to fragment).
fn render_note_embed(
    ui: &mut Ui,
    vault: &Vault,
    path: &str,
    embed_target: &str,
    alias: Option<&str>,
) -> LineClick {
    let content = vault.read(path).unwrap_or_default();
    let title = alias
        .map(str::to_string)
        .unwrap_or_else(|| bluelink_core::title_from_content(&content, path));
    let fragment = link_fragment(embed_target);

    enum Body<'a> {
        Lines(Vec<&'a str>),
        Missing(String),
    }
    let body = match fragment.as_deref() {
        None => Body::Lines(
            content
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with("---")
                })
                .take(4)
                .collect(),
        ),
        Some(frag) => match fragment_line_range(&content, frag) {
            Some((start, end)) => {
                Body::Lines(content.lines().skip(start).take(end - start).collect())
            }
            None => Body::Missing(format!("Not found: #{frag}")),
        },
    };

    let mut open = false;
    egui::Frame::NONE
        .fill(theme::CODE_BG)
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round())
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let title_resp = ui.add(
                egui::Label::new(
                    RichText::new(&title)
                        .size(theme::EDITOR_SIZE)
                        .strong()
                        .color(theme::ACCENT),
                )
                .sense(Sense::click()),
            );
            open |= title_resp
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked();
            match body {
                Body::Lines(lines) => {
                    for line in lines {
                        let resp = ui.add(
                            egui::Label::new(RichText::new(line).size(13.0).color(theme::WEAK))
                                .sense(Sense::click()),
                        );
                        open |= resp.clicked();
                    }
                }
                Body::Missing(msg) => {
                    let resp = ui.add(
                        egui::Label::new(RichText::new(msg).size(13.0).color(theme::WEAK))
                            .sense(Sense::click()),
                    );
                    open |= resp.clicked();
                }
            }
        });
    if open {
        let stem = path.trim_end_matches(".md");
        let raw = match fragment.as_deref() {
            Some(frag) => format!("{stem}#{frag}"),
            None => stem.to_string(),
        };
        LineClick::Wiki {
            raw,
            new_tab: ui.input(|i| i.modifiers.command),
        }
    } else {
        LineClick::None
    }
}

/// Click on a callout card: edit a source line, or toggle the `+`/`-` marker.
pub struct CalloutClick {
    pub edit: Option<usize>,
    pub toggle_fold: bool,
}

/// Paint a callout card for lines `[start, end)`. A `-` marker hides the body.
pub fn paint_callout(ui: &mut Ui, lines: &[String], start: usize, end: usize) -> CalloutClick {
    let Some(first) = lines.get(start) else {
        return CalloutClick {
            edit: None,
            toggle_fold: false,
        };
    };
    let collapsed = callout_collapsed(first);
    let ty = callout_type(first).unwrap_or("note");
    let (kind, unknown) = callout_kind(ty);
    let visual = callout_visual(kind);
    let type_label = if unknown {
        ty.to_ascii_uppercase()
    } else {
        kind.label().to_string()
    };
    let title = {
        let t = first.trim_start();
        let after_gt = t.strip_prefix('>').map(str::trim_start).unwrap_or(t);
        let rest = after_gt.find(']').map(|i| &after_gt[i + 1..]).unwrap_or("");
        let rest = if rest.starts_with('+') || rest.starts_with('-') {
            &rest[1..]
        } else {
            rest
        };
        rest.trim().to_string()
    };
    let mut edit = None;
    let mut toggle_fold = false;
    egui::Frame::NONE
        .fill(visual.fill)
        .stroke(egui::Stroke::new(1.0_f32, visual.accent))
        .corner_radius(theme::round())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.y = 18.0;
                let chevron = if collapsed {
                    regular::CARET_RIGHT
                } else {
                    regular::CARET_DOWN
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(chevron).size(14.0).color(theme::WEAK))
                            .frame(false),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    toggle_fold = true;
                }
                ui.label(RichText::new(visual.icon).size(14.0).color(visual.accent));
                ui.label(
                    RichText::new(&type_label)
                        .size(12.0)
                        .strong()
                        .color(visual.accent),
                );
                if !title.is_empty() {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(&title)
                                .size(theme::EDITOR_SIZE)
                                .strong()
                                .color(theme::TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        edit = Some(start);
                    }
                }
            });
            if !collapsed {
                for (k, line) in lines.iter().enumerate().take(end).skip(start + 1) {
                    let body = line
                        .trim_start()
                        .strip_prefix('>')
                        .map(str::trim_start)
                        .unwrap_or(line.as_str());
                    let shown = if body.is_empty() { " " } else { body };
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(shown)
                                .size(theme::EDITOR_SIZE)
                                .color(theme::TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        edit = Some(k);
                    }
                }
            }
            if title.is_empty() {
                let resp =
                    ui.allocate_response(egui::vec2(ui.available_width(), 4.0), Sense::click());
                if resp.clicked() {
                    edit = Some(start);
                }
            }
        });
    CalloutClick { edit, toggle_fold }
}

/// Embed / local image: capped async texture (not egui file:// decode), other media as open-externally.
fn render_media_embed(
    ui: &mut Ui,
    vault: &Vault,
    target: &str,
    alias: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
) -> LineClick {
    let display = alias.unwrap_or(target);
    // Cache resolve across frames — avoid even `is_file` / dir walks while scrolling.
    let resolve_id = egui::Id::new(("bl_media_resolve", target));
    let abs = ui
        .ctx()
        .data(|d| d.get_temp::<Option<std::path::PathBuf>>(resolve_id));
    let abs = match abs {
        Some(cached) => cached,
        None => {
            let found = media::resolve_media(vault, target);
            ui.ctx()
                .data_mut(|d| d.insert_temp(resolve_id, found.clone()));
            found
        }
    };
    let Some(abs) = abs else {
        let resp = ui
            .add(
                egui::Label::new(
                    RichText::new(format!("{}  {display}", regular::IMAGE_BROKEN))
                        .size(theme::EDITOR_SIZE)
                        .color(theme::WEAK),
                )
                .sense(Sense::click()),
            )
            .on_hover_text(format!("Missing: {target}"));
        return if resp.clicked() {
            LineClick::Edit
        } else {
            LineClick::None
        };
    };

    let ext = media::extension_of(&abs).unwrap_or_default();
    let url = media::file_url(&abs);
    let avail = ui.available_width().max(40.0);
    let drag_id = ui.id().with("embed_w").with(target);
    let drag_w = ui.ctx().data(|d| d.get_temp::<f32>(drag_id));
    let base_w = embed_draw_width(ui, width);
    let draw_w = drag_w.unwrap_or(base_w).clamp(40.0, avail);

    if media::is_previewable_image(&ext) {
        let (media_rect, img_clicked) = match embed_texture(ui, &abs) {
            Some(tex) => {
                let native = tex.size_vec2();
                let draw_size = match height {
                    Some(h) => egui::vec2(draw_w, h as f32),
                    None => {
                        let scale = draw_w / native.x.max(1.0);
                        native * scale
                    }
                };
                let resp = ui
                    .add(
                        egui::Image::new((tex.id(), draw_size))
                            .maintain_aspect_ratio(height.is_none())
                            .sense(Sense::click()),
                    )
                    .on_hover_text(abs.display().to_string());
                (resp.rect, resp.clicked())
            }
            None => {
                // Decode in flight or failed — keep layout stable while loading.
                let ph_h = height
                    .map(|h| h as f32)
                    .unwrap_or(draw_w * 0.55)
                    .clamp(40.0, 320.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(draw_w, ph_h), Sense::hover());
                ui.painter()
                    .rect_filled(rect, theme::round_sm(), theme::CODE_BG);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    regular::IMAGE,
                    FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
                    theme::WEAK,
                );
                (rect, false)
            }
        };

        let resized = paint_resize_handle(ui, media_rect, drag_id, draw_w, avail, target, height);
        if let Some(click) = resized {
            return click;
        }
        if img_clicked && drag_w.is_none() {
            LineClick::Edit
        } else {
            LineClick::None
        }
    } else {
        let kind = media_placeholder_kind(&ext);
        let box_w = draw_w.min(avail);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(box_w, 40.0), Sense::click());
        ui.painter()
            .rect_filled(rect, theme::round_sm(), theme::CODE_BG);
        ui.painter().text(
            rect.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{}  {display}", kind.icon),
            FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
            theme::ACCENT,
        );
        let resized = paint_resize_handle(ui, rect, drag_id, draw_w, avail, target, height);
        if let Some(click) = resized {
            return click;
        }
        resp.clone()
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("Open {}", abs.display()));
        if resp.clicked() && drag_w.is_none() {
            ui.ctx().open_url(OpenUrl::new_tab(url));
        }
        LineClick::None
    }
}

/// Off-thread capped decode → [`TextureHandle`]. Avoids egui_extras sync UI-thread decode.
fn embed_texture(ui: &mut Ui, abs: &std::path::Path) -> Option<egui::TextureHandle> {
    use std::sync::{Arc, Mutex};

    let key = abs.to_string_lossy().to_string();
    let id = egui::Id::new(("bl_embed_tex", key.as_str()));

    #[derive(Clone)]
    enum Slot {
        Loading(Arc<Mutex<Option<Result<egui::ColorImage, String>>>>),
        Ready(egui::TextureHandle),
        Failed,
    }

    let slot = ui.ctx().data(|d| d.get_temp::<Slot>(id));
    match slot {
        Some(Slot::Ready(tex)) => return Some(tex),
        Some(Slot::Failed) => return None,
        Some(Slot::Loading(pending)) => {
            let done = pending.lock().ok().and_then(|mut g| g.take());
            match done {
                Some(Ok(color)) => {
                    let tex = ui.ctx().load_texture(
                        format!("bluelink-embed:{key}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(id, Slot::Ready(tex.clone())));
                    return Some(tex);
                }
                Some(Err(_)) => {
                    ui.ctx().data_mut(|d| d.insert_temp(id, Slot::Failed));
                    return None;
                }
                None => return None,
            }
        }
        None => {}
    }

    let pending = Arc::new(Mutex::new(None));
    ui.ctx()
        .data_mut(|d| d.insert_temp(id, Slot::Loading(pending.clone())));
    let path = abs.to_path_buf();
    let ctx = ui.ctx().clone();
    let _ = std::thread::Builder::new()
        .name("bluelink-embed-decode".into())
        .spawn(move || {
            let result = media::load_capped_color_image(&path);
            if let Ok(mut g) = pending.lock() {
                *g = Some(result);
            }
            ctx.request_repaint();
        });
    None
}

/// Corner handle: drag updates temp width; release → [`LineClick::ResizeEmbed`].
fn paint_resize_handle(
    ui: &mut Ui,
    media_rect: egui::Rect,
    drag_id: egui::Id,
    draw_w: f32,
    avail: f32,
    target: &str,
    height: Option<u32>,
) -> Option<LineClick> {
    let handle = 12.0;
    let handle_rect = egui::Rect::from_center_size(
        egui::pos2(media_rect.right() - 2.0, media_rect.bottom() - 2.0),
        egui::vec2(handle, handle),
    );
    let dragging = ui.ctx().data(|d| d.get_temp::<f32>(drag_id)).is_some();
    let hover = ui.input(|i| i.pointer.hover_pos());
    let show = dragging || hover.is_some_and(|p| media_rect.contains(p) || handle_rect.contains(p));
    if !show {
        return None;
    }

    ui.painter()
        .rect_filled(handle_rect, theme::round_sm(), theme::ACCENT);
    let sense = ui.interact(handle_rect, drag_id.with("h"), Sense::drag());
    sense.clone().on_hover_cursor(CursorIcon::ResizeEast);

    if sense.dragged() {
        let new_w = (draw_w + sense.drag_delta().x).clamp(40.0, avail);
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, new_w));
        ui.ctx().request_repaint();
    }
    if sense.drag_stopped() {
        let final_w = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<f32>(drag_id))
            .unwrap_or(draw_w)
            .round()
            .clamp(40.0, avail) as u32;
        return Some(LineClick::ResizeEmbed {
            target: target.to_string(),
            width: final_w.max(40),
            height,
        });
    }
    None
}

struct MediaKind {
    icon: &'static str,
}

fn media_placeholder_kind(ext: &str) -> MediaKind {
    let icon = match ext {
        "mp4" | "webm" | "ogv" | "mov" | "mkv" => regular::VIDEO,
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "opus" => regular::MUSIC_NOTES,
        "pdf" => regular::FILE_PDF,
        "svg" | "avif" | "bmp" => regular::IMAGE,
        _ => regular::FILE,
    };
    MediaKind { icon }
}

/// Brighten for hover on dark sheet (accent/warn stay legible).
fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(40),
        c.g().saturating_add(36),
        c.b().saturating_add(30),
    )
}

#[cfg(test)]
mod fence_tests {
    use super::*;

    #[test]
    fn highlight_spans_marks_rust_string_and_comment() {
        let spans = highlight_spans(r#"let x = "hi"; // done"#, Some("rust"));
        let kinds: Vec<_> = spans.iter().map(|(_, k)| *k).collect();
        assert!(
            kinds.contains(&CodeSpanKind::String),
            "expected string span in {spans:?}"
        );
        assert!(
            kinds.contains(&CodeSpanKind::Comment),
            "expected comment span in {spans:?}"
        );
        assert!(
            kinds.contains(&CodeSpanKind::Keyword),
            "expected keyword span in {spans:?}"
        );
        let joined: String = spans
            .iter()
            .filter(|(_, k)| *k == CodeSpanKind::String)
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(joined, r#""hi""#);
    }

    #[test]
    fn highlight_spans_json_and_bash_keywords() {
        let spans = highlight_spans("true", Some("json"));
        assert_eq!(spans, vec![("true", CodeSpanKind::Keyword)]);
        let spans = highlight_spans("echo hi", Some("bash"));
        assert!(spans.iter().any(|(t, k)| *t == "echo" && *k == CodeSpanKind::Keyword));
    }

    #[test]
    fn tracks_fenced_block_roles() {
        let mut open = None;
        assert_eq!(advance_fence("```rust", &mut open), FenceRole::Open);
        assert!(open.is_some());
        assert_eq!(advance_fence("let x = 1;", &mut open), FenceRole::Body);
        assert_eq!(advance_fence("```", &mut open), FenceRole::Close);
        assert!(open.is_none());
        assert_eq!(advance_fence("after", &mut open), FenceRole::Outside);
    }

    #[test]
    fn tracks_standalone_block_comment_roles() {
        let mut open = false;
        assert_eq!(
            advance_comment("before %%inline%%", &mut open),
            CommentRole::Outside
        );
        assert_eq!(advance_comment("%%", &mut open), CommentRole::Open);
        assert_eq!(advance_comment("comment body", &mut open), CommentRole::Body);
        assert_eq!(advance_comment("  %%  ", &mut open), CommentRole::Close);
        assert_eq!(advance_comment("after", &mut open), CommentRole::Outside);
        assert!(!open);
    }

    #[test]
    fn inline_text_uses_one_wrapping_job_and_keeps_actions() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let doc = parse_preview(
            "Hello %%secret%% world with `code` and [[Missing Note]] plus #tag [^1]",
        );
        let spans = doc
            .blocks
            .iter()
            .find_map(|block| match block {
                PreviewBlock::Paragraph { spans } => Some(spans.as_slice()),
                _ => None,
            })
            .expect("paragraph spans");

        let (job, hits) = build_inline_text_job(
            &vault,
            None,
            spans,
            theme::EDITOR_SIZE,
            false,
            0,
            true,
            false,
        );
        assert_eq!(
            job.text,
            "Hello %%secret%% world with code and Missing Note plus #tag [^1]"
        );
        assert!(hits.iter().any(|hit| matches!(hit.action, InlineHitAction::Wiki(_))));
        assert!(hits.iter().any(|hit| matches!(hit.action, InlineHitAction::Tag(_))));
        assert!(hits
            .iter()
            .any(|hit| matches!(hit.action, InlineHitAction::Footnote(_))));

        let (reading_job, _) = build_inline_text_job(
            &vault,
            None,
            spans,
            theme::EDITOR_SIZE,
            false,
            0,
            false,
            false,
        );
        assert_eq!(
            reading_job.text,
            "Hello  world with code and Missing Note plus #tag [^1]"
        );

        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let _ = ctx.run(Default::default(), |ctx| {
            let mut narrow = job.clone();
            narrow.wrap.max_width = 180.0;
            let galley = ctx.fonts(|fonts| fonts.layout_job(narrow));
            assert!(
                galley.rows.len() > 1,
                "the acceptance text must wrap (size={:?}, max_width={})",
                galley.size(),
                galley.job.wrap.max_width
            );
            assert!(
                galley.rows.len() < job.text.chars().count() / 3,
                "text collapsed into near-vertical rows: {} rows",
                galley.rows.len()
            );

            let wiki = hits
                .iter()
                .find(|hit| matches!(hit.action, InlineHitAction::Wiki(_)))
                .expect("wiki hit");
            let wiki_char = job.text[..wiki.bytes.start].chars().count();
            let glyph_center = |wanted: usize| {
                let mut seen = 0;
                for row in &galley.rows {
                    for glyph in &row.glyphs {
                        if seen == wanted {
                            return glyph.logical_rect().center();
                        }
                        seen += 1;
                    }
                    if row.ends_with_newline {
                        seen += 1;
                    }
                }
                panic!("missing glyph {wanted}");
            };
            assert!(matches!(
                hit_at_pointer(&galley, egui::Pos2::ZERO, glyph_center(wiki_char), &hits),
                Some(InlineHitAction::Wiki(_))
            ));
            assert!(
                hit_at_pointer(
                    &galley,
                    egui::Pos2::ZERO,
                    glyph_center(wiki_char - 1),
                    &hits,
                )
                .is_none(),
                "adjacent prose must not activate the wiki link"
            );
        });
    }

    #[test]
    fn text_runs_are_kept_around_non_text_widgets() {
        let spans = vec![
            Span::Text("before ".into()),
            Span::WikiLink {
                raw: "Note".into(),
                alias: None,
            },
            Span::InlineMath {
                formula: "x".into(),
            },
            Span::Text(" after ".into()),
            Span::Code("code".into()),
            Span::Embed {
                target: "image.png".into(),
                alias: None,
                width: None,
                height: None,
            },
            Span::Text(" tail".into()),
        ];
        assert_eq!(inline_text_run_end(&spans, 0), 2);
        assert_eq!(inline_text_run_end(&spans, 3), 5);
        assert_eq!(inline_text_run_end(&spans, 6), 7);
    }

    #[test]
    fn checklist_marker_roundtrip() {
        assert_eq!(
            parse_checklist("- [ ] buy milk"),
            Some((CheckMark::Unchecked, "buy milk"))
        );
        assert_eq!(
            parse_checklist("  * [x] done"),
            Some((CheckMark::Checked, "done"))
        );
        assert_eq!(
            toggle_checklist_line("- [ ] buy milk").as_deref(),
            Some("- [x] buy milk")
        );
        assert_eq!(
            toggle_checklist_line("- [x] buy milk").as_deref(),
            Some("- [ ] buy milk")
        );
        assert!(parse_checklist("- not a task").is_none());
    }

    #[test]
    fn numbered_checklist_marker_roundtrip_and_continue() {
        assert_eq!(
            parse_checklist("4. [ ] numbered task"),
            Some((CheckMark::Unchecked, "numbered task"))
        );
        assert_eq!(
            toggle_checklist_line("4. [ ] numbered task").as_deref(),
            Some("4. [x] numbered task")
        );
        let next = continue_list_marker("4. [x] numbered task").expect("numbered checklist");
        assert!(!next.empty_body);
        assert_eq!(next.next_prefix, "5. [ ] ");

        assert_eq!(
            toggle_checklist_line("4. [ ]body").as_deref(),
            Some("4. [x]body"),
            "toggle must not insert whitespace"
        );
        assert_eq!(
            toggle_checklist_line("  04. [X] ").as_deref(),
            Some("  04. [ ] "),
            "toggle must preserve indentation, spelling and trailing whitespace"
        );

        let max = continue_list_marker("18446744073709551615. [ ] task")
            .expect("arbitrarily large numbered checklist");
        assert_eq!(max.next_prefix, "18446744073709551616. [ ] ");
    }

    #[test]
    fn numbered_checklist_render_has_hanging_indent_and_inline_widgets() {
        fn find_text<'a>(
            shape: &'a egui::Shape,
            text: &str,
        ) -> Option<&'a egui::epaint::TextShape> {
            match shape {
                egui::Shape::Text(found) if found.galley.text() == text => Some(found),
                egui::Shape::Vec(children) => {
                    children.iter().find_map(|child| find_text(child, text))
                }
                _ => None,
            }
        }

        fn find_checkbox(shape: &egui::Shape) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Rect(found)
                    if (found.rect.width() - 16.0).abs() < f32::EPSILON
                        && (found.rect.height() - 16.0).abs() < f32::EPSILON =>
                {
                    Some(found.rect)
                }
                egui::Shape::Vec(children) => children.iter().find_map(find_checkbox),
                _ => None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let body = "A long numbered checklist body with `inline code`, [[Missing Note]], and enough trailing prose to wrap cleanly";
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(340.0, 240.0),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = render_checklist(
                    ui,
                    &vault,
                    None,
                    Some("4."),
                    CheckMark::Unchecked,
                    body,
                    true,
                );
            });
        });
        let body_shape = output
            .shapes
            .iter()
            .find_map(|clipped| find_text(&clipped.shape, "A long numbered checklist body with inline code, Missing Note, and enough trailing prose to wrap cleanly"))
            .expect("body text shape");
        let number_shape = output
            .shapes
            .iter()
            .find_map(|clipped| find_text(&clipped.shape, "4."))
            .expect("number text shape");
        let checkbox_rect = output
            .shapes
            .iter()
            .find_map(|clipped| find_checkbox(&clipped.shape))
            .expect("checkbox shape");
        let first_body_row_center = body_shape.pos.y
            + body_shape
                .galley
                .rows
                .first()
                .expect("body row")
                .rect
                .center()
                .y;
        let number_center = number_shape.pos.y
            + number_shape
                .galley
                .rows
                .first()
                .expect("number row")
                .rect
                .center()
                .y;
        assert!(
            (checkbox_rect.center().y - first_body_row_center).abs() <= 1.0,
            "checkbox center={} body first-row center={first_body_row_center}",
            checkbox_rect.center().y
        );
        assert!(
            (number_center - first_body_row_center).abs() <= 1.0,
            "number center={number_center} body first-row center={first_body_row_center}"
        );
        assert!(
            body_shape.galley.rows.len() > 1,
            "body must wrap in the narrow pane"
        );
        assert_eq!(
            body_shape
                .galley
                .job
                .sections
                .first()
                .map(|section| section.leading_space),
            Some(0.0),
            "body rows must start at the body column, not use a first-row-only indent"
        );
        assert!(
            body_shape.pos.x > 35.0,
            "number and checkbox must precede the body"
        );

        let mut mixed_height = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 160.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let top = ui.cursor().top();
                let _ = render_checklist(
                    ui,
                    &vault,
                    None,
                    Some("5."),
                    CheckMark::Unchecked,
                    "before $x$ after",
                    true,
                );
                mixed_height = ui.cursor().top() - top;
            });
        });
        assert!(
            mixed_height < 60.0,
            "text + inline math + text stacked vertically: height={mixed_height}"
        );
    }

    #[test]
    fn callout_bounds() {
        let lines = vec![
            "before".into(),
            "> [!note] Title".into(),
            "> body".into(),
            "> more".into(),
            "after".into(),
        ];
        let places = callout_places(&lines);
        assert_eq!(places[0], CalloutPlace::None);
        assert_eq!(places[1], CalloutPlace::Head { end: 4 });
        assert_eq!(places[2], CalloutPlace::Tail { start: 1, end: 4 });
        assert_eq!(places[3], CalloutPlace::Tail { start: 1, end: 4 });
        assert_eq!(places[4], CalloutPlace::None);
        assert_eq!(callout_type("> [!warning]"), Some("warning"));
        assert_eq!(callout_type("> plain"), None);

        let fenced = vec![
            "```".into(),
            "> [!note] inside".into(),
            "```".into(),
            "> [!danger] out".into(),
            "> body".into(),
        ];
        let places = callout_places(&fenced);
        assert_eq!(places[0], CalloutPlace::None);
        assert_eq!(places[1], CalloutPlace::None);
        assert_eq!(places[2], CalloutPlace::None);
        assert_eq!(places[3], CalloutPlace::Head { end: 5 });
        assert_eq!(places[4], CalloutPlace::Tail { start: 3, end: 5 });
    }

    #[test]
    fn callout_kind_aliases_and_unknown() {
        let cases = [
            ("note", CalloutKind::Note, false),
            ("NOTE", CalloutKind::Note, false),
            ("summary", CalloutKind::Abstract, false),
            ("tldr", CalloutKind::Abstract, false),
            ("info", CalloutKind::Info, false),
            ("todo", CalloutKind::Todo, false),
            ("hint", CalloutKind::Tip, false),
            ("important", CalloutKind::Tip, false),
            ("check", CalloutKind::Success, false),
            ("done", CalloutKind::Success, false),
            ("help", CalloutKind::Question, false),
            ("faq", CalloutKind::Question, false),
            ("caution", CalloutKind::Warning, false),
            ("attention", CalloutKind::Warning, false),
            ("fail", CalloutKind::Failure, false),
            ("missing", CalloutKind::Failure, false),
            ("error", CalloutKind::Danger, false),
            ("bug", CalloutKind::Bug, false),
            ("example", CalloutKind::Example, false),
            ("cite", CalloutKind::Quote, false),
            ("weird", CalloutKind::Note, true),
            ("custom_type", CalloutKind::Note, true),
        ];
        for (raw, kind, unknown) in cases {
            assert_eq!(callout_kind(raw), (kind, unknown), "raw={raw}");
        }
        let note = callout_visual(CalloutKind::Note);
        let danger = callout_visual(CalloutKind::Danger);
        assert_ne!(note.accent, danger.accent);
        assert_ne!(note.icon, danger.icon);
    }

    #[test]
    fn callout_fold_marker_round_trip() {
        assert!(!callout_collapsed("> [!note] Title"));
        assert!(callout_collapsed("> [!note]- Title"));
        assert_eq!(
            set_callout_collapsed("> [!note] Title", true).as_deref(),
            Some("> [!note]- Title")
        );
        assert_eq!(
            set_callout_collapsed("> [!note]- Title", false).as_deref(),
            Some("> [!note]+ Title")
        );
        assert!(set_callout_collapsed("> [!note]+ Title", false).is_none());
        let text = "> [!faq]- Q\n> A\n\n> [!note] open\n";
        let folded = set_all_callouts_collapsed(text, false).unwrap();
        assert!(folded.contains("> [!faq]+ Q"));
        assert!(folded.contains("> [!note] open"));
        let src = "> [!note]- T\n> body\nafter";
        assert!(expand_collapsed_callout_at(src, 1).unwrap().contains("+ T"));
        assert!(expand_collapsed_callout_at(src, 0).is_none());
        let fenced = "```\n> [!note] inside\n```\n> [!note] out\n";
        let folded = set_all_callouts_collapsed(fenced, true).unwrap();
        assert!(folded.contains("```\n> [!note] inside\n```"));
        assert!(folded.contains("> [!note]- out"));
        let fenced_lines: Vec<&str> = fenced.split('\n').collect();
        assert!(callout_at(&fenced_lines, 1).is_none());
        assert_eq!(callout_at(&fenced_lines, 3), Some((3, 4)));
    }

    #[test]
    fn continue_marker_on_empty_clears() {
        let c = continue_list_marker("- [ ] ").unwrap();
        assert!(c.empty_body);
        let c = continue_list_marker("- item").unwrap();
        assert!(!c.empty_body);
        assert_eq!(c.next_prefix, "- ");
    }

    #[test]
    fn list_line_predicate_and_indent() {
        assert!(is_list_line("- a"));
        assert!(is_list_line("* a"));
        assert!(is_list_line("1. a"));
        assert!(is_list_line("- [ ] a"));
        assert!(is_list_line("  - nested"));
        assert!(!is_list_line("text"));
        assert!(!is_list_line("> q"));

        assert_eq!(indent_list_line("- a"), "  - a");
        assert_eq!(unindent_list_line("  - a").as_deref(), Some("- a"));
        assert_eq!(unindent_list_line("- a"), None);
        assert_eq!(indent_list_line("3. x"), "  3. x");
        assert!(indent_list_line("3. x").contains("3. "));

        let text = "- a\nplain\n- b\n";
        let (next, lo, hi) =
            adjust_list_indent_in_range(text, 0, text.chars().count(), true).unwrap();
        assert_eq!(next, "  - a\nplain\n  - b\n");
        assert_eq!((lo, hi), (0, text.chars().count() + 4));
        let (back, _, _) =
            adjust_list_indent_in_range(&next, 0, next.chars().count(), false).unwrap();
        assert_eq!(back, text);
    }
}
