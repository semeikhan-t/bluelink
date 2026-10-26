use bluelink_core::NoteMeta;
use egui::{
    emath::TSTransform, Color32, CursorIcon, FontId, Id, Key, LayerId, Order, Pos2, RichText,
    Sense, TextEdit, Ui, UiBuilder, Vec2,
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
const ROW_H: f32 = 28.0;

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

    // Action row keeps a right pad so buttons don't sit under the scroll gutter.
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 0,
            right: 12,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.columns(2, |cols| {
                let mk = |label: String| {
                    egui::Button::new(RichText::new(label).size(13.0).color(theme::TEXT))
                        .min_size(Vec2::new(0.0, 30.0))
                        .corner_radius(theme::round_sm())
                        .fill(theme::FAINT)
                        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
                };
                if cols[0]
                    .add_sized(
                        [cols[0].available_width(), 30.0],
                        mk(format!("{}  Note", regular::PLUS)),
                    )
                    .on_hover_text("Create note")
                    .clicked()
                {
                    action.new_note = true;
                }
                if cols[1]
                    .add_sized(
                        [cols[1].available_width(), 30.0],
                        mk(format!("{}  Folder", regular::FOLDER_PLUS)),
                    )
                    .on_hover_text("Create folder")
                    .clicked()
                {
                    action.new_folder = true;
                }
            });
        });
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .id_salt("sidebar_tree_scroll")
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
            }

            let root_resp = ui.vertical(|ui| {
                if !root_notes.is_empty()
                    || egui::DragAndDrop::has_payload_of_type::<String>(ui.ctx())
                {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{}  root", regular::HOUSE))
                            .size(12.0)
                            .color(theme::WEAK),
                    );
                    ui.add_space(2.0);
                }
                for note in &root_notes {
                    note_row(ui, note, selected, renaming, &mut action);
                }
                if root_notes.is_empty() {
                    let _ = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 20.0),
                        Sense::hover(),
                    );
                }
            });
            if let Some(from) = root_resp.response.dnd_release_payload::<String>() {
                let file = basename(&from);
                let to = file.to_string();
                if *from != to {
                    action.move_note = Some(((*from).clone(), to));
                }
            }

            // Click-empty → context menu (Undo). Bound height so the panel cannot grow sideways.
            let fill_h = ui.available_height().clamp(8.0, 10_000.0);
            let fill_w = ui.available_width();
            let fill = ui.allocate_response(egui::vec2(fill_w, fill_h), Sense::click());
            open_tree_menu_on_right_click(&fill, None, false);
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
    let editing = renaming.as_ref().is_some_and(|r| r.from == folder);

    let resp = ui.vertical(|ui| {
        if editing {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(regular::FOLDER)
                        .size(14.0)
                        .color(theme::WEAK),
                );
                rename_field(ui, renaming, action, true);
            });
            if list.is_empty() {
                ui.label(
                    RichText::new("drop notes here")
                        .size(12.0)
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
                    .size(13.0)
                    .color(theme::TEXT),
            )
            .id_salt(("folder", folder))
            .default_open(true)
            .show(ui, |ui| {
                if list.is_empty() {
                    ui.label(
                        RichText::new("drop notes here")
                            .size(12.0)
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
            ui.set_min_height(ROW_H);
            ui.label(RichText::new(regular::NOTE).size(14.0).color(theme::WEAK));
            rename_field(ui, renaming, action, false);
        });
        return;
    }

    let id = Id::new(("note_dnd", note.rel_path.as_str()));
    let is_sel = selected == Some(note.rel_path.as_str());
    let name = if note.title.is_empty() {
        note.rel_path
            .rsplit('/')
            .next()
            .unwrap_or(&note.rel_path)
            .trim_end_matches(".md")
    } else {
        note.title.as_str()
    };
    let icon_color = if is_sel { theme::ACCENT } else { theme::WEAK };
    // Cap to clip so hover/paint never pushes SidePanel width.
    let row_w = ui.available_width().min(ui.clip_rect().width()).max(1.0);
    let row_size = egui::vec2(row_w, ROW_H);

    let paint_row = |ui: &mut Ui, rect: egui::Rect, hovered: bool| {
        let fill = if is_sel {
            theme::FAINT
        } else if hovered {
            Color32::from_rgb(0x1c, 0x1c, 0x1f)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, theme::round_sm(), fill);
        if is_sel {
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + 6.0),
                egui::pos2(rect.left() + 2.0, rect.bottom() - 6.0),
            );
            ui.painter()
                .rect_filled(bar, theme::round_sm(), theme::ACCENT);
        }

        let cy = rect.center().y;
        let icon_galley = ui.fonts(|f| {
            f.layout_no_wrap(
                regular::NOTE.to_owned(),
                FontId::proportional(14.0),
                icon_color,
            )
        });
        let name_galley = ui.fonts(|f| {
            f.layout_no_wrap(name.to_owned(), FontId::proportional(14.0), theme::TEXT)
        });
        let mut x = rect.left() + 8.0;
        ui.painter().galley(
            egui::pos2(x, cy - icon_galley.size().y * 0.5),
            icon_galley,
            icon_color,
        );
        x += 22.0;
        ui.painter().galley(
            egui::pos2(x, cy - name_galley.size().y * 0.5),
            name_galley,
            theme::TEXT,
        );
    };

    if ui.ctx().is_being_dragged(id) {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);

        let layer_id = LayerId::new(Order::Tooltip, id);
        let response = ui
            .scope_builder(UiBuilder::new().layer_id(layer_id), |ui| {
                let (rect, _) = ui.allocate_exact_size(row_size, Sense::hover());
                paint_row(ui, rect, false);
            })
            .response;
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let delta = pointer_pos - response.rect.center();
            ui.ctx()
                .transform_layer_shapes(layer_id, TSTransform::from_translation(delta));
        }
        return;
    }

    let (rect, response) = ui.allocate_exact_size(row_size, Sense::click_and_drag());
    let response = response
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Open · Ctrl+click for new tab");
    paint_row(ui, rect, response.hovered());

    if response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), note.rel_path.clone());
    }
    if response.clicked() {
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
    let Some(menu) =
        ctx.data_mut(|d| d.get_temp::<Option<TreeMenu>>(Id::new(TREE_MENU_KEY)).flatten())
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
                ui.set_min_width(160.0);
                if can_undo && ui.button(format!("{}  Undo", regular::ARROW_COUNTER_CLOCKWISE)).clicked()
                {
                    action.undo = true;
                    chosen = true;
                }
                if let Some(path) = &menu.path {
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
        ctx.data_mut(|d| d.insert_temp(Id::new(TREE_MENU_KEY), Some(menu)));
    }
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}
