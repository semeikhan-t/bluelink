//! Line-based live preview: active line is raw Markdown, others are rendered.
//! Ctrl+A opens full-document edit with everything selected.

use std::sync::Arc;

use bluelink_core::Vault;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{FontFamily, FontId, Id, Key, Order, TextEdit, Ui};

use crate::preview;
use crate::theme;

pub struct LiveResult {
    pub changed: bool,
    pub wikilink: Option<String>,
}

/// Show live editor. `active_line` is the raw-editing line index (0-based).
/// `doc_edit` — full-buffer multiline mode (Ctrl+A).
pub fn show_live(
    ui: &mut Ui,
    vault: &Vault,
    buffer: &mut String,
    active_line: &mut Option<usize>,
    doc_edit: &mut bool,
    readonly: bool,
) -> LiveResult {
    let mut result = LiveResult {
        changed: false,
        wikilink: None,
    };

    if readonly {
        *doc_edit = false;
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
        return show_doc_edit(ui, buffer, doc_edit, active_line);
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
    // Split active line at cursor: (start, end, left, right).
    let mut split_at_cursor: Option<(usize, usize, String, String)> = None;
    let mut merge_with_prev: Option<usize> = None;
    // After inserting a completed `[[link]]`, show it as preview (clickable).
    let mut leave_edit_after_wiki = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Viewport height at content start — used to fill empty space below lines
            // so clicks there hit a widget (ScrollArea empty chrome is not clickable).
            let viewport_h = ui.available_height();
            let content_top = ui.cursor().top();

            centered_column(ui, |ui| {
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing.y = 10.0;

                for (i, &(start, end)) in ranges.iter().enumerate() {
                    let line = buffer.get(start..end).unwrap_or("").to_string();
                    let is_active = !readonly && *active_line == Some(i);
                    let row_id = egui::Id::new(("live_line", i));

                    if is_active {
                        let mut draft = line.clone();
                        let font = preview::edit_font(&line);
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

                        // Place cursor after merge-up / wiki insert.
                        let cursor_key = egui::Id::new(("live_cursor", i));
                        if let Some(idx) = ui.ctx().data_mut(|d| d.remove_temp::<usize>(cursor_key)) {
                            let mut state = output.state.clone();
                            state
                                .cursor
                                .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
                            state.store(ui.ctx(), row_id);
                        }

                        let enter = ui.input(|inp| inp.key_pressed(Key::Enter));
                        let backspace = ui.input(|inp| inp.key_pressed(Key::Backspace));
                        let focused = response.has_focus() || response.lost_focus();

                        let char_idx = output
                            .cursor_range
                            .map(|r| r.primary.ccursor.index)
                            .unwrap_or_else(|| draft.chars().count());
                        let byte_idx = draft
                            .char_indices()
                            .nth(char_idx)
                            .map(|(b, _)| b)
                            .unwrap_or(draft.len());

                        // `[[query` autocomplete
                        let mut wiki_consumed_enter = false;
                        if focused {
                            if let Some((brack_at, query)) = open_wiki_query(&draft, byte_idx) {
                                let candidates = wiki_candidates(&vault.index().note_paths(), &query);
                                let mut picked: Option<String> = None;
                                if !candidates.is_empty() {
                                    egui::Area::new(Id::new(("wiki_ac", i)))
                                        .order(Order::Foreground)
                                        .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 4.0))
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
                                                    if ui
                                                        .selectable_label(false, label)
                                                        .clicked()
                                                    {
                                                        picked = Some(label.to_string());
                                                    }
                                                }
                                            });
                                        });
                                    if enter {
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
                    } else {
                        let click = ui
                            .allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.set_min_height(22.0);
                                    preview::render_line(ui, vault, &line)
                                },
                            )
                            .inner;

                        match click {
                            preview::LineClick::Wiki(raw) => {
                                result.wikilink = Some(raw);
                            }
                            preview::LineClick::Edit if !readonly => {
                                request_active = Some(i);
                            }
                            _ => {}
                        }
                    }
                }

            });

            if !readonly {
                let used = (ui.cursor().top() - content_top).max(0.0);
                // Full editor width (not only the centered column) + at least the viewport.
                let fill_h = (viewport_h - used).max(120.0);
                let resp = ui.allocate_response(
                    egui::vec2(ui.available_width(), fill_h),
                    egui::Sense::click(),
                );
                if resp.clicked() {
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

    // ↑/↓ move the active raw line (commit edits via splice above first).
    if !readonly && request_active.is_none() {
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

    result
}

fn show_doc_edit(
    ui: &mut Ui,
    buffer: &mut String,
    doc_edit: &mut bool,
    active_line: &mut Option<usize>,
) -> LiveResult {
    let mut result = LiveResult {
        changed: false,
        wikilink: None,
    };

    if ui.input(|i| i.key_pressed(Key::Escape)) {
        *doc_edit = false;
        *active_line = None;
        return result;
    }

    let id = egui::Id::new("live_doc_edit");
    let font = FontId::new(theme::EDITOR_SIZE, FontFamily::Monospace);

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
                    .show(ui);

                let select_all = ui.ctx().data_mut(|d| {
                    let v = d
                        .get_temp::<bool>(egui::Id::new("live_select_all"))
                        .unwrap_or(false);
                    if v {
                        d.insert_temp(egui::Id::new("live_select_all"), false);
                    }
                    v
                });

                if select_all {
                    let end = CCursor::new(buffer.chars().count());
                    let mut state = output.state;
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
                    state.store(ui.ctx(), id);
                    output.response.request_focus();
                }

                if output.response.changed() {
                    result.changed = true;
                }
            });
        });

    result
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
    use super::{line_ranges, open_wiki_query, wiki_candidates};

    #[test]
    fn ranges_basic() {
        assert_eq!(line_ranges("a\nb"), vec![(0, 1), (2, 3)]);
        assert_eq!(line_ranges("a\n"), vec![(0, 1), (2, 2)]);
        assert_eq!(line_ranges("abc"), vec![(0, 3)]);
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
