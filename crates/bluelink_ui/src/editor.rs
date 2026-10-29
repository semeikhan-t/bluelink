//! Line-based live preview: active line is raw Markdown, others are rendered.
//!
//! View modes: [`ViewMode::Live`] (default), [`ViewMode::Reading`] (preview only),
//! [`ViewMode::Source`] (full-buffer monospace). Ctrl+A enters Source with select-all.
//! BlueLink hotkeys: Ctrl+E = toggle reading, Ctrl+Shift+E = toggle source.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bluelink_core::{link_fragment, PreviewDoc, Vault};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{FontFamily, FontId, Id, Key, Order, TextEdit, Ui};

use crate::editor_menu::{self, EditorAction};
use crate::fold;
use crate::footnote;
use crate::math_block::{self, MathRole};
use crate::media;
use crate::mermaid::{self, MermaidRole};
use crate::note_history::NoteHistory;
use crate::preview;
use crate::table::{self, Place};
use crate::theme;

/// How the note body is shown. One shared buffer; modes only change the widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Live,
    Reading,
    Source,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Reading => "Reading",
            Self::Source => "Source",
        }
    }

    /// Reading ↔ Live; from Source goes to Reading.
    pub fn toggle_reading(self) -> Self {
        match self {
            Self::Reading => Self::Live,
            _ => Self::Reading,
        }
    }

    /// Source ↔ Live; from Reading goes to Source.
    pub fn toggle_source(self) -> Self {
        match self {
            Self::Source => Self::Live,
            _ => Self::Source,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PendingClipboard {
    #[default]
    None,
    Cut,
    Copy,
    Paste,
}

/// Focus target for [`NoteHistory`] burst grouping.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HistFocus {
    None,
    Line(usize),
    Doc,
}

impl PendingClipboard {
    fn from_action(action: &EditorAction) -> Self {
        match action {
            EditorAction::Cut => Self::Cut,
            EditorAction::Copy => Self::Copy,
            EditorAction::Paste => Self::Paste,
            _ => Self::None,
        }
    }

    fn dispatch(self, ui: &Ui) {
        let action = match self {
            Self::None => return,
            Self::Cut => EditorAction::Cut,
            Self::Copy => EditorAction::Copy,
            Self::Paste => EditorAction::Paste,
        };
        editor_menu::dispatch_clipboard(ui, &action);
    }
}

pub struct WikiClick {
    pub raw: String,
    pub new_tab: bool,
    /// Heading text or `^id` from `[[note#…]]`.
    pub fragment: Option<String>,
}

pub struct LiveResult {
    pub changed: bool,
    pub wikilink: Option<WikiClick>,
    /// Context-menu / command: host should open a file picker and attach.
    pub attach_file: bool,
    /// Status line hint from media ingest (errors / confirmation).
    pub status: Option<String>,
    /// Host can clear `scroll_to_line` after we applied the offset.
    pub scroll_consumed: bool,
    /// Tag click (`#tag`) — host opens quick switcher with this query.
    pub tag_query: Option<String>,
}

#[derive(Clone)]
struct CachedPreviewLine {
    fingerprint: u64,
    preview: Option<PreviewDoc>,
    height: f32,
}

#[derive(Clone)]
struct LiveLayoutCache {
    lines: Vec<CachedPreviewLine>,
}

impl LiveLayoutCache {
    fn for_lines(buffer: &str, ranges: &[(usize, usize)], roles: &[preview::FenceRole]) -> Self {
        let lines = ranges
            .iter()
            .enumerate()
            .map(|(i, &(start, end))| {
                let line = buffer.get(start..end).unwrap_or("");
                CachedPreviewLine {
                    fingerprint: line_fingerprint(line),
                    preview: None,
                    height: estimate_row_height(line, roles[i]),
                }
            })
            .collect();
        Self { lines }
    }
}

fn line_fingerprint(line: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

const TOP_PAD: f32 = 12.0;
/// 16px note text with a 24px baseline rhythm (Obsidian's 1.5 line height).
const ROW_GAP: f32 = 2.0;
/// Extra band so fast flings don't flash empty rows.
const OVERSCAN: f32 = 600.0;

/// Note-editor only. Wayland wheels often arrive as Point; `line_scroll_speed` only scales Line.
const NOTE_WHEEL_BOOST: f32 = 4.0;
const NOTE_TRACKPAD_BOOST: f32 = 1.5;

fn note_scroll_boost(raw_len: f32) -> f32 {
    if raw_len >= 8.0 {
        NOTE_WHEEL_BOOST
    } else {
        NOTE_TRACKPAD_BOOST
    }
}

/// Faster note scroll on Wayland Point wheels. Idle frames must not call `input_mut` —
/// rewriting input every hover frame contended with AccessKit/atspi and tripped Plasma ANR.
fn boost_note_scroll(ui: &Ui) {
    if !ui.rect_contains_pointer(ui.max_rect()) {
        return;
    }
    // Read first (no write). Bail before input_mut when there is nothing to boost.
    let (raw, point_only) = ui.ctx().input(|i| {
        let mut saw_point = false;
        let mut saw_line = false;
        for ev in &i.events {
            if let egui::Event::MouseWheel { unit, .. } = ev {
                match *unit {
                    egui::MouseWheelUnit::Point => saw_point = true,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => saw_line = true,
                }
            }
        }
        (i.raw_scroll_delta, saw_point && !saw_line)
    });
    if raw == egui::Vec2::ZERO || !point_only {
        return;
    }
    let boost = note_scroll_boost(raw.length());
    ui.ctx().input_mut(|i| {
        i.smooth_scroll_delta = raw * boost;
    });
}

/// Gap after row `i` (baked into stride; column uses `item_spacing.y = 0`).
fn gap_after_row(i: usize, n: usize, role: preview::FenceRole) -> f32 {
    if i + 1 >= n {
        return 0.0;
    }
    if matches!(role, preview::FenceRole::Outside) {
        ROW_GAP
    } else {
        0.0
    }
}

/// `prefix[i]` = y of row `i` start; `prefix[n]` = total content height (incl. top pad).
/// Folded lines take no height and no gap, so the scroll height matches what is drawn.
fn row_prefix_heights(heights: &[f32], roles: &[preview::FenceRole], hidden: &[bool]) -> Vec<f32> {
    let n = heights.len();
    debug_assert_eq!(n, roles.len());
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(TOP_PAD);
    for i in 0..n {
        let gone = hidden.get(i).copied().unwrap_or(false);
        let h = if gone { 0.0 } else { heights[i].max(1.0) };
        let gap = if gone {
            0.0
        } else {
            gap_after_row(i, n, roles[i])
        };
        prefix.push(prefix[i] + h + gap);
    }
    prefix
}

/// Half-open `[first, last)` of rows intersecting the viewport (± overscan).
fn visible_row_range(
    prefix: &[f32],
    viewport_min_y: f32,
    viewport_max_y: f32,
    force: Option<usize>,
) -> (usize, usize) {
    let n = prefix.len().saturating_sub(1);
    if n == 0 {
        return (0, 0);
    }
    let top = viewport_min_y - OVERSCAN;
    let bot = viewport_max_y + OVERSCAN;
    let mut first = 0usize;
    while first < n && prefix[first + 1] < top {
        first += 1;
    }
    let mut last = first;
    while last < n && prefix[last] <= bot {
        last += 1;
    }
    if let Some(i) = force {
        let i = i.min(n.saturating_sub(1));
        first = first.min(i);
        last = last.max(i + 1);
    }
    (first, last)
}

/// Row under a click in the editor margins. `local_y` is relative to the column top.
/// The top pad belongs to the first row; a click in a gap goes to the nearer row.
fn row_at_local_y(prefix: &[f32], heights: &[f32], local_y: f32) -> Option<usize> {
    let n = heights.len();
    if n == 0 || prefix.len() != n + 1 || local_y < 0.0 || local_y > prefix[n] {
        return None;
    }
    if local_y <= prefix[0] {
        return Some(0);
    }
    let mut lo = 0usize;
    let mut hi = n;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if prefix[mid] <= local_y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let row_bottom = prefix[lo] + heights[lo].max(1.0);
    if local_y <= row_bottom || lo + 1 == n {
        return Some(lo);
    }
    let next_top = prefix[lo + 1];
    let closer_to_next = local_y - row_bottom > next_top - local_y;
    if closer_to_next {
        Some(lo + 1)
    } else {
        Some(lo)
    }
}

/// Show live editor. `active_line` is the raw-editing line index (0-based).
/// `view_mode` selects Live / Reading / Source widgets over one buffer.
struct PaintedRow {
    height: f32,
    skip_gap: bool,
}

fn take_line_focus(ui: &Ui, line: usize) -> bool {
    ui.ctx().data_mut(|d| {
        let key = egui::Id::new(("live_focus", line));
        let on = d.get_temp::<bool>(key).unwrap_or(false);
        if on {
            d.insert_temp(key, false);
        }
        on
    })
}

/// Footnote field, visual table, callout, mermaid, or a resting math block.
/// `None` means this row is still a normal Markdown line.
fn paint_structured_row(
    ui: &mut Ui,
    i: usize,
    start: usize,
    end: usize,
    line: &str,
    lines: &[String],
    math_spans: &[math_block::MathSpan],
    mermaid_spans: &[mermaid::MermaidSpan],
    table_places: &[Place],
    callout_places: &[preview::CalloutPlace],
    footnote_places: &[footnote::FootnotePlace],
    ranges: &[(usize, usize)],
    active_line: Option<usize>,
    readonly: bool,
    splice: &mut Option<(usize, usize, String)>,
    table_edit: &mut Option<(usize, usize, String)>,
    request_active: &mut Option<usize>,
    leave_edit: &mut bool,
    erase_footnote: &mut Option<(String, usize, usize)>,
    footnote_jump: &mut Option<usize>,
) -> Option<PaintedRow> {
    let _ = end;
    match footnote_places
        .get(i)
        .copied()
        .unwrap_or(footnote::FootnotePlace::None)
    {
        footnote::FootnotePlace::Tail { .. } => {
            return Some(PaintedRow {
                height: 0.0,
                skip_gap: true,
            });
        }
        footnote::FootnotePlace::Head { end: block_end } => {
            let block = &lines[i..block_end];
            let Some((id, mut body)) = footnote::parse_block(block) else {
                return Some(PaintedRow {
                    height: 0.0,
                    skip_gap: true,
                });
            };
            let block_end_byte = ranges
                .get(block_end.saturating_sub(1))
                .map(|&(_, e)| e)
                .unwrap_or(start);
            let editing = !readonly && active_line.is_some_and(|a| (i..block_end).contains(&a));
            let top = ui.cursor().top();
            if editing {
                let want = take_line_focus(ui, i);
                let field = footnote::show_field(ui, &id, &mut body, want, true);
                if field.erase {
                    *erase_footnote = Some((id.clone(), start, block_end_byte));
                } else if field.changed {
                    *splice = Some((start, block_end_byte, footnote::format_block(&id, &body)));
                }
                if field.commit {
                    *leave_edit = true;
                }
                if field.jump_to_marker {
                    if let Some(line) = footnote::first_marker_line(&lines.join("\n"), &id) {
                        *footnote_jump = Some(line);
                        *request_active = Some(line);
                    }
                }
            } else {
                match footnote::paint_preview(ui, &id, &body) {
                    footnote::PreviewHit::Marker => {
                        if let Some(line) = footnote::first_marker_line(&lines.join("\n"), &id) {
                            *footnote_jump = Some(line);
                            *request_active = Some(line);
                        }
                    }
                    footnote::PreviewHit::Body if !readonly => {
                        *request_active = Some(i);
                    }
                    _ => {}
                }
            }
            let height = (ui.cursor().top() - top).max(28.0);
            return Some(PaintedRow {
                height,
                skip_gap: false,
            });
        }
        footnote::FootnotePlace::None => {}
    }

    match table_places.get(i).copied().unwrap_or(Place::None) {
        Place::Tail => {
            return Some(PaintedRow {
                height: 0.0,
                skip_gap: true,
            });
        }
        Place::Head { end: block_end } => {
            let Some(parsed) = lines.get(i..block_end).and_then(table::parse_slice) else {
                return Some(PaintedRow {
                    height: 0.0,
                    skip_gap: true,
                });
            };
            let want = take_line_focus(ui, i);
            let top = ui.cursor().top();
            if let Some(edit) = table::show(ui, i, &parsed, want && !readonly, !readonly) {
                if !readonly {
                    let mut next = parsed;
                    if table::apply_edit(&mut next, edit) {
                        if let (Some(&(b0, _)), Some(&(_, b1))) =
                            (ranges.get(i), ranges.get(block_end - 1))
                        {
                            *table_edit = Some((b0, b1, table::to_markdown(&next)));
                        }
                    }
                }
            }
            let height = (ui.cursor().top() - top).max(28.0);
            return Some(PaintedRow {
                height,
                skip_gap: false,
            });
        }
        Place::None => {}
    }

    match callout_places
        .get(i)
        .copied()
        .unwrap_or(preview::CalloutPlace::None)
    {
        preview::CalloutPlace::Tail {
            start,
            end: block_end,
        } => {
            let header = lines.get(start).map(String::as_str).unwrap_or("");
            let collapsed = preview::callout_collapsed(header);
            if !collapsed && active_line.is_some_and(|a| (start..block_end).contains(&a)) {
                return None;
            }
            return Some(PaintedRow {
                height: 0.0,
                skip_gap: true,
            });
        }
        preview::CalloutPlace::Head { end: block_end } => {
            let collapsed = preview::callout_collapsed(line);
            let editing = !collapsed && active_line.is_some_and(|a| (i..block_end).contains(&a));
            if !editing {
                let top = ui.cursor().top();
                let hit = preview::paint_callout(ui, lines, i, block_end);
                // A click on a collapsed title expands it; the chevron toggles either way.
                let expand_title = collapsed && hit.edit.is_some();
                if (hit.toggle_fold || expand_title) && !readonly {
                    if let Some(next) = preview::set_callout_collapsed(line, !collapsed) {
                        *splice = Some((start, end, next));
                    }
                } else if let Some(clicked) = hit.edit {
                    if !readonly {
                        *request_active = Some(clicked);
                    }
                }
                let height = (ui.cursor().top() - top).max(28.0);
                return Some(PaintedRow {
                    height,
                    skip_gap: false,
                });
            }
        }
        preview::CalloutPlace::None => {}
    }

    if let Some(span) = mermaid_spans.get(i).copied() {
        if span.role != MermaidRole::Outside {
            if active_line.is_some_and(|a| span.contains(a)) {
                return None;
            }
            if span.role != MermaidRole::Open {
                return Some(PaintedRow {
                    height: 0.0,
                    skip_gap: true,
                });
            }
            let top = ui.cursor().top();
            let (_, clicked) = mermaid::show_preview(ui, &mermaid::source(lines, span));
            if clicked && !readonly {
                let body = mermaid::body_line(span);
                let col = lines.get(body).map(|s| s.chars().count()).unwrap_or(0);
                *request_active = Some(body);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("live_focus", body)), true);
                    d.insert_temp(egui::Id::new(("live_cursor", body)), col);
                });
            }
            let height = (ui.cursor().top() - top).max(28.0);
            return Some(PaintedRow {
                height,
                skip_gap: false,
            });
        }
    }

    let Some(span) = math_spans.get(i).copied() else {
        return None;
    };
    if span.role == MathRole::Outside {
        return None;
    }
    if active_line.is_some_and(|a| span.contains(a)) {
        return None;
    }
    if span.role != MathRole::Open {
        return Some(PaintedRow {
            height: 0.0,
            skip_gap: true,
        });
    }
    let top = ui.cursor().top();
    let (_, clicked) = math_block::show_preview(ui, &math_block::formula(lines, span), true);
    if clicked && !readonly {
        let body = math_block::body_line(span);
        let col = lines.get(body).map(|s| s.chars().count()).unwrap_or(0);
        *request_active = Some(body);
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new(("live_focus", body)), true);
            d.insert_temp(egui::Id::new(("live_cursor", body)), col);
        });
    }
    let height = (ui.cursor().top() - top).max(28.0);
    Some(PaintedRow {
        height,
        skip_gap: false,
    })
}

