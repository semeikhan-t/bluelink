//! Line-based live preview: active line is raw Markdown, others are rendered.
//! Ctrl+A opens full-document edit with everything selected.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bluelink_core::{PreviewDoc, Vault};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{FontFamily, FontId, Id, Key, Order, TextEdit, Ui};

use crate::editor_menu::{self, EditorAction};
use crate::media;
use crate::preview;
use crate::theme;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PendingClipboard {
    #[default]
    None,
    Cut,
    Copy,
    Paste,
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
}

pub struct LiveResult {
    pub changed: bool,
    pub wikilink: Option<WikiClick>,
    /// Context-menu / command: host should open a file picker and attach.
    pub attach_file: bool,
    /// Status line hint from media ingest (errors / confirmation).
    pub status: Option<String>,
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
const ROW_GAP: f32 = 10.0;
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
fn row_prefix_heights(heights: &[f32], roles: &[preview::FenceRole]) -> Vec<f32> {
    let n = heights.len();
    debug_assert_eq!(n, roles.len());
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(TOP_PAD);
    for i in 0..n {
        let h = heights[i].max(1.0);
        prefix.push(prefix[i] + h + gap_after_row(i, n, roles[i]));
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
/// `doc_edit` — full-buffer multiline mode (Ctrl+A).
pub fn show_live(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    buffer: &mut String,
    active_line: &mut Option<usize>,
    doc_edit: &mut bool,
    readonly: bool,
) -> LiveResult {
    let mut result = LiveResult {
        changed: false,
        wikilink: None,
        attach_file: false,
        status: None,
    };

    if readonly {
        *doc_edit = false;
    }

    // Drops / clipboard images before TextEdit so paste does not also insert text.
    if !readonly {
        if let Some(note) = source_path {
            handle_dropped_files(ui, vault, note, buffer, *active_line, &mut result);
            handle_clipboard_image_paste(ui, vault, note, buffer, *active_line, &mut result);
        }
    }

    // Ctrl+A → select entire note in full edit mode.
    if !readonly
        && ui.input(|i| i.modifiers.command && i.key_pressed(Key::A))
    {
        *doc_edit = true;
        *active_line = None;
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("live_select_all"), true));
    }

    if *doc_edit {
        return show_doc_edit(ui, vault, source_path, buffer, doc_edit, active_line, result);
    }

    boost_note_scroll(ui);

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
    // Split active line at cursor: (start, end, left, right).
    let mut split_at_cursor: Option<(usize, usize, String, String)> = None;
    let mut merge_with_prev: Option<usize> = None;
    // After inserting a completed `[[link]]`, show it as preview (clickable).
    let mut leave_edit_after_wiki = false;
    let mut request_select_all = false;
    let mut pending_clipboard = PendingClipboard::None;
    // Active line + char selection (primary, secondary) for Shift+↑/↓.
    let mut active_sel: Option<(usize, usize, usize)> = None;

    // Precompute fence roles so viewport culling can skip lines without breaking ``` state.
    let fence_roles = {
        let mut open = None;
        let mut roles = Vec::with_capacity(ranges.len());
        for &(start, end) in &ranges {
            let line = buffer.get(start..end).unwrap_or("");
            roles.push(preview::advance_fence(line, &mut open));
        }
        roles
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            // Viewport height at content start — used to fill empty space below lines
            // so clicks there hit a widget (ScrollArea empty chrome is not clickable).
            let viewport_h = viewport.height();

            // Keep parsed previews and measured heights across frames. Scrolling should not
            // re-run pulldown-cmark for every visible line on every repaint.
            let layout_id = Id::new(("live_layout", vault.root(), source_path.unwrap_or("")));
            let mut layout = ui.ctx().data(|d| d.get_temp::<LiveLayoutCache>(layout_id));
            if layout
                .as_ref()
                .map_or(true, |cache| cache.lines.len() != ranges.len())
            {
                layout = Some(LiveLayoutCache::for_lines(buffer, &ranges, &fence_roles));
            }
            let mut layout = layout.expect("live layout cache initialized");

            let heights: Vec<f32> = layout.lines.iter().map(|l| l.height.max(1.0)).collect();
            let prefix = row_prefix_heights(&heights, &fence_roles);
            let content_top = ui.cursor().top();
            let pane = ui.available_rect_before_wrap();
            // Registered before the lines so text, links, and embeds stay on top.
            // Empty space at the same height (side margins and the tail of a short line)
            // still hits this widget and opens that row.
            let margin = if readonly {
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
                let force = if readonly {
                    None
                } else {
                    active_line.filter(|&i| i < n)
                };
                let (first, last) =
                    visible_row_range(&prefix, viewport.min.y, viewport.max.y, force);

                ui.add_space(prefix[first]);

                for i in first..last {
                    let (start, end) = ranges[i];
                    let fence_role = fence_roles[i];
                    let in_fence = !matches!(fence_role, preview::FenceRole::Outside);
                    let is_active = !readonly && *active_line == Some(i);

                    let line = buffer.get(start..end).unwrap_or("");

                    let row_id = egui::Id::new(("live_line", i));
                    let measured_h;

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
                        let tab_pressed = focused
                            && ui.input(|inp| {
                                inp.key_pressed(Key::Tab)
                                    && !inp.modifiers.command
                                    && !inp.modifiers.alt
                            });
                        if tab_pressed && splice.is_none() {
                            let unindent = ui.input(|inp| inp.modifiers.shift);
                            if unindent {
                                let (p, s) = unindent_line(&mut draft, primary, secondary);
                                primary = p;
                                secondary = s;
                            } else {
                                let lo = primary.min(secondary);
                                let hi = primary.max(secondary);
                                let b0 = editor_menu::char_to_byte(&draft, lo);
                                let b1 = editor_menu::char_to_byte(&draft, hi);
                                draft.replace_range(b0..b1, "\t");
                                primary = lo + 1;
                                secondary = primary;
                            }
                            splice = Some((start, end, draft.clone()));
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new(("live_cursor", i)), primary);
                            });
                        }

                        active_sel = Some((i, primary, secondary));
                        if response.secondary_clicked() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                editor_menu::open_at(
                                    ui.ctx(),
                                    pos,
                                    Some(i),
                                    primary,
                                    secondary,
                                );
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
                                let candidates =
                                    wiki_candidates(&vault.index().note_paths(), &query);
                                let mut picked: Option<String> = None;
                                if !candidates.is_empty() {
                                    egui::Area::new(Id::new(("wiki_ac", i)))
                                        .order(Order::Foreground)
                                        .fixed_pos(
                                            response.rect.left_bottom() + egui::vec2(0.0, 4.0),
                                        )
                                        .show(ui.ctx(), |ui| {
                                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                ui.set_min_width(220.0);
                                                ui.label(
                                                    egui::RichText::new("Link to note")
                                                        .size(12.0)
                                                        .color(theme::WEAK),
                                                );
                                                for path in candidates.iter().take(12) {
                                                    let label = path.trim_end_matches(".md");
                                                    let r = ui.selectable_label(false, label);
                                                    if r.hovered() || r.clicked() {
                                                        grace = 24;
                                                    }
                                                    if r.clicked() {
                                                        picked = Some(label.to_string());
                                                    }
                                                }
                                            });
                                        });
                                    if enter && focused {
                                        picked = Some(
                                            candidates[0].trim_end_matches(".md").to_string(),
                                        );
                                        wiki_consumed_enter = true;
                                    }
                                }
                                if let Some(name) = picked {
                                    let prefix = &draft[..brack_at];
                                    let suffix = &draft[byte_idx..];
                                    let inserted = format!("{prefix}[[{name}]]{suffix}");
                                    splice = Some((start, end, inserted));
                                    // Drop raw edit so the link renders as a clickable preview.
                                    leave_edit_after_wiki = true;
                                }
                            }
                        }
                        ui.ctx().data_mut(|d| d.insert_temp(grace_key, grace));

                        // singleline TextEdit loses focus on Enter — newline, unless wiki picked.
                        let enter_here =
                            enter && focused && !wiki_consumed_enter && splice.is_none();
                        if enter_here {
                            let left = draft[..byte_idx].to_string();
                            let right = draft[byte_idx..].to_string();
                            split_at_cursor = Some((start, end, left, right));
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
                            .map(|r| {
                                r.primary.ccursor.index == 0 && r.secondary.ccursor.index == 0
                            })
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
                                    preview,
                                )
                            },
                        );
                        let click = row.inner;
                        measured_h = row.response.rect.height().max(1.0);

                        if !readonly && row.response.secondary_clicked() {
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
                                result.wikilink = Some(WikiClick { raw, new_tab });
                            }
                            preview::LineClick::ResizeEmbed {
                                target,
                                width,
                                height,
                            } if !readonly => {
                                if media::rewrite_embed_size(
                                    buffer, Some(i), &target, width, height,
                                ) {
                                    result.changed = true;
                                }
                            }
                            preview::LineClick::Edit if !readonly => {
                                request_active = Some(i);
                            }
                            _ => {}
                        }
                    }

                    layout.lines[i].height = measured_h;
                    let gap = gap_after_row(i, n, fence_role);
                    if gap > 0.0 {
                        ui.add_space(gap);
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
                        if let Some(i) = row_at_local_y(&prefix, &heights, local_y) {
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

            if !readonly {
                let used = (ui.cursor().top() - content_top).max(0.0);
                // Full editor width (not only the centered column) + at least the viewport.
                let fill_h = (viewport_h - used).max(120.0);
                let resp = ui.allocate_response(
                    egui::vec2(ui.available_width(), fill_h),
                    egui::Sense::click(),
                );
                if resp.secondary_clicked() {
                    let last = ranges.len().saturating_sub(1);
                    let end_chars = ranges
                        .get(last)
                        .and_then(|&(s, e)| buffer.get(s..e))
                        .map(|t| t.chars().count())
                        .unwrap_or(0);
                    if let Some(pos) = resp.interact_pointer_pos() {
                        editor_menu::open_at(
                            ui.ctx(),
                            pos,
                            Some(last),
                            end_chars,
                            end_chars,
                        );
                    }
                    request_active = Some(last);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("live_focus", last)), true);
                        d.insert_temp(egui::Id::new(("live_cursor", last)), end_chars);
                    });
                } else if resp.clicked() {
                    let last = ranges.len().saturating_sub(1);
                    request_active = Some(last);
                    let end_chars = ranges
                        .get(last)
                        .and_then(|&(s, e)| buffer.get(s..e))
                        .map(|t| t.chars().count())
                        .unwrap_or(0);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("live_focus", last)), true);
                        d.insert_temp(egui::Id::new(("live_cursor", last)), end_chars);
                    });
                }
            }
        });

        // One floating menu for the whole editor (not per-line egui context_menu).
        if !readonly {
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
                                &mut result,
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
                                    splice = Some((s, e, draft));
                                    request_active = Some(i);
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(egui::Id::new(("live_focus", i)), true);
                                        d.insert_temp(
                                            egui::Id::new(("live_cursor", i)),
                                            caret,
                                        );
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

    // Shift+↑/↓ → full-doc edit with selection extended by one line.
    if !readonly && !request_select_all {
        let (shift_up, shift_down) = ui.input(|i| {
            let shift_only =
                i.modifiers.shift && !i.modifiers.command && !i.modifiers.alt;
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
            *doc_edit = true;
            *active_line = None;
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("live_select_range"), (anchor, moving));
            });
        }
    }

    // ↑/↓ move the active raw line (commit edits via splice above first).
    if !readonly && request_active.is_none() && !request_select_all {
        let (up, down) = ui.input(|i| {
            let no_chord = !i.modifiers.command && !i.modifiers.alt && !i.modifiers.shift;
            (
                no_chord && i.key_pressed(Key::ArrowUp),
                no_chord && i.key_pressed(Key::ArrowDown),
            )
        });
        if up || down {
            let last = ranges.len().saturating_sub(1);
            request_active = match (*active_line, up, down) {
                (Some(i), true, _) if i > 0 => Some(i - 1),
                (Some(i), _, true) if i < last => Some(i + 1),
                (None, _, true) => Some(0),
                (None, true, _) => Some(last),
                _ => None,
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

    if let Some((start, end, left, right)) = split_at_cursor {
        let i = active_line.unwrap_or(0);
        buffer.replace_range(start..end, &format!("{left}\n{right}"));
        *active_line = Some(i + 1);
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("live_focus", i + 1)), true));
        result.changed = true;
    } else if let Some((start, end, new_text)) = splice {
        buffer.replace_range(start..end, &new_text);
        result.changed = true;
    }

    if let Some(i) = merge_with_prev {
        let ranges = line_ranges(buffer);
        if i > 0 {
            if let Some(&(ps, pe)) = ranges.get(i - 1) {
                if pe < buffer.len() && buffer.as_bytes()[pe] == b'\n' {
                    let join_at = buffer[ps..pe].chars().count();
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
        *doc_edit = true;
        *active_line = None;
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("live_select_all"), true));
    }

    result
}

