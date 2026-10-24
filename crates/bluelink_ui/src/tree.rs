use bluelink_core::NoteMeta;
use egui::{
    emath::TSTransform, CursorIcon, Id, Key, LayerId, Order, RichText, Sense, TextEdit, Ui,
    UiBuilder, Vec2,
};
use egui_phosphor::regular;

use crate::theme;

/// In-place rename of a note or folder in the sidebar.
pub struct InlineRename {
    pub from: String,
    pub draft: String,
    pub request_focus: bool,
}

#[derive(Default)]
pub struct TreeAction {
    pub select: Option<String>,
    pub new_note: bool,
    pub new_folder: bool,
    /// Begin inline rename for this note/folder path.
    pub start_rename: Option<String>,
    /// Commit inline rename: `(from, to)` full rel paths.
    pub commit_rename: Option<(String, String)>,
    pub cancel_rename: bool,
    /// Note or folder path to delete.
    pub delete: Option<String>,
    /// Move note `from` → `to` (full rel paths).
    pub move_note: Option<(String, String)>,
}

/// Left-side note list grouped by folder. `folders` includes empty dirs from disk.
pub fn show_tree(
    ui: &mut Ui,
    notes: &[NoteMeta],
    folders: &[String],
    selected: Option<&str>,
    renaming: &mut Option<InlineRename>,
) -> TreeAction {
    let mut action = TreeAction::default();

    ui.horizontal(|ui| {
        let btn =
            |label: String| egui::Button::new(RichText::new(label).size(15.0)).min_size(Vec2::new(0.0, 34.0));
        if ui
            .add(btn(format!("{} Note", regular::PLUS)))
            .on_hover_text("Create note")
            .clicked()
        {
            action.new_note = true;
        }
        if ui
            .add(btn(format!("{} Folder", regular::FOLDER_PLUS)))
            .on_hover_text("Create folder")
            .clicked()
        {
            action.new_folder = true;
        }
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut by_folder: Vec<(String, Vec<&NoteMeta>)> = Vec::new();
            let mut root_notes: Vec<&NoteMeta> = Vec::new();

            for note in notes {
                if let Some((folder, _)) = note.rel_path.rsplit_once('/') {
                    if let Some((_, list)) = by_folder.iter_mut().find(|(f, _)| f == folder) {
                        list.push(note);
                    } else {
                        by_folder.push((folder.to_string(), vec![note]));
                    }
                } else {
                    root_notes.push(note);
                }
            }

            for folder in folders {
                if !by_folder.iter().any(|(f, _)| f == folder) {
                    by_folder.push((folder.clone(), Vec::new()));
                }
            }
            by_folder.sort_by(|a, b| a.0.cmp(&b.0));

            for (folder, list) in &by_folder {
                let dropped = folder_block(ui, folder, list, selected, renaming, &mut action);
                if let Some(from) = dropped {
                    let file = basename(&from);
                    let to = format!("{folder}/{file}");
                    if *from != to {
                        action.move_note = Some(((*from).clone(), to));
                    }
                }
                ui.add_space(4.0);
            }

            // Root: notes + drop target to move out of a folder.
            let root_resp = ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                if !root_notes.is_empty()
                    || egui::DragAndDrop::has_payload_of_type::<String>(ui.ctx())
                {
                    ui.label(
                        RichText::new(format!("{} root", regular::HOUSE))
                            .size(17.0)
                            .strong()
                            .color(theme::WEAK),
                    );
                    ui.add_space(4.0);
                }
                for note in &root_notes {
                    note_row(ui, note, selected, renaming, &mut action);
                }
                // Keep a hit area when root is empty so drops still work.
                if root_notes.is_empty() {
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::hover());
                }
            });
            if let Some(from) = root_resp.response.dnd_release_payload::<String>() {
                let file = basename(&from);
                let to = file.to_string();
                if *from != to {
                    action.move_note = Some(((*from).clone(), to));
                }
            }
        });

    action
}