fn apply_table_choice(
    buffer: &mut String,
    ranges: &[(usize, usize)],
    choice: table::MenuChoice,
    folds: &mut Vec<usize>,
) -> bool {
    let lines: Vec<String> = ranges
        .iter()
        .map(|&(s, e)| buffer.get(s..e).unwrap_or("").to_string())
        .collect();
    let Some(Place::Head { end }) = table::places(&lines).get(choice.start).copied() else {
        return false;
    };
    let Some(mut parsed) = lines.get(choice.start..end).and_then(table::parse_slice) else {
        return false;
    };
    let keep = table::mutate(&mut parsed, choice.row, choice.col, choice.action);
    let text = if keep {
        table::to_markdown(&parsed)
    } else {
        String::new()
    };
    let Some(&(b0, _)) = ranges.get(choice.start) else {
        return false;
    };
    let Some(&(_, b1)) = ranges.get(end - 1) else {
        return false;
    };
    let delta = fold::line_delta(buffer.get(b0..b1).unwrap_or(""), &text);
    fold::shift_lines(folds, choice.start + 1, delta);
    buffer.replace_range(b0..b1, &text);
    true
}

/// Insert `block` on its own lines. `caret_in_block` is a char offset into `block`.
fn insert_isolated_block(
    text: &mut String,
    primary: usize,
    secondary: usize,
    block: &str,
    caret_in_block: usize,
) -> usize {
    let lo = primary.min(secondary);
    let hi = primary.max(secondary);
    let byte_lo = editor_menu::char_to_byte(text, lo);
    let byte_hi = editor_menu::char_to_byte(text, hi);
    let lead = byte_lo > 0 && !text[..byte_lo].ends_with('\n');
    let tail = byte_hi < text.len() && !text[byte_hi..].starts_with('\n');
    let mut snippet = String::new();
    if lead {
        snippet.push('\n');
    }
    snippet.push_str(block);
    if tail {
        snippet.push('\n');
    }
    text.replace_range(byte_lo..byte_hi, &snippet);
    lo + usize::from(lead) + caret_in_block
}