fn show_doc_edit(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    buffer: &mut String,
    doc_edit: &mut bool,
    active_line: &mut Option<usize>,
    mut result: LiveResult,
) -> LiveResult {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        *doc_edit = false;
        *active_line = None;
        return result;
    }

    let id = egui::Id::new("live_doc_edit");
    let font = FontId::new(theme::EDITOR_SIZE, FontFamily::Monospace);

    boost_note_scroll(ui);

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

                if response.secondary_clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        editor_menu::open_at(ui.ctx(), pos, None, primary, secondary);
                    }
                }

                if response.changed() {
                    result.changed = true;
                }
            });
        });

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
                    if !try_insert_clipboard_image(vault, note, buffer, None, &mut result) {
                        editor_menu::dispatch_clipboard(ui, &EditorAction::Paste);
                    }
                } else {
                    editor_menu::dispatch_clipboard(ui, &EditorAction::Paste);
                }
            }
            EditorAction::Cut | EditorAction::Copy => {
                editor_menu::dispatch_clipboard(ui, &action);
            }
            ref text_action => {
                if let Some(caret) = editor_menu::apply_text_action(
                    buffer,
                    menu.primary,
                    menu.secondary,
                    text_action,
                ) {
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id)
                        .unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
                    state.store(ui.ctx(), id);
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
    result: &mut LiveResult,
) {
    let dropped = ui.ctx().input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
    if dropped.is_empty() {
        return;
    }
    let mut embedded = 0usize;
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
                        media::insert_embed(buffer, active_line, &rel);
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
                media::insert_embed(buffer, active_line, &rel);
                embedded += 1;
                result.changed = true;
            }
            Err(e) => result.status = Some(format!("drop failed: {e}")),
        }
    }
    if embedded > 0 && result.status.is_none() {
        result.status = Some(if embedded == 1 {
            "attached file".into()
        } else {
            format!("attached {embedded} files")
        });
    }
}

