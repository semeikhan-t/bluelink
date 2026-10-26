//! Note outline (ATX headings) for the right rail — jump and rename.

use egui::{RichText, Sense, TextEdit, Ui};
use egui_phosphor::regular;

use crate::theme;

#[derive(Clone, Debug)]
pub struct OutlineEntry {
    pub line: usize,
    pub level: u8,
    pub title: String,
}

/// Collect ATX headings (`#`…`######`) with 0-based line indexes.
pub fn headings(text: &str) -> Vec<OutlineEntry> {
    let mut out = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        let Some((level, title)) = atx_heading(raw) else {
            continue;
        };
        out.push(OutlineEntry {
            line,
            level,
            title: title.to_string(),
        });
    }
    out
}

fn atx_heading(line: &str) -> Option<(u8, &str)> {
    let t = line.trim();
    if !t.starts_with('#') {
        return None;
    }
    let after = t.trim_start_matches('#');
    let level = (t.len() - after.len()) as u8;
    if !(1..=6).contains(&level) {
        return None;
    }
    let title = after.trim();
    if title.is_empty() {
        None
    } else {
        Some((level, title))
    }
}

/// Inline rename of one outline row (`line` is the heading's buffer line).
pub struct OutlineRename {
    pub line: usize,
    pub old: String,
    pub draft: String,
    pub request_focus: bool,
}

#[derive(Default)]
pub struct OutlineAction {
    pub jump: Option<usize>,
    pub start_rename: Option<(usize, String)>,
    pub commit_rename: Option<(usize, String, String)>,
    pub cancel_rename: bool,
}

/// Outline list above backlinks. Click jumps; context / pencil starts rename.
pub fn show(
    ui: &mut Ui,
    entries: &[OutlineEntry],
    renaming: &mut Option<OutlineRename>,
) -> OutlineAction {
    let mut action = OutlineAction::default();
    if entries.is_empty() {
        return action;
    }

    ui.label(
        RichText::new("Outline")
            .size(16.0)
            .family(theme::display_family())
            .color(theme::TEXT),
    );
    ui.add_space(6.0);

    for entry in entries {
        let indent = ((entry.level.saturating_sub(1)) as f32) * 12.0;

        if renaming
            .as_ref()
            .is_some_and(|r| r.line == entry.line)
        {
            let Some(rename) = renaming.as_mut() else {
                continue;
            };
            ui.horizontal(|ui| {
                ui.add_space(indent);
                let resp = ui.add(
                    TextEdit::singleline(&mut rename.draft)
                        .desired_width(ui.available_width() - 8.0)
                        .font(egui::TextStyle::Small),
                );
                if rename.request_focus {
                    resp.request_focus();
                    rename.request_focus = false;
                }
                if resp.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        action.cancel_rename = true;
                    } else {
                        let new = rename.draft.trim().to_string();
                        if !new.is_empty() && new != rename.old {
                            action.commit_rename =
                                Some((rename.line, rename.old.clone(), new));
                        } else {
                            action.cancel_rename = true;
                        }
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && resp.has_focus() {
                    let new = rename.draft.trim().to_string();
                    if !new.is_empty() && new != rename.old {
                        action.commit_rename = Some((rename.line, rename.old.clone(), new));
                    } else {
                        action.cancel_rename = true;
                    }
                }
            });
            continue;
        }

        ui.horizontal(|ui| {
            ui.add_space(indent);
            let row = ui
                .add(
                    egui::Label::new(
                        RichText::new(&entry.title)
                            .size(13.0)
                            .color(theme::TEXT),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_text(format!("Line {}", entry.line + 1));
            if row.clicked() {
                action.jump = Some(entry.line);
            }
            if row.secondary_clicked() {
                action.start_rename = Some((entry.line, entry.title.clone()));
            }
            if ui
                .small_button(regular::PENCIL_SIMPLE.to_string())
                .on_hover_text("Rename heading")
                .clicked()
            {
                action.start_rename = Some((entry.line, entry.title.clone()));
            }
        });
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    action
}

/// Replace the ATX title on `line_idx` (0-based). Preserves `#` level and indent.
pub fn rewrite_heading_line(buffer: &mut String, line_idx: usize, new_title: &str) -> bool {
    let mut out = String::with_capacity(buffer.len());
    let mut found = false;
    for (i, chunk) in buffer.split_inclusive('\n').enumerate() {
        if i != line_idx {
            out.push_str(chunk);
            continue;
        }
        let nl = if chunk.ends_with('\n') { "\n" } else { "" };
        let core = chunk.trim_end_matches('\n').trim_end_matches('\r');
        let trimmed = core.trim_start();
        let indent_len = core.len() - trimmed.len();
        let indent = &core[..indent_len];
        let after_hashes = trimmed.trim_start_matches('#');
        let level = trimmed.len() - after_hashes.len();
        if !(1..=6).contains(&level) {
            out.push_str(chunk);
            continue;
        }
        out.push_str(indent);
        for _ in 0..level {
            out.push('#');
        }
        out.push(' ');
        out.push_str(new_title);
        out.push_str(nl);
        found = true;
    }
    if found {
        *buffer = out;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_levels_and_lines() {
        let text = "# A\n\n## B\ntext\n### C\n";
        let h = headings(text);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].line, 0);
        assert_eq!(h[0].level, 1);
        assert_eq!(h[0].title, "A");
        assert_eq!(h[1].line, 2);
        assert_eq!(h[1].level, 2);
        assert_eq!(h[2].line, 4);
        assert_eq!(h[2].level, 3);
    }

    #[test]
    fn rewrite_preserves_level() {
        let mut buf = String::from("intro\n## Old Title\nbody\n");
        assert!(rewrite_heading_line(&mut buf, 1, "New Title"));
        assert_eq!(buf, "intro\n## New Title\nbody\n");
    }
}