pub fn show_live(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    buffer: &mut String,
    active_line: &mut Option<usize>,
    view_mode: &mut ViewMode,
    readonly: bool,
    scroll_to_line: Option<usize>,
    history: &mut NoteHistory,
    props_folded: &mut bool,
) -> LiveResult {
    let mut result = LiveResult {
        changed: false,
        wikilink: None,
        attach_file: false,
        status: None,
        scroll_consumed: false,
        tag_query: None,
    };

    // Footnote marker/def clicks queue a scroll for the next frame.
    let scroll_to_line = scroll_to_line.or_else(|| {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<usize>(Id::new("live_footnote_jump")))
    });

    // Disk-locked notes cannot enter Source (no typing). Reading still works.
    if readonly && *view_mode == ViewMode::Source {
        *view_mode = ViewMode::Live;
    }
    table::set_cell_focused(ui.ctx(), false);

    let fold_key = Id::new(("bluelink_folds", source_path.unwrap_or("")));
    let mut folds: Vec<usize> = ui.ctx().data(|d| d.get_temp(fold_key)).unwrap_or_default();

    // BlueLink: Ctrl+E = reading, Ctrl+Shift+E = source (not Obsidian-identical keymap).
    {
        let (toggle_reading, toggle_source) = ui.input(|i| {
            let cmd = i.modifiers.command && !i.modifiers.alt;
            (
                cmd && !i.modifiers.shift && i.key_pressed(Key::E),
                cmd && i.modifiers.shift && i.key_pressed(Key::E),
            )
        });
        if toggle_source && !readonly {
            history.end_burst(buffer);
            *view_mode = view_mode.toggle_source();
            *active_line = None;
        } else if toggle_reading {
            history.end_burst(buffer);
            *view_mode = view_mode.toggle_reading();
            *active_line = None;
        }
    }

    let can_edit = !readonly && *view_mode == ViewMode::Live;
    let show_comments = *view_mode != ViewMode::Reading;

    // Drops / clipboard images before TextEdit so paste does not also insert text.
    if can_edit {
        if let Some(note) = source_path {
            handle_dropped_files(
                ui,
                vault,
                note,
                buffer,
                *active_line,
                history,
                &mut result,
                &mut folds,
            );
            handle_clipboard_image_paste(
                ui,
                vault,
                note,
                buffer,
                *active_line,
                history,
                &mut result,
                &mut folds,
            );
        }
    }

    // Ctrl+A → Source with entire note selected.
    if can_edit && ui.input(|i| i.modifiers.command && i.key_pressed(Key::A)) {
        history.end_burst(buffer);
        // ponytail: Source uses a different ScrollArea; caret/scroll are not mapped back.
        *view_mode = ViewMode::Source;
        *active_line = None;
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("live_select_all"), true));
    }

    // Sync typing bursts with the active raw line (or full-doc session).
    {
        let hist_key = Id::new(("live_hist_focus", source_path.unwrap_or("")));
        let prev: Option<HistFocus> = ui.ctx().data(|d| d.get_temp(hist_key));
        let now = if *view_mode == ViewMode::Source {
            HistFocus::Doc
        } else if let Some(i) = *active_line {
            HistFocus::Line(i)
        } else {
            HistFocus::None
        };
        if prev != Some(now) {
            if prev.is_some_and(|p| p != HistFocus::None) {
                history.end_burst(buffer);
            }
            if now != HistFocus::None {
                history.begin_burst(buffer);
            }
            ui.ctx().data_mut(|d| d.insert_temp(hist_key, now));
        }
    }

    if *view_mode == ViewMode::Source {
        ui.ctx().data_mut(|d| d.insert_temp(fold_key, folds));
        return show_doc_edit(
            ui,
            vault,
            source_path,
            buffer,
            view_mode,
            active_line,
            history,
            props_folded,
            result,
        );
    }

    boost_note_scroll(ui);

    // Expanding a callout writes `+`/`-` into the note — only while editing.
    if can_edit {
        if let Some(target) = scroll_to_line {
            if let Some(next) = preview::expand_collapsed_callout_at(buffer, target) {
                history.checkpoint(buffer);
                *buffer = next;
                result.changed = true;
            }
        }
    }

    let mut ranges = line_ranges(buffer);
    if ranges.is_empty() {
        ranges.push((0, 0));
    }

    if let Some(i) = *active_line {
        if i >= ranges.len() {
            *active_line = Some(ranges.len().saturating_sub(1));
        }
    }

    let mut request_active: Option<usize> = None;
    let mut splice: Option<(usize, usize, String)> = None;
    let mut table_edit: Option<(usize, usize, String)> = None;
    let mut active_in_math = false;
    let mut active_in_mermaid = false;
    // Split active line at cursor: (start, end, left, right).
    let mut split_at_cursor: Option<(usize, usize, String, String)> = None;
    let mut merge_with_prev: Option<usize> = None;
    // After inserting a completed `[[link]]`, show it as preview (clickable).
    let mut leave_edit_after_wiki = false;
    let mut request_select_all = false;
    let mut erase_footnote: Option<(String, usize, usize)> = None;
    let mut footnote_jump: Option<usize> = None;
    let mut pending_clipboard = PendingClipboard::None;
    // Active line + char selection (primary, secondary) for Shift+↑/↓.
    let mut active_sel: Option<(usize, usize, usize)> = None;
    let mut structural_checkpoint = false;

    // Precompute fence roles so viewport culling can skip lines without breaking ``` state.
    let (fence_roles, fence_langs) = {
        let mut open = None;
        let mut cur_lang: Option<String> = None;
        let mut roles = Vec::with_capacity(ranges.len());
        let mut langs = Vec::with_capacity(ranges.len());
        for &(start, end) in &ranges {
            let line = buffer.get(start..end).unwrap_or("");
            let role = preview::advance_fence(line, &mut open);
            match role {
                preview::FenceRole::Open => {
                    cur_lang = preview::fence_lang(line);
                    langs.push(cur_lang.clone());
                }
                preview::FenceRole::Body | preview::FenceRole::Close => {
                    langs.push(cur_lang.clone());
                    if matches!(role, preview::FenceRole::Close) {
                        cur_lang = None;
                    }
                }
                preview::FenceRole::Outside => {
                    langs.push(None);
                    cur_lang = None;
                }
            }
            roles.push(role);
        }
        (roles, langs)
    };

    let (regions, hidden, jump_to) = {
        let fold_lines: Vec<&str> = ranges
            .iter()
            .map(|&(start, end)| buffer.get(start..end).unwrap_or(""))
            .collect();
        let in_fence: Vec<bool> = fence_roles
            .iter()
            .map(|role| !matches!(role, preview::FenceRole::Outside))
            .collect();
        let regions = fold::regions(&fold_lines, &in_fence);
        fold::retain_valid(&mut folds, &regions);
        if let Some(target) = scroll_to_line {
            fold::unfold_containing(&mut folds, &regions, target);
        }
        let mut hidden = fold::hidden(fold_lines.len(), &regions, &folds);
        // Closed YAML frontmatter is edited in the Properties panel — hide fence lines here.
        if let Some(n) = bluelink_core::frontmatter_line_count(buffer) {
            for i in 0..n.min(hidden.len()) {
                hidden[i] = true;
            }
        }
        for (i, tail) in preview::collapsed_callout_tails(&fold_lines)
            .into_iter()
            .enumerate()
        {
            if tail {
                hidden[i] = true;
            }
        }
        let jump_to: Vec<Option<usize>> = (0..fold_lines.len())
            .map(|i| {
                if hidden[i] {
                    fold::visible_anchor(&regions, &folds, i)
                        .or_else(|| preview::callout_at(&fold_lines, i).map(|(header, _)| header))
                } else {
                    None
                }
            })
            .collect();
        (regions, hidden, jump_to)
    };
    if let Some(i) = *active_line {
        if hidden.get(i).copied().unwrap_or(false) {
            *active_line = jump_to.get(i).copied().flatten();
        }
    }

    // Scroll target from prefix heights (virtual rows are not in the tessellator).
    let layout_id = Id::new(("live_layout", vault.root(), source_path.unwrap_or("")));
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt(("live_note_scroll", source_path.unwrap_or("")))
        .auto_shrink([false, false]);
    if let Some(target) = scroll_to_line {
        if let Some(cache) = ui.ctx().data(|d| d.get_temp::<LiveLayoutCache>(layout_id)) {
            if cache.lines.len() == ranges.len() {
                let heights: Vec<f32> = cache.lines.iter().map(|l| l.height.max(1.0)).collect();
                let prefix = row_prefix_heights(&heights, &fence_roles, &hidden);
                let idx = target.min(prefix.len().saturating_sub(1));
                let offset = prefix.get(idx).copied().unwrap_or(TOP_PAD);
                scroll_area = scroll_area.vertical_scroll_offset(offset);
                result.scroll_consumed = true;
            }
        }
    }

    scroll_area.show_viewport(ui, |ui, viewport| {
        // Viewport height at content start — used to fill empty space below lines
        // so clicks there hit a widget (ScrollArea empty chrome is not clickable).
        let viewport_h = viewport.height();

        // Keep parsed previews and measured heights across frames. Scrolling should not
        // re-run pulldown-cmark for every visible line on every repaint.
        let mut layout = ui.ctx().data(|d| d.get_temp::<LiveLayoutCache>(layout_id));
        if layout
            .as_ref()
            .map_or(true, |cache| cache.lines.len() != ranges.len())
        {
            layout = Some(LiveLayoutCache::for_lines(buffer, &ranges, &fence_roles));
        }
        let mut layout = layout.expect("live layout cache initialized");

        let heights: Vec<f32> = layout.lines.iter().map(|l| l.height.max(1.0)).collect();
        let prefix = row_prefix_heights(&heights, &fence_roles, &hidden);
        let content_top = ui.cursor().top();
        let pane = ui.available_rect_before_wrap();
        // Registered before the lines so text, links, and embeds stay on top.
        // Empty space at the same height (side margins and the tail of a short line)
        // still hits this widget and opens that row.
        let margin = if !can_edit {
            None
        } else {
            let total = *prefix.last().unwrap_or(&TOP_PAD);
            let rect = egui::Rect::from_min_max(
                egui::pos2(pane.left(), content_top),
                egui::pos2(pane.right(), content_top + total),
            );
            Some(
                ui.interact(
                    rect,
                    Id::new(("live_margin", source_path.unwrap_or(""))),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::Text),
            )
        };

        let mut column_top = content_top;
        let mut column_left = pane.left();
        centered_column(ui, |ui| {
            // Prefix heights are measured from this cursor, not the outer pane.
            column_top = ui.cursor().top();
            column_left = ui.cursor().left();
            // Gaps are baked into prefix strides — avoid egui item_spacing double-count.
            ui.spacing_mut().item_spacing.y = 0.0;

            let n = ranges.len();
            let force = scroll_to_line.filter(|&i| i < n).or_else(|| {
                if can_edit {
                    active_line.filter(|&i| i < n)
                } else {
                    None
                }
            });
            let (first, last) = visible_row_range(&prefix, viewport.min.y, viewport.max.y, force);

            ui.add_space(prefix[first]);

            let lines: Vec<String> = ranges
                .iter()
                .map(|&(s, e)| buffer.get(s..e).unwrap_or("").to_string())
                .collect();
            let math_spans = math_block::spans(&lines, &fence_roles);
            let mermaid_spans = mermaid::spans(&lines, &fence_roles, &fence_langs);
            let table_places = table::places(&lines);
            let callout_places = preview::callout_places(&lines);
            let footnote_places = footnote::places(&lines);
            if let Some(i) = *active_line {
                active_in_math = math_spans
                    .get(i)
                    .is_some_and(|span| span.role != MathRole::Outside);
                active_in_mermaid = mermaid_spans
                    .get(i)
                    .is_some_and(|span| span.role != MermaidRole::Outside);
            }

            let mut fold_clicked: Option<usize> = None;
            for i in first..last {
                if hidden.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let row_top = ui.cursor().top();
                if regions.iter().any(|region| region.start == i) {
                    let collapsed = folds.contains(&i);
                    let center = egui::pos2(column_left - 14.0, row_top + 11.0);
                    if fold_chevron(
                        ui,
                        Id::new(("fold_chevron", source_path.unwrap_or(""), i)),
                        center,
                        collapsed,
                    ) {
                        fold_clicked = Some(i);
                    }
                }
                let (start, end) = ranges[i];
                let fence_role = fence_roles[i];
                let fence_lang = fence_langs[i].as_deref();
                let in_fence = !matches!(fence_role, preview::FenceRole::Outside);
                let is_active = can_edit && *active_line == Some(i);

                let line = buffer.get(start..end).unwrap_or("");

                if let Some(painted) = paint_structured_row(
                    ui,
                    i,
                    start,
                    end,
                    line,
                    &lines,
                    &math_spans,
                    &mermaid_spans,
                    &table_places,
                    &callout_places,
                    &footnote_places,
                    &ranges,
                    *active_line,
                    !can_edit,
                    &mut splice,
                    &mut table_edit,
                    &mut request_active,
                    &mut leave_edit_after_wiki,
                    &mut erase_footnote,
                    &mut footnote_jump,
                ) {
                    layout.lines[i].height = painted.height;
                    if !painted.skip_gap {
                        let gap = gap_after_row(i, n, fence_role);
                        if gap > 0.0 {
                            ui.add_space(gap);
                        }
                    }
                    continue;
                }

                let row_id = egui::Id::new(("live_line", i));
                let mut measured_h;

                if is_active {
                    let mut draft = line.to_string();
                    let font = preview::edit_font_for(&line, fence_role);
                    // TextEdit floors height at interact_size (32px) — match preview row.
                    let row_h = if line.is_empty() {
                        22.0
                    } else {
                        ui.fonts(|f| f.row_height(&font))
                    };
                    ui.spacing_mut().interact_size.y = row_h;
                    let mut layouter = |ui: &Ui, text: &str, wrap_width: f32| {
                        layout_line_edit(ui, text, wrap_width)
                    };
                    let output = TextEdit::singleline(&mut draft)
                        .id(row_id)
                        .desired_width(f32::INFINITY)
                        .font(font)
                        .layouter(&mut layouter)
                        .frame(false)
                        .margin(egui::Margin::ZERO)
                        // Tab inserts indent instead of moving focus.
                        .lock_focus(true)
                        .show(ui);
                    let response = output.response;

                    let focus_key = egui::Id::new(("live_focus", i));
                    let want_focus = ui.ctx().data_mut(|d| {
                        let v = d.get_temp::<bool>(focus_key).unwrap_or(false);
                        if v {
                            d.insert_temp(focus_key, false);
                        }
                        v
                    });
                    if want_focus {
                        response.request_focus();
                    }

                    // Place cursor after merge-up / wiki insert / context-menu edit.
                    let cursor_key = egui::Id::new(("live_cursor", i));
                    if let Some(idx) = ui.ctx().data_mut(|d| d.remove_temp::<usize>(cursor_key)) {
                        let mut state = output.state.clone();
                        state
                            .cursor
                            .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
                        state.store(ui.ctx(), row_id);
                    }

                    // Deferred clipboard from preview-row / fill context menu.
                    let clip_key = egui::Id::new("live_pending_clipboard");
                    if response.has_focus() {
                        let clip = ui.ctx().data_mut(|d| {
                            d.remove_temp::<PendingClipboard>(clip_key)
                                .unwrap_or_default()
                        });
                        clip.dispatch(ui);
                    }

                    let enter = ui.input(|inp| inp.key_pressed(Key::Enter));
                    let backspace = ui.input(|inp| inp.key_pressed(Key::Backspace));
                    let focused = response.has_focus() || response.lost_focus();

                    let (mut primary, mut secondary) = output
                        .cursor_range
                        .map(|r| (r.primary.ccursor.index, r.secondary.ccursor.index))
                        .unwrap_or_else(|| {
                            let n = draft.chars().count();
                            (n, n)
                        });

                    // egui only inserts Tab in multiline — handle it for the live line.
                    // ponytail: Live has no multi-line selection; Source applies per list line in the range.
                    let tab_pressed = focused
                        && ui.input(|inp| {
                            inp.key_pressed(Key::Tab)
                                && !inp.modifiers.command
                                && !inp.modifiers.alt
                        });
                    if tab_pressed && splice.is_none() {
                        let unindent = ui.input(|inp| inp.modifiers.shift);
                        let mut applied = false;
                        if preview::is_list_line(&draft) {
                            if unindent {
                                if let Some(next) = preview::unindent_list_line(&draft) {
                                    let n = draft.chars().count() - next.chars().count();
                                    draft = next;
                                    primary = primary.saturating_sub(n);
                                    secondary = secondary.saturating_sub(n);
                                    applied = true;
                                }
                            } else {
                                draft = preview::indent_list_line(&draft);
                                let n = preview::LIST_INDENT.chars().count();
                                primary += n;
                                secondary += n;
                                applied = true;
                            }
                        } else if unindent {
                            let (p, s) = unindent_line(&mut draft, primary, secondary);
                            primary = p;
                            secondary = s;
                            applied = true;
                        } else {
                            // Non-list (paragraph or fence): insert a literal tab.
                            let lo = primary.min(secondary);
                            let hi = primary.max(secondary);
                            let b0 = editor_menu::char_to_byte(&draft, lo);
                            let b1 = editor_menu::char_to_byte(&draft, hi);
                            draft.replace_range(b0..b1, "\t");
                            primary = lo + 1;
                            secondary = primary;
                            applied = true;
                        }
                        if applied {
                            splice = Some((start, end, draft.clone()));
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new(("live_cursor", i)), primary);
                            });
                        }
                    }

                    // Format shortcuts when the line field has focus (not S/P/Z/F/A/W/T/G).
                    if focused && splice.is_none() {
                        let wrap = ui.input(|inp| {
                            if !inp.modifiers.command || inp.modifiers.shift || inp.modifiers.alt {
                                return None;
                            }
                            if inp.key_pressed(Key::B) {
                                Some(("**", "**"))
                            } else if inp.key_pressed(Key::I) {
                                Some(("*", "*"))
                            } else if inp.key_pressed(Key::K) {
                                Some(("[[", "]]"))
                            } else {
                                None
                            }
                        });
                        if let Some((before, after)) = wrap {
                            structural_checkpoint = true;
                            let caret = editor_menu::wrap_selection(
                                &mut draft, primary, secondary, before, after,
                            );
                            primary = caret;
                            secondary = caret;
                            splice = Some((start, end, draft.clone()));
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new(("live_cursor", i)), caret);
                            });
                        }
                    }

                    active_sel = Some((i, primary, secondary));
                    if response.secondary_clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            editor_menu::open_at(ui.ctx(), pos, Some(i), primary, secondary);
                        }
                    }

                    let byte_idx = draft
                        .char_indices()
                        .nth(primary)
                        .map(|(b, _)| b)
                        .unwrap_or(draft.len());

                    // Wiki autocomplete. Clicking a candidate steals TextEdit focus on
                    // pointer-down and only registers `clicked` on pointer-up next frame —
                    // keep the popup alive for a few frames after lost_focus so the pick lands.
                    let mut wiki_consumed_enter = false;
                    let wiki_open = open_wiki_query(&draft, byte_idx);
                    let grace_key = egui::Id::new(("wiki_ac_grace", i));
                    let mut grace: u8 = ui
                        .ctx()
                        .data_mut(|d| d.get_temp::<u8>(grace_key).unwrap_or(0));
                    if wiki_open.is_none() {
                        grace = 0;
                    } else if response.lost_focus() {
                        grace = 24; // ~0.4s at 60fps
                    } else if focused {
                        grace = 2;
                    } else if grace > 0 {
                        grace -= 1;
                    }
                    let show_wiki = wiki_open.is_some() && (focused || grace > 0);
                    if let Some((brack_at, query)) = wiki_open {
                        if show_wiki {
                            let suggest = parse_wiki_suggest_query(&query);
                            let (popup_title, candidates) = match &suggest {
                                WikiSuggestQuery::Notes { query } => (
                                    "Link to note",
                                    wiki_candidates(&vault.index().note_paths(), query),
                                ),
                                WikiSuggestQuery::Headings { note, prefix } => {
                                    let titles = wiki_target_headings(
                                        vault,
                                        source_path,
                                        note,
                                        buffer,
                                    );
                                    (
                                        "Heading",
                                        filter_frag_candidates(&titles, prefix),
                                    )
                                }
                                WikiSuggestQuery::Blocks { note, prefix } => {
                                    let ids = wiki_target_block_ids(
                                        vault,
                                        source_path,
                                        note,
                                        buffer,
                                    );
                                    ("Block", filter_frag_candidates(&ids, prefix))
                                }
                            };
                            let mut picked: Option<String> = None;
                            if !candidates.is_empty() {
                                egui::Area::new(Id::new(("wiki_ac", i)))
                                    .order(Order::Foreground)
                                    .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 4.0))
                                    .show(ui.ctx(), |ui| {
                                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                                            ui.set_min_width(220.0);
                                            ui.label(
                                                egui::RichText::new(popup_title)
                                                    .size(12.0)
                                                    .color(theme::WEAK),
                                            );
                                            for item in candidates.iter().take(12) {
                                                let label = match &suggest {
                                                    WikiSuggestQuery::Notes { .. } => {
                                                        item.trim_end_matches(".md").to_string()
                                                    }
                                                    WikiSuggestQuery::Headings { .. } => {
                                                        item.clone()
                                                    }
                                                    WikiSuggestQuery::Blocks { .. } => {
                                                        format!("^{item}")
                                                    }
                                                };
                                                let r = ui.selectable_label(false, &label);
                                                if r.hovered() || r.clicked() {
                                                    grace = 24;
                                                }
                                                if r.clicked() {
                                                    picked = Some(wiki_suggest_insert_token(
                                                        &suggest, item,
                                                    ));
                                                }
                                            }
                                        });
                                    });
                                if enter && focused {
                                    picked = Some(wiki_suggest_insert_token(
                                        &suggest,
                                        &candidates[0],
                                    ));
                                    wiki_consumed_enter = true;
                                }
                            }
                            if let Some(name) = picked {
                                // Keep a leading `!` so `![[…` becomes an embed on pick.
                                let open_at = if brack_at > 0
                                    && draft.as_bytes().get(brack_at - 1) == Some(&b'!')
                                {
                                    brack_at - 1
                                } else {
                                    brack_at
                                };
                                let bang = open_at < brack_at;
                                let prefix = &draft[..open_at];
                                let suffix = &draft[byte_idx..];
                                let open = if bang { "![[" } else { "[[" };
                                let inserted = format!("{prefix}{open}{name}]]{suffix}");
                                splice = Some((start, end, inserted));
                                // Drop raw edit so the link renders as a clickable preview.
                                leave_edit_after_wiki = true;
                            }
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(grace_key, grace));

                    // singleline TextEdit loses focus on Enter — newline, unless wiki picked.
                    let enter_here = enter && focused && !wiki_consumed_enter && splice.is_none();
                    if enter_here {
                        let left = draft[..byte_idx].to_string();
                        let right = draft[byte_idx..].to_string();
                        // Empty list/checklist item: strip marker. Else continue it.
                        if right.is_empty() {
                            if let Some(cont) = preview::continue_list_marker(&left) {
                                structural_checkpoint = true;
                                if cont.empty_body {
                                    splice = Some((start, end, cont.clear_to));
                                    leave_edit_after_wiki = true;
                                } else {
                                    split_at_cursor = Some((start, end, left, cont.next_prefix));
                                }
                            } else {
                                split_at_cursor = Some((start, end, left, right));
                            }
                        } else {
                            split_at_cursor = Some((start, end, left, right));
                        }
                    } else if splice.is_none() && response.changed() && draft != line {
                        // Typing the closing `]]` → switch to preview so the link is clickable.
                        let closed_wiki = open_wiki_query(&draft, byte_idx).is_none()
                            && draft.contains("[[")
                            && draft.contains("]]")
                            && open_wiki_query(&line, line.len()).is_some();
                        splice = Some((start, end, draft.clone()));
                        if closed_wiki {
                            leave_edit_after_wiki = true;
                        }
                    }

                    // Backspace at start of line → join onto previous line
                    // (draft == line means TextEdit did not consume the key).
                    let cursor_at_start = output
                        .cursor_range
                        .map(|r| r.primary.ccursor.index == 0 && r.secondary.ccursor.index == 0)
                        .unwrap_or(false);
                    if backspace
                        && i > 0
                        && cursor_at_start
                        && draft == line
                        && focused
                        && splice.is_none()
                    {
                        merge_with_prev = Some(i);
                    }
                    measured_h = response.rect.height().max(1.0);
                } else {
                    let preview = if in_fence {
                        None
                    } else {
                        let fingerprint = line_fingerprint(line);
                        let cached = &mut layout.lines[i];
                        if cached.preview.is_none() || cached.fingerprint != fingerprint {
                            cached.fingerprint = fingerprint;
                            cached.preview = Some(bluelink_core::parse_preview(line));
                            // Prefer underestimate until painted — overestimates feel like dead scroll.
                            cached.height = estimate_row_height(line, fence_role);
                        }
                        cached.preview.as_ref()
                    };
                    let row = ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.set_min_height(22.0);
                            preview::render_line_with_preview(
                                ui,
                                vault,
                                source_path,
                                line,
                                fence_role,
                                fence_lang,
                                preview,
                                show_comments,
                            )
                        },
                    );
                    let click = row.inner;
                    measured_h = row.response.rect.height().max(1.0);

                    if can_edit && row.response.secondary_clicked() {
                        let n = line.chars().count();
                        if let Some(pos) = row.response.interact_pointer_pos() {
                            editor_menu::open_at(ui.ctx(), pos, Some(i), n, n);
                        }
                        request_active = Some(i);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("live_focus", i)), true);
                        });
                    }

                    match click {
                        preview::LineClick::Wiki { raw, new_tab } => {
                            result.wikilink = Some(WikiClick {
                                fragment: link_fragment(&raw),
                                raw,
                                new_tab,
                            });
                        }
                        preview::LineClick::Tag { query } => {
                            result.tag_query = Some(query);
                        }
                        preview::LineClick::Footnote { id } => {
                            if let Some(line) = footnote::def_line(buffer, &id) {
                                footnote_jump = Some(line);
                                request_active = Some(line);
                            }
                        }
                        preview::LineClick::ToggleCheck if can_edit => {
                            if let Some(next) = preview::toggle_checklist_line(line) {
                                structural_checkpoint = true;
                                splice = Some((start, end, next));
                            }
                        }
                        preview::LineClick::ResizeEmbed {
                            target,
                            width,
                            height,
                        } if can_edit => {
                            history.checkpoint(buffer);
                            if media::rewrite_embed_size(buffer, Some(i), &target, width, height) {
                                result.changed = true;
                                if *active_line == Some(i) {
                                    history.begin_burst(buffer);
                                }
                            }
                        }
                        preview::LineClick::Edit if can_edit => {
                            request_active = Some(i);
                        }
                        _ => {}
                    }
                }

                let span = math_spans[i];
                let editing_math = active_line.is_some_and(|a| span.contains(a));
                if editing_math && span.role == MathRole::Close {
                    let (extra, _) =
                        math_block::show_preview(ui, &math_block::formula(&lines, span), false);
                    measured_h += extra;
                }
                let mspan = mermaid_spans[i];
                let editing_mermaid = active_line.is_some_and(|a| mspan.contains(a));
                if editing_mermaid && mspan.role == MermaidRole::Close {
                    let (extra, _) = mermaid::show_preview(ui, &mermaid::source(&lines, mspan));
                    measured_h += extra;
                }

                layout.lines[i].height = measured_h;
                let gap = gap_after_row(i, n, fence_role);
                if gap > 0.0 {
                    ui.add_space(gap);
                }
            }
            if let Some(i) = fold_clicked {
                let closed_now = if let Some(pos) = folds.iter().position(|&start| start == i) {
                    folds.remove(pos);
                    false
                } else {
                    folds.push(i);
                    true
                };
                if closed_now {
                    if let Some(active) = *active_line {
                        let covers = regions.iter().any(|region| {
                            region.start == i && region.start < active && active < region.end
                        });
                        if covers {
                            request_active = Some(i);
                        }
                    }
                }
            }
            let total = *prefix.last().unwrap_or(&TOP_PAD);
            let after = (total - prefix[last]).max(0.0);
            if after > 0.0 {
                ui.add_space(after);
            }
        });

        if let Some(resp) = margin {
            let primary = resp.clicked();
            let secondary = resp.secondary_clicked();
            if primary || secondary {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let local_y = pos.y - column_top;
                    if let Some(mut i) = row_at_local_y(&prefix, &heights, local_y) {
                        if hidden.get(i).copied().unwrap_or(false) {
                            i = jump_to.get(i).copied().flatten().unwrap_or(i);
                        }
                        let n = ranges
                            .get(i)
                            .and_then(|&(s, e)| buffer.get(s..e))
                            .map(|t| t.chars().count())
                            .unwrap_or(0);
                        // Left of the text column → start of the line. Anywhere else
                        // the click is past the glyphs, so the caret goes to the end.
                        let caret = if pos.x < column_left { 0 } else { n };
                        if secondary {
                            editor_menu::open_at(ui.ctx(), pos, Some(i), caret, caret);
                        }
                        request_active = Some(i);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("live_cursor", i)), caret);
                        });
                    }
                }
            }
        }

        ui.ctx().data_mut(|d| d.insert_temp(layout_id, layout));

        if can_edit {
            let used = (ui.cursor().top() - content_top).max(0.0);
            // Full editor width (not only the centered column) + at least the viewport.
            let fill_h = (viewport_h - used).max(120.0);
            let resp = ui.allocate_response(
                egui::vec2(ui.available_width(), fill_h),
                egui::Sense::click(),
            );
            if resp.clicked() || resp.secondary_clicked() {
                // Empty area under the note edits the line after the last one.
                let (i, created) = line_below_last(buffer);
                if created {
                    result.changed = true;
                }
                if resp.secondary_clicked() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        editor_menu::open_at(ui.ctx(), pos, Some(i), 0, 0);
                    }
                }
                request_active = Some(i);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new(("live_focus", i)), true);
                    d.insert_temp(egui::Id::new(("live_cursor", i)), 0);
                });
            }
        }
    });

    // One floating menu for the whole editor (not per-line egui context_menu).
    if can_edit {
        let menu_was_open = editor_menu::load_menu(ui.ctx()).is_some();
        let table_was_open = table::menu_open(ui.ctx());
        if let Some((menu, action)) = editor_menu::show_popup(ui.ctx()) {
            match action {
                EditorAction::SelectAll => {
                    request_select_all = true;
                }
                EditorAction::AttachFile => {
                    result.attach_file = true;
                    if let Some(i) = menu.line {
                        request_active = Some(i);
                    }
                }
                EditorAction::Paste => {
                    if let Some(i) = menu.line {
                        request_active = Some(i);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("live_focus", i)), true);
                        });
                    }
                    // Prefer clipboard image → embed; else OS text paste.
                    if let Some(note) = source_path {
                        if try_insert_clipboard_image(
                            vault,
                            note,
                            buffer,
                            menu.line.or(*active_line),
                            history,
                            &mut result,
                            &mut folds,
                        ) {
                            // Image consumed; skip RequestPaste.
                        } else {
                            pending_clipboard = PendingClipboard::Paste;
                        }
                    } else {
                        pending_clipboard = PendingClipboard::Paste;
                    }
                }
                EditorAction::Cut | EditorAction::Copy => {
                    if let Some(i) = menu.line {
                        request_active = Some(i);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("live_focus", i)), true);
                        });
                    }
                    pending_clipboard = PendingClipboard::from_action(&action);
                }
                EditorAction::InsertSnippet(snip)
                    if snip == math_block::SNIPPET || snip == table::SNIPPET =>
                {
                    history.checkpoint(buffer);
                    let (primary, secondary) = match menu.line {
                        Some(i) => (
                            line_char_offset(buffer, &ranges, i, menu.primary),
                            line_char_offset(buffer, &ranges, i, menu.secondary),
                        ),
                        None => (menu.primary, menu.secondary),
                    };
                    let caret_in = if snip == math_block::SNIPPET {
                        "$$\n".chars().count()
                    } else {
                        0
                    };
                    let nl_before = buffer.matches('\n').count() as i32;
                    let insert_line = menu.line.unwrap_or(0);
                    let caret = insert_isolated_block(buffer, primary, secondary, snip, caret_in);
                    let delta = buffer.matches('\n').count() as i32 - nl_before;
                    fold::shift_lines(&mut folds, insert_line + 1, delta);
                    let (line, col) = caret_line_col(buffer, caret);
                    request_active = Some(line);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("live_focus", line)), true);
                        d.insert_temp(egui::Id::new(("live_cursor", line)), col);
                    });
                    history.begin_burst(buffer);
                    result.changed = true;
                }
                // Definition belongs at the end of the note, not on this line.
                EditorAction::Footnote => {
                    history.checkpoint(buffer);
                    let (primary, secondary) = match menu.line {
                        Some(i) => (
                            line_char_offset(buffer, &ranges, i, menu.primary),
                            line_char_offset(buffer, &ranges, i, menu.secondary),
                        ),
                        None => (menu.primary, menu.secondary),
                    };
                    if let Some(caret) = editor_menu::apply_text_action(
                        buffer,
                        primary,
                        secondary,
                        &EditorAction::Footnote,
                    ) {
                        let (line, col) = caret_line_col(buffer, caret);
                        request_active = Some(line);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("live_focus", line)), true);
                            d.insert_temp(egui::Id::new(("live_cursor", line)), col);
                        });
                        history.begin_burst(buffer);
                        result.changed = true;
                    }
                }
                EditorAction::FoldMore
                | EditorAction::FoldLess
                | EditorAction::FoldAll
                | EditorAction::UnfoldAll => {
                    let line = menu.line.or(*active_line).unwrap_or(0);
                    if let Some(next) = apply_fold_action(buffer, &mut folds, line, &action) {
                        history.checkpoint(buffer);
                        *buffer = next;
                        history.begin_burst(buffer);
                        result.changed = true;
                    }
                }
                EditorAction::ToggleFoldProperties => {
                    *props_folded = !*props_folded;
                }
                EditorAction::ToggleReading => {
                    history.end_burst(buffer);
                    *view_mode = view_mode.toggle_reading();
                    *active_line = None;
                }
                EditorAction::ToggleSource => {
                    history.end_burst(buffer);
                    *view_mode = view_mode.toggle_source();
                    *active_line = None;
                }
                ref text_action => {
                    if let Some(i) = menu.line {
                        if let Some(&(s, e)) = ranges.get(i) {
                            let mut draft = buffer.get(s..e).unwrap_or("").to_string();
                            let n = draft.chars().count();
                            let primary = menu.primary.min(n);
                            let secondary = menu.secondary.min(n);
                            if let Some(caret) = editor_menu::apply_text_action(
                                &mut draft,
                                primary,
                                secondary,
                                text_action,
                            ) {
                                structural_checkpoint = true;
                                splice = Some((s, e, draft));
                                request_active = Some(i);
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(egui::Id::new(("live_focus", i)), true);
                                    d.insert_temp(egui::Id::new(("live_cursor", i)), caret);
                                });
                            }
                        }
                    }
                }
            }
        }

        if let Some(choice) = table::show_menu(ui.ctx()) {
            if table_edit.is_none() && !result.changed {
                history.checkpoint(buffer);
                if apply_table_choice(buffer, &ranges, choice, &mut folds) {
                    result.changed = true;
                }
            }
        }
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        if esc && !menu_was_open && !table_was_open && (active_in_math || active_in_mermaid) {
            *active_line = None;
        }
    }

    // Shift+↑/↓ → full-doc edit with selection extended by one line.
    // A focused table cell owns the arrows.
    let in_table_cell = table::cell_focused(ui.ctx());
    if can_edit && !request_select_all && !in_table_cell {
        let (shift_up, shift_down) = ui.input(|i| {
            let shift_only = i.modifiers.shift && !i.modifiers.command && !i.modifiers.alt;
            (
                shift_only && i.key_pressed(Key::ArrowUp),
                shift_only && i.key_pressed(Key::ArrowDown),
            )
        });
        if shift_up || shift_down {
            let last = ranges.len().saturating_sub(1);
            let (line_i, primary, secondary) = active_sel.unwrap_or_else(|| {
                let i = active_line.unwrap_or(if shift_down { 0 } else { last });
                let n = ranges
                    .get(i)
                    .and_then(|&(s, e)| buffer.get(s..e))
                    .map(|t| t.chars().count())
                    .unwrap_or(0);
                (i, n, n)
            });
            let anchor = line_char_offset(buffer, &ranges, line_i, secondary);
            let target_line = if shift_down {
                (line_i + 1).min(last)
            } else {
                line_i.saturating_sub(1)
            };
            let target_len = ranges
                .get(target_line)
                .and_then(|&(s, e)| buffer.get(s..e))
                .map(|t| t.chars().count())
                .unwrap_or(0);
            let target_col = primary.min(target_len);
            let moving = line_char_offset(buffer, &ranges, target_line, target_col);
            *view_mode = ViewMode::Source;
            *active_line = None;
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("live_select_range"), (anchor, moving));
            });
        }
    }

    // ↑/↓ move the active raw line (commit edits via splice above first).
    if can_edit && request_active.is_none() && !request_select_all && !in_table_cell {
        let (up, down) = ui.input(|i| {
            let no_chord = !i.modifiers.command && !i.modifiers.alt && !i.modifiers.shift;
            (
                no_chord && i.key_pressed(Key::ArrowUp),
                no_chord && i.key_pressed(Key::ArrowDown),
            )
        });
        if up || down {
            request_active = match (*active_line, up) {
                (Some(i), true) => fold::step_visible(&hidden, i, false).or(Some(i)),
                (Some(i), false) => fold::step_visible(&hidden, i, true).or(Some(i)),
                (None, false) => fold::first_visible(&hidden),
                (None, true) => fold::last_visible(&hidden),
            };
        }
    }

    if pending_clipboard != PendingClipboard::None {
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new("live_pending_clipboard"), pending_clipboard);
        });
    }

    if let Some(i) = request_active {
        *active_line = Some(i);
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("live_focus", i)), true));
    }

    if leave_edit_after_wiki {
        *active_line = None;
    }

    // Typing splices stay in the open burst; structural edits get their own step.
    if structural_checkpoint || erase_footnote.is_some() || table_edit.is_some() {
        history.checkpoint(buffer);
    }

    if let Some(line) = footnote_jump {
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("live_footnote_jump"), line));
    }

    if let Some((id, start, end)) = erase_footnote {
        let at = line_index_at(&ranges, start);
        let breaks_before = buffer.matches('\n').count() as i32;
        let caret = footnote::erase(buffer, &id, start, end);
        let delta = buffer.matches('\n').count() as i32 - breaks_before;
        fold::shift_lines(&mut folds, at, delta);
        let (line, col) = caret_line_col(buffer, caret);
        *active_line = Some(line);
        ui.ctx().data_mut(|d| {
            d.insert_temp(egui::Id::new(("live_focus", line)), true);
            d.insert_temp(egui::Id::new(("live_cursor", line)), col);
        });
        history.begin_burst(buffer);
        result.changed = true;
    } else if let Some((start, end, left, right)) = split_at_cursor {
        let i = active_line.unwrap_or(0);
        let new_text = format!("{left}\n{right}");
        shift_folds_for_replace(&mut folds, &ranges, start, end, &new_text, buffer);
        buffer.replace_range(start..end, &new_text);
        *active_line = Some(i + 1);
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("live_focus", i + 1)), true));
        // Caret on the new line — place after continued list marker when present.
        let col = right.chars().count();
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("live_cursor", i + 1)), col));
        history.begin_burst(buffer);
        result.changed = true;
    } else if let Some((start, end, new_text)) = table_edit {
        shift_folds_for_replace(&mut folds, &ranges, start, end, &new_text, buffer);
        buffer.replace_range(start..end, &new_text);
        result.changed = true;
    } else if let Some((start, end, new_text)) = splice {
        shift_folds_for_replace(&mut folds, &ranges, start, end, &new_text, buffer);
        buffer.replace_range(start..end, &new_text);
        if structural_checkpoint {
            history.begin_burst(buffer);
        }
        result.changed = true;
    }

    if let Some(i) = merge_with_prev {
        let ranges = line_ranges(buffer);
        if i > 0 {
            if let Some(&(ps, pe)) = ranges.get(i - 1) {
                if pe < buffer.len() && buffer.as_bytes()[pe] == b'\n' {
                    let join_at = buffer[ps..pe].chars().count();
                    let at = line_index_at(&ranges, pe);
                    fold::shift_lines(&mut folds, at + 1, fold::line_delta("\n", ""));
                    buffer.replace_range(pe..pe + 1, "");
                    *active_line = Some(i - 1);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("live_focus", i - 1)), true);
                        d.insert_temp(egui::Id::new(("live_cursor", i - 1)), join_at);
                    });
                    result.changed = true;
                }
            }
        }
    }

    if request_select_all {
        *view_mode = ViewMode::Source;
        *active_line = None;
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("live_select_all"), true));
    }

    ui.ctx().data_mut(|d| d.insert_temp(fold_key, folds));
    result
}

