use bluelink_core::{parse_preview, split_wikilinks, PreviewBlock, PreviewDoc, Span, Vault};
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

/// `Some((mark, body))` when the line is a task item.
pub fn parse_checklist(line: &str) -> Option<(CheckMark, &str)> {
    let trimmed = line.trim_start();
    let after_bullet = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))?;
    if let Some(body) = after_bullet.strip_prefix("[ ]") {
        return Some((CheckMark::Unchecked, body.strip_prefix(' ').unwrap_or(body)));
    }
    if let Some(body) = after_bullet
        .strip_prefix("[x]")
        .or_else(|| after_bullet.strip_prefix("[X]"))
    {
        return Some((CheckMark::Checked, body.strip_prefix(' ').unwrap_or(body)));
    }
    None
}

/// Flip `[ ]` ↔ `[x]` on a checklist line. Returns `None` when not a task.
pub fn toggle_checklist_line(line: &str) -> Option<String> {
    let (mark, _) = parse_checklist(line)?;
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let bullet = if trimmed.starts_with("- ") {
        "- "
    } else if trimmed.starts_with("* ") {
        "* "
    } else {
        "+ "
    };
    let body = parse_checklist(line)?.1;
    let box_ = match mark {
        CheckMark::Unchecked => "[x]",
        CheckMark::Checked => "[ ]",
    };
    if body.is_empty() {
        Some(format!("{indent}{bullet}{box_}"))
    } else {
        Some(format!("{indent}{bullet}{box_} {body}"))
    }
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
        CalloutKind::Abstract => (Color32::from_rgb(0x00, 0xbf, 0xa5), regular::BOOK_OPEN_TEXT, 48),
        CalloutKind::Info => (theme::ACCENT, regular::INFO, 48),
        CalloutKind::Todo => (Color32::from_rgb(0x7c, 0x3a, 0xed), regular::CHECK_SQUARE, 48),
        CalloutKind::Tip => (Color32::from_rgb(0x00, 0xb5, 0xe2), regular::LIGHTBULB, 48),
        CalloutKind::Success => (Color32::from_rgb(0x12, 0xb9, 0x81), regular::CHECK_CIRCLE, 48),
        CalloutKind::Question => (Color32::from_rgb(0xea, 0xb3, 0x08), regular::QUESTION, 48),
        CalloutKind::Warning => (theme::WARN, regular::WARNING, 48),
        CalloutKind::Failure => (theme::DESTRUCTIVE, regular::X_CIRCLE, 48),
        CalloutKind::Danger => (theme::DESTRUCTIVE, regular::FIRE, 72),
        CalloutKind::Bug => (Color32::from_rgb(0xf4, 0x3f, 0x5e), regular::BUG, 48),
        CalloutKind::Example => (Color32::from_rgb(0xa8, 0x55, 0xf7), regular::FLASK, 48),
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
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || numbered_list_rest(trimmed).is_some()
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
    if let Some((mark, body)) = parse_checklist(line) {
        let empty = body.trim().is_empty();
        let bullet = if trimmed.starts_with("- ") {
            "- "
        } else if trimmed.starts_with("* ") {
            "* "
        } else {
            "+ "
        };
        let box_ = match mark {
            CheckMark::Unchecked => "[ ]",
            CheckMark::Checked => "[ ]", // new item always unchecked
        };
        return Some(ListContinue {
            empty_body: empty,
            next_prefix: format!("{indent}{bullet}{box_} "),
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
        let n: u64 = trimmed[..digits].parse().unwrap_or(1);
        return Some(ListContinue {
            empty_body: rest.trim().is_empty(),
            next_prefix: format!("{indent}{}. ", n + 1),
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
    lang: Option<&str>,
    preview: Option<&PreviewDoc>,
    // Live: true (muted glyphs). Reading mode (stage 3) can pass false to hide.
    show_comments: bool,
) -> LineClick {
    if !matches!(role, FenceRole::Outside) {
        return render_fence_stripe(ui, line, role, lang);
    }
    if let Some((mark, body)) = parse_checklist(line) {
        return render_checklist(ui, vault, source_path, mark, body, show_comments);
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
            render_text_with_wikis(ui, vault, source_path, line, theme::EDITOR_SIZE, false, 0),
        );
    }
    click
}

fn render_checklist(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    mark: CheckMark,
    body: &str,
    show_comments: bool,
) -> LineClick {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let side = 16.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
        let stroke = egui::Stroke::new(1.5_f32, theme::ACCENT);
        ui.painter()
            .rect_stroke(rect, theme::round_sm(), stroke, egui::StrokeKind::Inside);
        if matches!(mark, CheckMark::Checked) {
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
        let click = if resp.clicked() {
            LineClick::ToggleCheck
        } else {
            LineClick::None
        };
        let body_click = if body.is_empty() {
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
                render_text_with_wikis(ui, vault, source_path, body, theme::EDITOR_SIZE, false, 0)
            } else {
                render_spans(
                    ui,
                    vault,
                    source_path,
                    spans,
                    theme::EDITOR_SIZE,
                    false,
                    0,
                    show_comments,
                )
            }
        };
        merge_click(click, body_click)
    })
    .inner
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

fn highlight_code_line(line: &str, lang: Option<&str>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let base = TextFormat {
        font_id: FontId::new(16.0, FontFamily::Monospace),
        color: theme::TEXT,
        ..Default::default()
    };
    let kw = TextFormat {
        font_id: FontId::new(16.0, FontFamily::Monospace),
        color: theme::ACCENT,
        ..Default::default()
    };
    let words = keywords_for(lang);
    if words.is_empty() {
        job.append(line, 0.0, base);
        return job;
    }
    let mut rest = line;
    while !rest.is_empty() {
        let (tok, next) = next_code_token(rest);
        rest = next;
        let fmt = if is_ident(tok) && words.iter().any(|w| *w == tok) {
            kw.clone()
        } else {
            base.clone()
        };
        job.append(tok, 0.0, fmt);
    }
    job
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
) -> LineClick {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut click = LineClick::None;
        for span in split_wikilinks(text) {
            click = merge_click(
                click,
                render_one_span(ui, vault, source_path, &span, size, heading, level, true),
            );
        }
        click
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
) -> LineClick {
    let mut click = LineClick::None;
    for span in spans {
        click = merge_click(
            click,
            render_one_span(
                ui,
                vault,
                source_path,
                span,
                size,
                heading,
                level,
                show_comments,
            ),
        );
    }
    click
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
) -> LineClick {
    match span {
        Span::Text(t) => render_text_chunks(ui, t, size, heading, level),
        Span::Comment(body) => {
            if !show_comments {
                return LineClick::None;
            }
            // Paint delimiters so inactive-line width matches the buffer.
            let shown = format!("%%{body}%%");
            let resp = ui.add(
                egui::Label::new(RichText::new(shown).size(size).color(theme::WEAK))
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
                egui::Label::new(RichText::new(t).size(size).strong().color(theme::TEXT))
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
                egui::Label::new(RichText::new(t).size(size).italics().color(theme::TEXT))
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
                egui::Label::new(
                    RichText::new(t)
                        .size(size)
                        .strikethrough()
                        .color(theme::WEAK),
                )
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
                    56,
                ))
                .corner_radius(theme::round_sm())
                .inner_margin(egui::Margin::symmetric(3, 1))
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::Label::new(RichText::new(t).size(size).color(theme::TEXT))
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
                        egui::Label::new(
                            RichText::new(t).size(size).monospace().color(theme::TEXT),
                        )
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
            ui.hyperlink_to(RichText::new(text).size(size), dest);
            LineClick::None
        }
        Span::WikiLink { raw, alias } => {
            let target = bluelink_core::normalize_link_target(raw);
            let label = alias
                .as_deref()
                .unwrap_or_else(|| raw.split('|').next().unwrap_or(raw));
            let resolved = vault.index().resolve(raw, source_path);
            let base = if resolved.is_some() {
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
            let color = if hovered { lighten(base) } else { base };

            if hovered {
                ui.painter().rect_filled(
                    rect.expand2(egui::vec2(4.0, 2.0)),
                    theme::round_sm(),
                    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 40),
                );
            }
            let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font, color));
            ui.painter().galley(rect.min, galley, color);
            let underline_y = rect.bottom() - 1.0;
            ui.painter().hline(
                rect.left()..=rect.right(),
                underline_y,
                egui::Stroke::new(if hovered { 1.5_f32 } else { 1.0_f32 }, color),
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
                    return render_note_embed(ui, vault, &path, label);
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
fn render_text_chunks(ui: &mut Ui, text: &str, size: f32, heading: bool, level: u8) -> LineClick {
    let mut click = LineClick::None;
    for piece in crate::footnote::split_inline_footnotes(text) {
        match piece {
            crate::footnote::InlinePiece::Text(chunk) => {
                click = merge_click(click, render_tag_chunks(ui, chunk, size, heading, level));
            }
            crate::footnote::InlinePiece::Inline { text: inner } => {
                let resp = ui
                    .add(
                        egui::Label::new(
                            RichText::new("*")
                                .size(size * 0.85)
                                .color(theme::ACCENT),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_text(inner)
                    .on_hover_cursor(CursorIcon::Help);
                if resp.clicked() {
                    click = merge_click(click, LineClick::Edit);
                }
            }
            crate::footnote::InlinePiece::Ref { id } => {
                let resp = ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("[^{id}]"))
                                .size(size * 0.85)
                                .color(theme::ACCENT),
                        )
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
fn render_tag_chunks(ui: &mut Ui, text: &str, size: f32, heading: bool, level: u8) -> LineClick {
    let mut click = LineClick::None;
    let mut rest = text;
    while !rest.is_empty() {
        let Some(hash_at) = rest.find('#') else {
            click = merge_click(click, paint_plain_text(ui, rest, size, heading, level));
            break;
        };
        if hash_at > 0 {
            click = merge_click(
                click,
                paint_plain_text(ui, &rest[..hash_at], size, heading, level),
            );
            rest = &rest[hash_at..];
        }
        let after = &rest[1..];
        let tag_len = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
            .count();
        if tag_len == 0 {
            click = merge_click(click, paint_plain_text(ui, "#", size, heading, level));
            rest = after;
            continue;
        }
        let tag = &after[..tag_len];
        let resp = ui
            .add(
                egui::Label::new(
                    RichText::new(format!("#{tag}"))
                        .size(size)
                        .color(theme::ACCENT),
                )
                .sense(Sense::click()),
            )
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

fn paint_plain_text(ui: &mut Ui, text: &str, size: f32, heading: bool, level: u8) -> LineClick {
    if text.is_empty() {
        return LineClick::None;
    }
    let rich = if heading {
        theme::heading_rich(text, level)
    } else {
        RichText::new(text).size(size).color(theme::TEXT)
    };
    let resp = ui.add(egui::Label::new(rich).sense(Sense::click()));
    if resp.clicked() {
        LineClick::Edit
    } else {
        LineClick::None
    }
}

/// `![[Note]]` card: title + first few lines. Click opens the note.
fn render_note_embed(ui: &mut Ui, vault: &Vault, path: &str, alias: Option<&str>) -> LineClick {
    let content = vault.read(path).unwrap_or_default();
    let title = alias
        .map(str::to_string)
        .unwrap_or_else(|| bluelink_core::title_from_content(&content, path));
    let preview_lines: Vec<&str> = content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("---")
        })
        .take(4)
        .collect();

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
            for line in preview_lines {
                let resp = ui.add(
                    egui::Label::new(RichText::new(line).size(13.0).color(theme::WEAK))
                    .sense(Sense::click()),
                );
                open |= resp.clicked();
            }
        });
    if open {
        LineClick::Wiki {
            raw: path.trim_end_matches(".md").to_string(),
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
        let (next, lo, hi) = adjust_list_indent_in_range(text, 0, text.chars().count(), true).unwrap();
        assert_eq!(next, "  - a\nplain\n  - b\n");
        assert_eq!((lo, hi), (0, text.chars().count() + 4));
        let (back, _, _) = adjust_list_indent_in_range(&next, 0, next.chars().count(), false).unwrap();
        assert_eq!(back, text);
    }
}