fn folder_block(
    ui: &mut Ui,
    folder: &str,
    list: &[&NoteMeta],
    selected: Option<&str>,
    renaming: &mut Option<InlineRename>,
    action: &mut TreeAction,
) -> Option<std::sync::Arc<String>> {
    let editing = renaming
        .as_ref()
        .is_some_and(|r| r.from == folder);

    let resp = ui.vertical(|ui| {
        ui.set_min_width(ui.available_width());

        if editing {
            ui.horizontal(|ui| {
                ui.label(RichText::new(regular::FOLDER).size(18.0).strong());
                rename_field(ui, renaming, action, true);
            });
            ui.add_space(4.0);
            if list.is_empty() {
                ui.label(
                    RichText::new("drop notes here")
                        .size(14.0)
                        .italics()
                        .color(theme::WEAK),
                );
            }
            for note in list {
                note_row(ui, note, selected, renaming, action);
            }
        } else {
            let cr = egui::CollapsingHeader::new(
                RichText::new(format!("{}  {folder}", regular::FOLDER))
                    .size(18.0)
                    .strong(),
            )
            .id_salt(("folder", folder))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                if list.is_empty() {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 28.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new("drop notes here")
                                    .size(14.0)
                                    .italics()
                                    .color(theme::WEAK),
                            );
                        },
                    );
                }
                for note in list {
                    note_row(ui, note, selected, renaming, action);
                }
            });
            path_context_menu(&cr.header_response, folder, action);
        }
    });
    resp.response.dnd_release_payload::<String>()
}

fn note_row(
    ui: &mut Ui,
    note: &NoteMeta,
    selected: Option<&str>,
    renaming: &mut Option<InlineRename>,
    action: &mut TreeAction,
) {
    let editing = renaming
        .as_ref()
        .is_some_and(|r| r.from == note.rel_path);

    if editing {
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            ui.label(RichText::new(regular::NOTE).size(16.0));
            rename_field(ui, renaming, action, false);
        });
        return;
    }

    let id = Id::new(("note_dnd", note.rel_path.as_str()));
    let is_sel = selected == Some(note.rel_path.as_str());
    let name = note
        .rel_path
        .rsplit('/')
        .next()
        .unwrap_or(&note.rel_path)
        .trim_end_matches(".md");
    let text = format!("{}  {name}", regular::NOTE);
    let label = if is_sel {
        RichText::new(text).size(16.0).strong().color(theme::ACCENT)
    } else {
        RichText::new(text).size(16.0)
    };
    let row_size = [ui.available_width(), 30.0];

    if ui.ctx().is_being_dragged(id) {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);

        // Follow pointer (same approach as egui::Ui::dnd_drag_source).
        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui
            .scope_builder(UiBuilder::new().layer_id(layer_id), |ui| {
                ui.add_sized(row_size, egui::Label::new(label));
            })
            .response;
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let delta = pointer_pos - response.rect.center();
            ui.ctx()
                .transform_layer_shapes(layer_id, TSTransform::from_translation(delta));
        }
        return;
    }

    // click_and_drag: click opens; drag starts only after movement threshold.
    let painted = ui.add_sized(row_size, egui::Label::new(label).sense(Sense::hover()));
    let response = ui
        .interact(painted.rect, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::Grab);

    if response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
    }
    if response.clicked() {
        action.select = Some(note.rel_path.clone());
    }
    path_context_menu(&response, &note.rel_path, action);
}

fn rename_field(
    ui: &mut Ui,
    renaming: &mut Option<InlineRename>,
    action: &mut TreeAction,
    is_folder: bool,
) {
    let Some(state) = renaming.as_mut() else {
        return;
    };
    let resp = ui.add(
        TextEdit::singleline(&mut state.draft)
            .desired_width(ui.available_width().max(60.0))
            .font(egui::TextStyle::Body)
            .frame(true),
    );
    if state.request_focus {
        resp.request_focus();
        state.request_focus = false;
    }

    if resp.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            action.cancel_rename = true;
            return;
        }
        let state = renaming.take().expect("renaming");
        let to = renamed_target(&state.from, &state.draft, is_folder);
        if !state.draft.trim().is_empty() && to != state.from {
            action.commit_rename = Some((state.from, to));
        }
    }
}

fn renamed_target(from: &str, draft: &str, is_folder: bool) -> String {
    let trimmed = draft.trim().trim_matches('/');
    let name = if is_folder {
        trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
    } else {
        let stem = trimmed
            .rsplit('/')
            .next()
            .unwrap_or(trimmed)
            .trim_end_matches(".md");
        format!("{stem}.md")
    };
    match from.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name,
    }
}

pub fn rename_draft(path: &str, is_folder: bool) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    if is_folder {
        base.to_string()
    } else {
        base.trim_end_matches(".md").to_string()
    }
}

fn path_context_menu(response: &egui::Response, path: &str, action: &mut TreeAction) {
    response.context_menu(|ui| {
        if ui
            .button(format!("{}  Rename", regular::PENCIL_SIMPLE))
            .clicked()
        {
            action.start_rename = Some(path.to_string());
            ui.close_menu();
        }
        if ui
            .button(format!("{}  Delete", regular::TRASH))
            .clicked()
        {
            action.delete = Some(path.to_string());
            ui.close_menu();
        }
    });
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}