fn show_doc_edit(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    buffer: &mut String,
    view_mode: &mut ViewMode,
    active_line: &mut Option<usize>,
    history: &mut NoteHistory,
    props_folded: &mut bool,
    mut result: LiveResult,
) -> LiveResult {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        history.end_burst(buffer);
        *view_mode = ViewMode::Live;
        *active_line = None;
        return result;
    }

    let id = egui::Id::new("live_doc_edit");
    let font = FontId::new(theme::EDITOR_SIZE, FontFamily::Monospace);
    // Captured before TextEdit mutates the buffer.
    let newlines_before = buffer.matches('\n').count();

    boost_note_scroll(ui);

    // Steal Tab before multiline TextEdit inserts `\t` into a list line.
    let mut pending_list_tab: Option<(bool, usize, usize)> = None;
    let source_focused = ui.memory(|m| m.has_focus(id));
    if source_focused {
        let (p, s) = egui::text_edit::TextEditState::load(ui.ctx(), id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| (r.primary.index, r.secondary.index))
            .unwrap_or_else(|| {
                let n = buffer.chars().count();
                (n, n)
            });
        let wants_tab = ui
            .input(|inp| inp.key_pressed(Key::Tab) && !inp.modifiers.command && !inp.modifiers.alt);
        if wants_tab {
            let deeper = !ui.input(|inp| inp.modifiers.shift);
            if preview::adjust_list_indent_in_range(buffer, p, s, deeper).is_some() {
                let mods = if deeper {
                    egui::Modifiers::NONE
                } else {
                    egui::Modifiers::SHIFT
                };
                ui.input_mut(|i| {
                    i.consume_key(mods, Key::Tab);
                });
                pending_list_tab = Some((deeper, p, s));
            }
        }
    }

    let mut caret_primary = 0usize;
    let mut caret_secondary = 0usize;
    let mut focused = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            centered_column(ui, |ui| {
                ui.add_space(12.0);
                let output = TextEdit::multiline(buffer)
                    .id(id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .font(font)
                    .frame(false)
                    .margin(egui::Margin::ZERO)
                    // Tab / Shift+Tab indent instead of changing focus.
                    .lock_focus(true)
                    .show(ui);

                let response = output.response;
                focused = response.has_focus() || response.lost_focus();

                let select_all = ui.ctx().data_mut(|d| {
                    let v = d
                        .get_temp::<bool>(egui::Id::new("live_select_all"))
                        .unwrap_or(false);
                    if v {
                        d.insert_temp(egui::Id::new("live_select_all"), false);
                    }
                    v
                });
                let select_range = ui.ctx().data_mut(|d| {
                    d.remove_temp::<(usize, usize)>(egui::Id::new("live_select_range"))
                });

                if select_all {
                    let end = CCursor::new(buffer.chars().count());
                    let mut state = output.state.clone();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
                    state.store(ui.ctx(), id);
                    response.request_focus();
                } else if let Some((anchor, moving)) = select_range {
                    let mut state = output.state.clone();
                    state.cursor.set_char_range(Some(CCursorRange::two(
                        CCursor::new(anchor),
                        CCursor::new(moving),
                    )));
                    state.store(ui.ctx(), id);
                    response.request_focus();
                }

                let (primary, secondary) = output
                    .cursor_range
                    .map(|r| (r.primary.ccursor.index, r.secondary.ccursor.index))
                    .unwrap_or_else(|| {
                        let n = buffer.chars().count();
                        (n, n)
                    });
                caret_primary = primary;
                caret_secondary = secondary;

                if response.secondary_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        editor_menu::open_at(ui.ctx(), pos, None, primary, secondary);
                    }
                }

                if response.changed() {
                    result.changed = true;
                    // ponytail: this path has no edit span, so shift_lines cannot
                    // run. A net newline change drops session folds. A paste that
                    // keeps the same newline count can still leave them on the
                    // wrong rows until retain_valid.
                    let newlines_after = buffer.matches('\n').count();
                    if newlines_before != newlines_after {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                Id::new(("bluelink_folds", source_path.unwrap_or(""))),
                                Vec::<usize>::new(),
                            );
                        });
                    }
                }
            });
        });

    if focused {
        let wrap = ui.input(|inp| {
            if !inp.modifiers.command || inp.modifiers.shift || inp.modifiers.alt {
                return None;
            }
            if inp.key_pressed(Key::B) {
                Some(("**", "**"))
            } else if inp.key_pressed(Key::I) {
                Some(("*", "*"))
            } else if inp.key_pressed(Key::K) {
                Some(("[[", "]]"))
            } else {
                None
            }
        });
        if let Some((before, after)) = wrap {
            history.checkpoint(buffer);
            let caret =
                editor_menu::wrap_selection(buffer, caret_primary, caret_secondary, before, after);
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
            state.store(ui.ctx(), id);
            history.begin_burst(buffer);
            result.changed = true;
        }
    }

    if let Some((deeper, p, s)) = pending_list_tab {
        if let Some((next, new_p, new_s)) =
            preview::adjust_list_indent_in_range(buffer, p, s, deeper)
        {
            history.checkpoint(buffer);
            *buffer = next;
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(new_p),
                CCursor::new(new_s),
            )));
            state.store(ui.ctx(), id);
            history.begin_burst(buffer);
            result.changed = true;
        }
    }

    if let Some((menu, action)) = editor_menu::show_popup(ui.ctx()) {
        match action {
            EditorAction::SelectAll => {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("live_select_all"), true));
            }
            EditorAction::AttachFile => {
                result.attach_file = true;
            }
            EditorAction::Paste => {
                if let Some(note) = source_path {
                    let fold_key = Id::new(("bluelink_folds", source_path.unwrap_or("")));
                    let mut folds: Vec<usize> =
                        ui.ctx().data(|d| d.get_temp(fold_key)).unwrap_or_default();
                    if !try_insert_clipboard_image(
                        vault,
                        note,
                        buffer,
                        None,
                        history,
                        &mut result,
                        &mut folds,
                    ) {
                        editor_menu::dispatch_clipboard(ui, &EditorAction::Paste);
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(fold_key, folds));
                } else {
                    editor_menu::dispatch_clipboard(ui, &EditorAction::Paste);
                }
            }
            EditorAction::Cut | EditorAction::Copy => {
                editor_menu::dispatch_clipboard(ui, &action);
            }
            EditorAction::FoldMore
            | EditorAction::FoldLess
            | EditorAction::FoldAll
            | EditorAction::UnfoldAll => {
                let fold_key = Id::new(("bluelink_folds", source_path.unwrap_or("")));
                let mut folds: Vec<usize> =
                    ui.ctx().data(|d| d.get_temp(fold_key)).unwrap_or_default();
                let (line, _) = caret_line_col(buffer, caret_primary);
                if let Some(next) = apply_fold_action(buffer, &mut folds, line, &action) {
                    history.checkpoint(buffer);
                    *buffer = next;
                    history.begin_burst(buffer);
                    result.changed = true;
                }
                ui.ctx().data_mut(|d| d.insert_temp(fold_key, folds));
            }
            EditorAction::ToggleFoldProperties => {
                *props_folded = !*props_folded;
            }
            EditorAction::ToggleReading => {
                history.end_burst(buffer);
                *view_mode = view_mode.toggle_reading();
                *active_line = None;
            }
            EditorAction::ToggleSource => {
                history.end_burst(buffer);
                *view_mode = view_mode.toggle_source();
                *active_line = None;
            }
            ref text_action => {
                history.checkpoint(buffer);
                if let Some(caret) = editor_menu::apply_text_action(
                    buffer,
                    menu.primary,
                    menu.secondary,
                    text_action,
                ) {
                    let mut state =
                        egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
                    state.store(ui.ctx(), id);
                    history.begin_burst(buffer);
                    result.changed = true;
                }
            }
        }
    }

    result
}