fn handle_clipboard_image_paste(
    ui: &Ui,
    vault: &Vault,
    active_note: &str,
    buffer: &mut String,
    active_line: Option<usize>,
    result: &mut LiveResult,
) {
    let wants_paste = ui.input(|i| {
        let key = i.modifiers.command && i.key_pressed(Key::V) && !i.modifiers.shift;
        let event = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
        key || event
    });
    if !wants_paste {
        return;
    }
    if !try_insert_clipboard_image(vault, active_note, buffer, active_line, result) {
        return;
    }
    // Swallow text paste so TextEdit does not also insert clipboard text.
    ui.ctx().input_mut(|i| {
        i.events
            .retain(|e| !matches!(e, egui::Event::Paste(_)));
        let _ = i.consume_key(egui::Modifiers::COMMAND, Key::V);
    });
}

fn try_insert_clipboard_image(
    vault: &Vault,
    active_note: &str,
    buffer: &mut String,
    active_line: Option<usize>,
    result: &mut LiveResult,
) -> bool {
    let Some(png) = media::clipboard_image_png() else {
        return false;
    };
    let stem = media::pasted_image_stem();
    match media::save_bytes(vault, &stem, "png", &png, active_note) {
        Ok(rel) => {
            media::insert_embed(buffer, active_line, &rel);
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
    (
        primary.saturating_sub(n),
        secondary.saturating_sub(n),
    )
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

#[cfg(test)]
mod tests {
    use super::{
        estimate_row_height, gap_after_row, line_char_offset, line_ranges, note_scroll_boost,
        open_wiki_query, row_at_local_y, row_prefix_heights, unindent_line, visible_row_range,
        wiki_candidates,
        NOTE_TRACKPAD_BOOST, NOTE_WHEEL_BOOST, ROW_GAP, TOP_PAD,
    };
    use crate::preview::FenceRole;

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
        let prefix = row_prefix_heights(&heights, &roles);
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
        let prefix = row_prefix_heights(&heights, &roles);
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
        let prefix = row_prefix_heights(&heights, &roles);
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
    fn ranges_basic() {
        assert_eq!(line_ranges("a\nb"), vec![(0, 1), (2, 3)]);
        assert_eq!(line_ranges("a\n"), vec![(0, 1), (2, 2)]);
        assert_eq!(line_ranges("abc"), vec![(0, 3)]);
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
        assert_eq!(
            open_wiki_query(s, s.len()),
            Some((4, "Hel".into()))
        );
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
}
