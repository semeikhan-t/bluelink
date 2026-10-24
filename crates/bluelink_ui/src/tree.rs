use bluelink_core::NoteMeta;
use egui::{
    emath::TSTransform, Align, CursorIcon, Id, Key, LayerId, Layout, Order, Pos2, RichText, Sense,
    TextEdit, Ui, UiBuilder, Vec2,
};
use egui_phosphor::regular;

use crate::theme;

/// One floating tree popup — not per-row egui `context_menu` (that freezes via BarState).
#[derive(Clone)]
struct TreeMenu {
    pos: Pos2,
    /// `None` = empty panel (Undo only).
    path: Option<String>,
    is_note: bool,
}

const TREE_MENU_KEY: &str = "bluelink_tree_ctx_menu";

/// In-place rename of a note or folder in the sidebar.
pub struct InlineRename {
    pub from: String,
    pub draft: String,
    pub request_focus: bool,
}

#[derive(Default)]
pub struct TreeAction {
    /// Open note in the current tab (in-place).
    pub select: Option<String>,
    /// Open note in a new tab (Ctrl+click / context menu).
    pub open_in_new_tab: Option<String>,
    pub new_note: bool,
    pub new_folder: bool,
    /// Begin inline rename for this note/folder path.
    pub start_rename: Option<String>,
    /// Commit inline rename: `(from, to)` full rel paths.
    pub commit_rename: Option<(String, String)>,
    pub cancel_rename: bool,
    /// Note or folder path to delete.
    pub delete: Option<String>,
    /// Undo last delete.
    pub undo: bool,
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
    can_undo: bool,
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
            // add_sized() centers; keep every tree row flush left.
            ui.with_layout(Layout::top_down(Align::Min).with_cross_justify(true), |ui| {
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
                let dropped =
                    folder_block(ui, folder, list, selected, renaming, &mut action);
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

            // Empty space under the tree — still offers Undo via context menu.
            let fill = ui.allocate_response(ui.available_size(), Sense::click());
            open_tree_menu_on_right_click(&fill, None, false);
            }); // left-aligned layout
        });

    show_tree_menu(ui.ctx(), can_undo, &mut action);
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
            });
            open_tree_menu_on_right_click(&cr.header_response, Some(folder.to_string()), false);
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
        RichText::new(text)
            .size(16.0)
            .strong()
            .color(theme::ACCENT)
    } else {
        RichText::new(text).size(16.0).color(theme::TEXT)
    };
    let row_size = egui::vec2(ui.available_width(), 30.0);

    if ui.ctx().is_being_dragged(id) {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);

        // Follow pointer (same approach as egui::Ui::dnd_drag_source).
        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui
            .scope_builder(UiBuilder::new().layer_id(layer_id), |ui| {
                left_row(ui, row_size, |ui| {
                    ui.label(label);
                });
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
    // Don't use add_sized — it forces Layout::centered_and_justified.
    let painted = left_row(ui, row_size, |ui| {
        ui.add(egui::Label::new(label).sense(Sense::hover()));
    });
    let response = ui
        .interact(painted.rect, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Open · Ctrl+click for new tab");

    if response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
    }
    if response.clicked() {
        // Ctrl/Cmd+click → new tab; plain click → replace current tab.
        let new_tab = ui.input(|i| i.modifiers.command);
        if new_tab {
            action.open_in_new_tab = Some(note.rel_path.clone());
        } else {
            action.select = Some(note.rel_path.clone());
        }
    }
    open_tree_menu_on_right_click(&response, Some(note.rel_path.clone()), true);
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

fn open_tree_menu_on_right_click(response: &egui::Response, path: Option<String>, is_note: bool) {
    if !response.secondary_clicked() {
        return;
    }
    let Some(pos) = response.interact_pointer_pos() else {
        return;
    };
    response.ctx.data_mut(|d| {
        d.insert_temp(
            Id::new(TREE_MENU_KEY),
            Some(TreeMenu {
                pos,
                path,
                is_note,
            }),
        );
    });
}

fn show_tree_menu(ctx: &egui::Context, can_undo: bool, action: &mut TreeAction) {
    let Some(menu) = ctx.data_mut(|d| d.get_temp::<Option<TreeMenu>>(Id::new(TREE_MENU_KEY)).flatten())
    else {
        return;
    };

    let mut chosen = false;
    let popup = egui::Area::new(Id::new("bluelink_tree_ctx_popup"))
        .order(Order::Foreground)
        .fixed_pos(menu.pos)
        .sense(Sense::click())
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.add_enabled_ui(can_undo, |ui| {
                    if ui
                        .button(format!("{}  Undo", regular::ARROW_COUNTER_CLOCKWISE))
                        .clicked()
                    {
                        action.undo = true;
                        chosen = true;
                    }
                });
                if let Some(path) = menu.path.as_ref() {
                    if can_undo {
                        ui.separator();
                    }
                    if menu.is_note
                        && ui
                            .button(format!("{}  Open in new tab", regular::ARROW_SQUARE_OUT))
                            .clicked()
                    {
                        action.open_in_new_tab = Some(path.clone());
                        chosen = true;
                    }
                    if ui
                        .button(format!("{}  Rename", regular::PENCIL_SIMPLE))
                        .clicked()
                    {
                        action.start_rename = Some(path.clone());
                        chosen = true;
                    }
                    if ui
                        .button(format!("{}  Delete", regular::TRASH))
                        .clicked()
                    {
                        action.delete = Some(path.clone());
                        chosen = true;
                    }
                }
            });
        });

    let escape = ctx.input(|i| i.key_pressed(Key::Escape));
    let click_outside = ctx.input(|i| {
        i.pointer.primary_clicked()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !popup.response.rect.contains(p))
    });
    if chosen || escape || click_outside {
        ctx.data_mut(|d| d.insert_temp(Id::new(TREE_MENU_KEY), None::<TreeMenu>));
    } else {
        // Temp data is frame-local — re-insert while the popup stays open.
        ctx.data_mut(|d| d.insert_temp(Id::new(TREE_MENU_KEY), Some(menu)));
    }
}

fn left_row(ui: &mut Ui, size: Vec2, add: impl FnOnce(&mut Ui)) -> egui::Response {
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), add)
        .response
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}