fn handle_dropped_files(
    ui: &Ui,
    vault: &Vault,
    active_note: &str,
    buffer: &mut String,
    active_line: Option<usize>,
    history: &mut NoteHistory,
    result: &mut LiveResult,
    folds: &mut Vec<usize>,
) {
    let dropped = ui
        .ctx()
        .input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
    if dropped.is_empty() {
        return;
    }
    let mut embedded = 0usize;
    let mut checked = false;
    for file in dropped {
        let Some(path) = file.path.as_deref() else {
            // Web / in-memory drops: use bytes + name when present.
            if let (Some(bytes), name) = (file.bytes.as_ref(), file.name.as_str()) {
                let ext = std::path::Path::new(name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !bluelink_core::is_media_extension(ext) {
                    continue;
                }
                let stem = std::path::Path::new(name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("attachment");
                match media::save_bytes(vault, stem, ext, bytes, active_note) {
                    Ok(rel) => {
                        if !checked {
                            history.checkpoint(buffer);
                            checked = true;
                        }
                        let grew = embed_insert_line(buffer, active_line);
                        media::insert_embed(buffer, active_line, &rel);
                        if let Some(i) = grew {
                            fold::shift_lines(folds, i + 1, 1);
                        }
                        embedded += 1;
                        result.changed = true;
                    }
                    Err(e) => result.status = Some(format!("attach failed: {e}")),
                }
            }
            continue;
        };
        match media::ingest_path(vault, path, active_note) {
            Ok(rel) => {
                if !checked {
                    history.checkpoint(buffer);
                    checked = true;
                }
                let grew = embed_insert_line(buffer, active_line);
                media::insert_embed(buffer, active_line, &rel);
                if let Some(i) = grew {
                    fold::shift_lines(folds, i + 1, 1);
                }
                embedded += 1;
                result.changed = true;
            }
            Err(e) => result.status = Some(format!("drop failed: {e}")),
        }
    }
    if embedded > 0 {
        history.begin_burst(buffer);
        if result.status.is_none() {
            result.status = Some(if embedded == 1 {
                "attached file".into()
            } else {
                format!("attached {embedded} files")
            });
        }
    }
}

fn handle_clipboard_image_paste(
    ui: &Ui,
    vault: &Vault,
    active_note: &str,
    buffer: &mut String,
    active_line: Option<usize>,
    history: &mut NoteHistory,
    result: &mut LiveResult,
    folds: &mut Vec<usize>,
) {
    let wants_paste = ui.input(|i| {
        let key = i.modifiers.command && i.key_pressed(Key::V) && !i.modifiers.shift;
        let event = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
        key || event
    });
    if !wants_paste {
        return;
    }
    if !try_insert_clipboard_image(
        vault,
        active_note,
        buffer,
        active_line,
        history,
        result,
        folds,
    ) {
        return;
    }
    // Swallow text paste so TextEdit does not also insert clipboard text.
    ui.ctx().input_mut(|i| {
        i.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
        let _ = i.consume_key(egui::Modifiers::COMMAND, Key::V);
    });
}

fn try_insert_clipboard_image(
    vault: &Vault,
    active_note: &str,
    buffer: &mut String,
    active_line: Option<usize>,
    history: &mut NoteHistory,
    result: &mut LiveResult,
    folds: &mut Vec<usize>,
) -> bool {
    let Some(png) = media::clipboard_image_png() else {
        return false;
    };
    let stem = media::pasted_image_stem();
    match media::save_bytes(vault, &stem, "png", &png, active_note) {
        Ok(rel) => {
            history.checkpoint(buffer);
            let grew = embed_insert_line(buffer, active_line);
            media::insert_embed(buffer, active_line, &rel);
            if let Some(i) = grew {
                fold::shift_lines(folds, i + 1, 1);
            }
            history.begin_burst(buffer);
            result.changed = true;
            result.status = Some(format!("pasted {rel}"));
            true
        }
        Err(e) => {
            result.status = Some(format!("paste image failed: {e}"));
            true // still consume — image was present
        }
    }
}

/// Unclosed `[[query` before `cursor_byte`. Returns byte index of `[[` and query.
fn open_wiki_query(draft: &str, cursor_byte: usize) -> Option<(usize, String)> {
    let cursor_byte = cursor_byte.min(draft.len());
    let before = &draft[..cursor_byte];
    let start = before.rfind("[[")?;
    let after = &before[start + 2..];
    if after.contains("]]") {
        return None;
    }
    let query = after.split('|').next().unwrap_or(after).to_string();
    Some((start, query))
}

/// Parsed wiki-suggest query: note paths, headings of a note, or block ids.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WikiSuggestQuery {
    Notes { query: String },
    Headings { note: String, prefix: String },
    Blocks { note: String, prefix: String },
}

/// Split `Note` / `Note#H` / `Note#^id` / `Note^id` (alias already stripped).
fn parse_wiki_suggest_query(query: &str) -> WikiSuggestQuery {
    if let Some((note, frag)) = query.split_once('#') {
        if let Some(id) = frag.strip_prefix('^') {
            return WikiSuggestQuery::Blocks {
                note: note.to_string(),
                prefix: id.to_string(),
            };
        }
        return WikiSuggestQuery::Headings {
            note: note.to_string(),
            prefix: frag.to_string(),
        };
    }
    // Plan also accepts `[[Note^id` without `#`.
    if let Some((note, id)) = query.split_once('^') {
        return WikiSuggestQuery::Blocks {
            note: note.to_string(),
            prefix: id.to_string(),
        };
    }
    WikiSuggestQuery::Notes {
        query: query.to_string(),
    }
}

fn wiki_candidates(notes: &[String], query: &str) -> Vec<String> {
    let q = query.to_ascii_lowercase();
    let mut out: Vec<String> = notes
        .iter()
        .filter(|p| {
            let path_no = p.trim_end_matches(".md");
            let stem = path_no.rsplit('/').next().unwrap_or(path_no);
            q.is_empty()
                || stem.to_ascii_lowercase().contains(&q)
                || path_no.to_ascii_lowercase().contains(&q)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    out
}

fn filter_frag_candidates(items: &[String], prefix: &str) -> Vec<String> {
    let q = prefix.to_ascii_lowercase();
    let mut out: Vec<String> = items
        .iter()
        .filter(|t| q.is_empty() || t.to_ascii_lowercase().contains(&q))
        .cloned()
        .collect();
    out.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    out
}

/// Token inserted inside `[[…]]` for a suggest pick.
fn wiki_suggest_insert_token(suggest: &WikiSuggestQuery, item: &str) -> String {
    match suggest {
        WikiSuggestQuery::Notes { .. } => item.trim_end_matches(".md").to_string(),
        WikiSuggestQuery::Headings { note, .. } => {
            if note.is_empty() {
                format!("#{item}")
            } else {
                format!("{note}#{item}")
            }
        }
        WikiSuggestQuery::Blocks { note, .. } => {
            if note.is_empty() {
                format!("#^{item}")
            } else {
                format!("{note}#^{item}")
            }
        }
    }
}

fn wiki_target_text(
    vault: &Vault,
    source_path: Option<&str>,
    note: &str,
    live_buffer: &str,
) -> Option<String> {
    let path = vault.index().resolve(note, source_path)?;
    if source_path == Some(path.as_str()) {
        Some(live_buffer.to_string())
    } else {
        vault.read(&path).ok()
    }
}

fn wiki_target_headings(
    vault: &Vault,
    source_path: Option<&str>,
    note: &str,
    live_buffer: &str,
) -> Vec<String> {
    let Some(text) = wiki_target_text(vault, source_path, note, live_buffer) else {
        return Vec::new();
    };
    crate::outline::headings(&text)
        .into_iter()
        .map(|e| e.title)
        .collect()
}

fn wiki_target_block_ids(
    vault: &Vault,
    source_path: Option<&str>,
    note: &str,
    live_buffer: &str,
) -> Vec<String> {
    let Some(text) = wiki_target_text(vault, source_path, note, live_buffer) else {
        return Vec::new();
    };
    collect_block_ids(&text)
}

/// Block ids as scanned by [`bluelink_core::block_line`] rules (`^id` alone or trailing).
fn collect_block_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_end();
        if let Some(id) = t.strip_prefix('^') {
            if !id.is_empty() && !id.chars().any(char::is_whitespace) {
                out.push(id.to_string());
                continue;
            }
        }
        if let Some(idx) = t.rfind(" ^") {
            let id = &t[idx + 2..];
            if !id.is_empty() && !id.chars().any(char::is_whitespace) {
                out.push(id.to_string());
            }
        }
    }
    out
}

/// Leading ATX `#`…`######` markers — dimmed while editing.
fn leading_hash_bytes(s: &str) -> usize {
    let n = s.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&n) {
        n
    } else {
        0
    }
}

fn layout_line_edit(ui: &Ui, text: &str, wrap_width: f32) -> Arc<egui::Galley> {
    let font = preview::edit_font(text);
    let dim = egui::Color32::from_rgba_unmultiplied(
        theme::TEXT.r(),
        theme::TEXT.g(),
        theme::TEXT.b(),
        96,
    );
    let mut job = LayoutJob::default();
    let prefix = leading_hash_bytes(text);
    if prefix > 0 {
        job.append(
            &text[..prefix],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: dim,
                ..Default::default()
            },
        );
        job.append(
            &text[prefix..],
            0.0,
            TextFormat {
                font_id: font,
                color: theme::TEXT,
                ..Default::default()
            },
        );
    } else {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font,
                color: theme::TEXT,
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = wrap_width;
    ui.fonts(|f| f.layout_job(job))
}

/// Horizontally center a readable-width column in the editor pane.
/// Height follows content (not the full viewport) so empty space below stays clickable.
fn centered_column(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let avail = ui.available_width();
    let width = avail.min(theme::CONTENT_MAX_WIDTH);
    let pad = ((avail - width) * 0.5).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(width);
            add_contents(ui);
        });
    });
}

/// Cheap height guess for off-screen rows (refined once the row is painted).
/// Prefer underestimates: overestimates create empty scroll travel that feels like low sensitivity.
fn estimate_row_height(line: &str, role: preview::FenceRole) -> f32 {
    if !matches!(role, preview::FenceRole::Outside) {
        return 22.0;
    }
    if line.is_empty() {
        return 22.0;
    }
    if line.contains("![[") || line.contains("![") {
        // Undershoot on purpose; measured height corrects when the row enters the viewport.
        return embed_width_hint(line)
            .map(|w| (w as f32 * 0.4).clamp(48.0, 160.0))
            .unwrap_or(64.0);
    }
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('#') {
        let level = 1 + rest.chars().take_while(|c| *c == '#').count();
        return match level {
            1 => 36.0,
            2 => 32.0,
            _ => 28.0,
        };
    }
    22.0
}

fn embed_width_hint(line: &str) -> Option<u32> {
    let start = line.find("![[")?;
    let body = &line[start + 3..];
    let end = body.find("]]")?;
    let inner = &body[..end];
    let size = inner.rsplit('|').next()?;
    if size.bytes().all(|b| b.is_ascii_digit()) {
        return size.parse().ok();
    }
    if let Some((w, _)) = size.split_once('x').or_else(|| size.split_once('X')) {
        if w.bytes().all(|b| b.is_ascii_digit()) {
            return w.parse().ok();
        }
    }
    None
}

/// Remove one leading indent level (`\\t`, 4 spaces, or 2 spaces).
fn unindent_line(s: &mut String, primary: usize, secondary: usize) -> (usize, usize) {
    let n = if s.starts_with('\t') {
        1
    } else if s.starts_with("    ") {
        4
    } else if s.starts_with("  ") {
        2
    } else {
        return (primary, secondary);
    };
    *s = s.chars().skip(n).collect();
    (primary.saturating_sub(n), secondary.saturating_sub(n))
}

/// Line and column for an absolute char caret. A caret past the last line
/// clamps to the end of that line.
fn caret_line_col(buffer: &str, caret: usize) -> (usize, usize) {
    let ranges = line_ranges(buffer);
    if ranges.is_empty() {
        return (0, caret);
    }
    let mut chars = 0usize;
    for (j, &(start, end)) in ranges.iter().enumerate() {
        let line_chars = buffer
            .get(start..end)
            .map(|t| t.chars().count())
            .unwrap_or(0);
        let line_end = chars + line_chars;
        let last = j + 1 == ranges.len();
        if caret <= line_end || last {
            return (j, caret.saturating_sub(chars).min(line_chars));
        }
        chars = line_end + 1;
    }
    (ranges.len() - 1, 0)
}

/// Absolute char offset of `col` on `line_i` (newlines between lines count as 1).
fn line_char_offset(buffer: &str, ranges: &[(usize, usize)], line_i: usize, col: usize) -> usize {
    let mut chars = 0usize;
    for (j, &(s, e)) in ranges.iter().enumerate() {
        let line_chars = buffer.get(s..e).map(|t| t.chars().count()).unwrap_or(0);
        if j == line_i {
            return chars + col.min(line_chars);
        }
        chars += line_chars + 1; // +1 for `\n`
    }
    chars
}

/// Index of the empty line under the last one. Appends it when the note
/// does not already end with an empty line. `true` means `buffer` grew.
fn line_below_last(buffer: &mut String) -> (usize, bool) {
    if buffer.is_empty() {
        return (0, false);
    }
    let ranges = line_ranges(buffer);
    let last = ranges.len() - 1;
    let (start, end) = ranges[last];
    let empty = buffer.get(start..end).unwrap_or("").is_empty();
    if empty {
        return (last, false);
    }
    buffer.push('\n');
    (last + 1, true)
}

/// Byte ranges of each line without the trailing `\n`.
fn line_ranges(s: &str) -> Vec<(usize, usize)> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, s.len()));
    out
}

/// Line whose content contains `byte`, or the line that ends at that newline.
fn line_index_at(ranges: &[(usize, usize)], byte: usize) -> usize {
    for (i, &(_, end)) in ranges.iter().enumerate() {
        if byte <= end {
            return i;
        }
    }
    ranges.len().saturating_sub(1)
}

pub(crate) fn shift_folds_in_text(
    folds: &mut Vec<usize>,
    buffer: &str,
    start: usize,
    end: usize,
    new_text: &str,
) {
    let ranges = line_ranges(buffer);
    shift_folds_for_replace(folds, &ranges, start, end, new_text, buffer);
}

fn shift_folds_for_replace(
    folds: &mut Vec<usize>,
    ranges: &[(usize, usize)],
    start: usize,
    end: usize,
    new_text: &str,
    buffer: &str,
) {
    let old = buffer.get(start..end).unwrap_or("");
    let at = line_index_at(ranges, start);
    fold::shift_lines(folds, at + 1, fold::line_delta(old, new_text));
}

/// Line index when an embed will be inserted *after* a non-empty line.
fn embed_insert_line(buffer: &str, active_line: Option<usize>) -> Option<usize> {
    let i = active_line?;
    let (start, end) = line_ranges(buffer).get(i).copied()?;
    (start != end).then_some(i)
}

fn fold_chevron(ui: &Ui, id: Id, center: egui::Pos2, collapsed: bool) -> bool {
    let rect = egui::Rect::from_center_size(center, egui::vec2(18.0, 18.0));
    let response = ui.interact(rect, id, egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon = if collapsed {
        egui_phosphor::regular::CARET_RIGHT
    } else {
        egui_phosphor::regular::CARET_DOWN
    };
    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::WEAK
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
        color,
    );
    response.clicked()
}

/// `Some(new buffer)` when the note text changed. Heading and indent folds
/// update `folds` either way.
fn apply_fold_action(
    buffer: &str,
    folds: &mut Vec<usize>,
    line: usize,
    action: &EditorAction,
) -> Option<String> {
    let lines: Vec<&str> = buffer.split('\n').collect();
    let mut in_fence = vec![false; lines.len()];
    let mut open = None;
    for (i, line_text) in lines.iter().enumerate() {
        let role = preview::advance_fence(line_text, &mut open);
        in_fence[i] = !matches!(role, preview::FenceRole::Outside);
    }
    let regions = fold::regions(&lines, &in_fence);
    fold::retain_valid(folds, &regions);
    let callout = preview::callout_at(&lines, line)
        .map(|(header, end)| (header, end, lines[header].to_string()));

    match action {
        EditorAction::FoldAll => {
            *folds = regions.iter().map(|region| region.start).collect();
            preview::set_all_callouts_collapsed(buffer, true)
        }
        EditorAction::UnfoldAll => {
            folds.clear();
            preview::set_all_callouts_collapsed(buffer, false)
        }
        EditorAction::FoldMore => {
            if let Some((header, end, text)) = callout {
                let has_body = end > header + 1;
                if has_body && !preview::callout_collapsed(&text) {
                    let next = preview::set_callout_collapsed(&text, true)?;
                    return replace_line(buffer, header, &next);
                }
            }
            fold::fold_more(&regions, folds, line);
            None
        }
        EditorAction::FoldLess => {
            if fold::fold_less(&regions, folds, line) {
                return None;
            }
            let (header, _, text) = callout?;
            if !preview::callout_collapsed(&text) {
                return None;
            }
            let next = preview::set_callout_collapsed(&text, false)?;
            replace_line(buffer, header, &next)
        }
        _ => None,
    }
}

fn replace_line(buffer: &str, line: usize, next: &str) -> Option<String> {
    let (start, end) = line_ranges(buffer).get(line).copied()?;
    let mut out = buffer.to_string();
    out.replace_range(start..end, next);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{
        caret_line_col, collect_block_ids, estimate_row_height, filter_frag_candidates,
        gap_after_row, insert_isolated_block, line_below_last, line_char_offset, line_ranges,
        note_scroll_boost, open_wiki_query, parse_wiki_suggest_query, row_at_local_y,
        row_prefix_heights, unindent_line, visible_row_range, wiki_candidates,
        wiki_suggest_insert_token, ViewMode, WikiSuggestQuery, NOTE_TRACKPAD_BOOST,
        NOTE_WHEEL_BOOST, ROW_GAP, TOP_PAD,
    };
    use crate::math_block;
    use crate::preview::FenceRole;

    #[test]
    fn view_mode_toggles() {
        assert_eq!(ViewMode::Live.toggle_reading(), ViewMode::Reading);
        assert_eq!(ViewMode::Reading.toggle_reading(), ViewMode::Live);
        assert_eq!(ViewMode::Source.toggle_reading(), ViewMode::Reading);

        assert_eq!(ViewMode::Live.toggle_source(), ViewMode::Source);
        assert_eq!(ViewMode::Source.toggle_source(), ViewMode::Live);
        assert_eq!(ViewMode::Reading.toggle_source(), ViewMode::Source);
    }

    #[test]
    fn note_scroll_boost_scales_discrete_notches() {
        assert_eq!(note_scroll_boost(15.0), NOTE_WHEEL_BOOST);
        assert_eq!(note_scroll_boost(2.0), NOTE_TRACKPAD_BOOST);
        assert!(NOTE_WHEEL_BOOST > NOTE_TRACKPAD_BOOST);
    }

    #[test]
    fn prefix_sums_bake_gaps_without_double_count() {
        // normal, fence-open, fence-body, fence-close, normal
        let roles = [
            FenceRole::Outside,
            FenceRole::Open,
            FenceRole::Body,
            FenceRole::Close,
            FenceRole::Outside,
        ];
        let heights = [22.0_f32, 22.0, 22.0, 22.0, 22.0];
        let prefix = row_prefix_heights(&heights, &roles, &[]);
        assert_eq!(prefix.len(), 6);
        assert_eq!(prefix[0], TOP_PAD);
        // After normal: height + ROW_GAP
        assert_eq!(prefix[1], TOP_PAD + 22.0 + ROW_GAP);
        // Fence rows: no gap between them
        assert_eq!(prefix[2], prefix[1] + 22.0);
        assert_eq!(prefix[3], prefix[2] + 22.0);
        assert_eq!(prefix[4], prefix[3] + 22.0);
        // Last normal: no trailing gap after final row
        assert_eq!(prefix[5], prefix[4] + 22.0);
        assert_eq!(gap_after_row(4, 5, FenceRole::Outside), 0.0);
        assert_eq!(gap_after_row(0, 5, FenceRole::Outside), ROW_GAP);
        assert_eq!(gap_after_row(1, 5, FenceRole::Open), 0.0);
    }

    #[test]
    fn click_y_selects_the_row_at_that_height() {
        let roles = [FenceRole::Outside, FenceRole::Outside];
        let heights = [22.0, 22.0];
        let prefix = row_prefix_heights(&heights, &roles, &[]);
        assert_eq!(row_at_local_y(&prefix, &heights, 0.0), Some(0));
        assert_eq!(row_at_local_y(&prefix, &heights, TOP_PAD + 10.0), Some(0));
        let gap_start = prefix[1] - ROW_GAP;
        assert_eq!(row_at_local_y(&prefix, &heights, gap_start + 1.0), Some(0));
        assert_eq!(row_at_local_y(&prefix, &heights, prefix[2] - 1.0), Some(1));
        assert_eq!(row_at_local_y(&prefix, &heights, prefix[2] + 4.0), None);
    }

    #[test]
    fn visible_range_includes_forced_active_line() {
        let roles = [FenceRole::Outside; 10];
        let heights = [20.0_f32; 10];
        let prefix = row_prefix_heights(&heights, &roles, &[]);
        // Viewport far below content — without `force` the range would be empty at end.
        let (first, last) = visible_row_range(&prefix, 10_000.0, 10_100.0, Some(1));
        assert!(first <= 1, "active row must not be clipped away");
        assert!(last > 1, "active row must be in [first, last)");
    }

    #[test]
    fn embed_estimate_undershoots() {
        let h = estimate_row_height("![[pic|425]]", FenceRole::Outside);
        assert!(h <= 160.0, "estimate {h} should undershoot measured embeds");
        assert!(h >= 48.0);
    }

    #[test]
    fn click_below_document_edits_the_line_under_the_last() {
        let mut note = "hello".to_string();
        assert_eq!(line_below_last(&mut note), (1, true));
        assert_eq!(note, "hello\n");
        assert_eq!(line_below_last(&mut note), (1, false));
        assert_eq!(note, "hello\n");

        let mut note = "hello\nworld".to_string();
        assert_eq!(line_below_last(&mut note), (2, true));
        assert_eq!(note, "hello\nworld\n");
    }

    #[test]
    fn ranges_basic() {
        assert_eq!(line_ranges("a\nb"), vec![(0, 1), (2, 3)]);
        assert_eq!(line_ranges("a\n"), vec![(0, 1), (2, 2)]);
        assert_eq!(line_ranges("abc"), vec![(0, 3)]);
    }

    #[test]
    fn math_block_caret_lands_on_the_blank_line() {
        let mut s = "hello".to_string();
        let caret =
            insert_isolated_block(&mut s, 5, 5, math_block::SNIPPET, "$$\n".chars().count());
        assert_eq!(s, "hello\n$$\n\n$$");
        assert_eq!(caret_line_col(&s, caret), (2, 0));
    }

    #[test]
    fn char_offset_accounts_for_newlines() {
        let s = "hi\nworld";
        let ranges = line_ranges(s);
        assert_eq!(line_char_offset(s, &ranges, 0, 0), 0);
        assert_eq!(line_char_offset(s, &ranges, 0, 2), 2);
        assert_eq!(line_char_offset(s, &ranges, 1, 0), 3); // after "hi\n"
        assert_eq!(line_char_offset(s, &ranges, 1, 5), 8);
    }

    #[test]
    fn unindent_strips_tab_or_spaces() {
        let mut s = "\thello".to_string();
        assert_eq!(unindent_line(&mut s, 3, 3), (2, 2));
        assert_eq!(s, "hello");

        let mut s = "    hello".to_string();
        assert_eq!(unindent_line(&mut s, 4, 4), (0, 0));
        assert_eq!(s, "hello");

        let mut s = "hello".to_string();
        assert_eq!(unindent_line(&mut s, 2, 2), (2, 2));
        assert_eq!(s, "hello");
    }

    #[test]
    fn wiki_query_detects_open_brackets() {
        let s = "see [[Hel";
        assert_eq!(open_wiki_query(s, s.len()), Some((4, "Hel".into())));
        assert!(open_wiki_query("see [[Hel]] x", 13).is_none());
    }

    #[test]
    fn wiki_candidates_filter() {
        let notes = vec![
            "inbox/Hello.md".into(),
            "Other.md".into(),
            "hello.md".into(),
        ];
        let found = wiki_candidates(&notes, "hel");
        assert!(found.iter().any(|p| p == "hello.md"));
        assert!(found.iter().any(|p| p == "inbox/Hello.md"));
        assert!(!found.iter().any(|p| p == "Other.md"));
    }

    #[test]
    fn wiki_suggest_query_parses_note_heading_block() {
        assert_eq!(
            parse_wiki_suggest_query("Hel"),
            WikiSuggestQuery::Notes {
                query: "Hel".into()
            }
        );
        assert_eq!(
            parse_wiki_suggest_query("Note#He"),
            WikiSuggestQuery::Headings {
                note: "Note".into(),
                prefix: "He".into()
            }
        );
        assert_eq!(
            parse_wiki_suggest_query("Note#"),
            WikiSuggestQuery::Headings {
                note: "Note".into(),
                prefix: String::new()
            }
        );
        assert_eq!(
            parse_wiki_suggest_query("Note#^ab"),
            WikiSuggestQuery::Blocks {
                note: "Note".into(),
                prefix: "ab".into()
            }
        );
        assert_eq!(
            parse_wiki_suggest_query("#^x"),
            WikiSuggestQuery::Blocks {
                note: String::new(),
                prefix: "x".into()
            }
        );
        assert_eq!(
            parse_wiki_suggest_query("Note^id"),
            WikiSuggestQuery::Blocks {
                note: "Note".into(),
                prefix: "id".into()
            }
        );
    }

    #[test]
    fn wiki_frag_candidates_filter_and_insert_token() {
        let heads = vec!["Hello".into(), "World".into(), "Help".into()];
        let found = filter_frag_candidates(&heads, "he");
        assert_eq!(found, vec!["Help".to_string(), "Hello".to_string()]);

        let blocks = collect_block_ids("para ^a\n^solo\n# H\nother ^b-2\n");
        assert_eq!(blocks, vec!["a".to_string(), "solo".to_string(), "b-2".to_string()]);
        let filtered = filter_frag_candidates(&blocks, "b");
        assert_eq!(filtered, vec!["b-2".to_string()]);

        let h = WikiSuggestQuery::Headings {
            note: "Note".into(),
            prefix: String::new(),
        };
        assert_eq!(wiki_suggest_insert_token(&h, "Hello"), "Note#Hello");
        let b = WikiSuggestQuery::Blocks {
            note: String::new(),
            prefix: String::new(),
        };
        assert_eq!(wiki_suggest_insert_token(&b, "id"), "#^id");
        let n = WikiSuggestQuery::Notes {
            query: "x".into(),
        };
        assert_eq!(wiki_suggest_insert_token(&n, "inbox/Hello.md"), "inbox/Hello");
    }

    #[test]
    fn replace_newline_shifts_the_fold_below_and_keeps_the_one_above() {
        let buffer = "# A\nkeep\n# B\n";
        let mut folds = vec![0, 2];
        let start = buffer.find("keep").unwrap();
        super::shift_folds_in_text(&mut folds, buffer, start, start + 4, "keep\nextra");
        assert_eq!(folds, vec![0, 3]);
    }

    #[test]
    fn fold_more_collapses_callout_before_the_heading() {
        let buffer = "# H\n> [!note] T\n> body\n";
        let mut folds = Vec::new();
        let next = super::apply_fold_action(
            buffer,
            &mut folds,
            2,
            &crate::editor_menu::EditorAction::FoldMore,
        );
        assert!(folds.is_empty());
        let next = next.expect("callout marker");
        assert!(next.contains("> [!note]- T"));
        assert!(next.starts_with("# H\n"));
    }
}
