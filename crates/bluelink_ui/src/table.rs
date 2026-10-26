//! Visual GFM table. The note still stores Markdown; the live editor never
//! shows those lines as raw text.

use egui::{Pos2, Rect, Ui, Vec2};
use egui_phosphor::regular;

pub const SNIPPET: &str = "|  |  |\n| --- | --- |\n|  |  |";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
    pub aligns: Vec<Align>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    None,
    /// First line of a table. `end` is exclusive.
    Head { end: usize },
    Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    AddRowAbove,
    AddRowBelow,
    MoveRowUp,
    DuplicateRow,
    DeleteRow,
    AddColLeft,
    AddColRight,
    MoveColLeft,
    Align(Align),
    DuplicateCol,
    DeleteCol,
    SortAsc,
    SortDesc,
    SetCell(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubMenu {
    Row,
    Column,
}

#[derive(Clone)]
struct MenuState {
    pos: Pos2,
    start: usize,
    row: usize,
    col: usize,
    cols: usize,
    sub: Option<SubMenu>,
}

const MENU_KEY: &str = "bluelink_table_menu";

pub fn places(lines: &[String]) -> Vec<Place> {
    let mut out = vec![Place::None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if let Some(end) = table_end(lines, i) {
            out[i] = Place::Head { end };
            for slot in out.iter_mut().take(end).skip(i + 1) {
                *slot = Place::Tail;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn table_end(lines: &[String], i: usize) -> Option<usize> {
    let header = parse_row(&lines[i])?;
    let aligns = lines.get(i + 1).and_then(|l| parse_sep(l))?;
    if header.is_empty() || header.len() != aligns.len() {
        return None;
    }
    let mut j = i + 2;
    while j < lines.len() {
        if lines[j].trim().is_empty() {
            break;
        }
        if parse_row(&lines[j]).is_none() {
            break;
        }
        j += 1;
    }
    Some(j)
}

pub fn parse_slice(lines: &[String]) -> Option<Table> {
    if lines.len() < 2 {
        return None;
    }
    let aligns = parse_sep(&lines[1])?;
    let n = aligns.len();
    if n == 0 {
        return None;
    }
    let mut rows = Vec::new();
    rows.push(pad(parse_row(&lines[0])?, n));
    for line in lines.iter().skip(2) {
        rows.push(pad(parse_row(line)?, n));
    }
    Some(Table { rows, aligns })
}

fn pad(mut cells: Vec<String>, n: usize) -> Vec<String> {
    cells.truncate(n);
    while cells.len() < n {
        cells.push(String::new());
    }
    cells
}

fn parse_row(line: &str) -> Option<Vec<String>> {
    let t = line.trim();
    if !t.contains('|') {
        return None;
    }
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    Some(t.split('|').map(|c| c.trim().to_string()).collect())
}

fn parse_sep(line: &str) -> Option<Vec<Align>> {
    let cells = parse_row(line)?;
    if cells.is_empty() {
        return None;
    }
    cells.iter().map(|c| sep_align(c)).collect()
}

fn sep_align(cell: &str) -> Option<Align> {
    let s = cell.trim();
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut i = 0;
    let left = b[0] == b':';
    if left {
        i += 1;
    }
    let dashes = i;
    while i < b.len() && b[i] == b'-' {
        i += 1;
    }
    if i == dashes {
        return None;
    }
    let right = i < b.len() && b[i] == b':';
    if right {
        i += 1;
    }
    if i != b.len() {
        return None;
    }
    Some(match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

pub fn to_markdown(table: &Table) -> String {
    let mut lines = Vec::with_capacity(table.rows.len() + 1);
    if let Some(header) = table.rows.first() {
        lines.push(row_line(header));
    }
    lines.push(sep_line(&table.aligns));
    for row in table.rows.iter().skip(1) {
        lines.push(row_line(row));
    }
    lines.join("\n")
}

fn row_line(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

fn sep_line(aligns: &[Align]) -> String {
    let parts: Vec<&str> = aligns
        .iter()
        .map(|a| match a {
            Align::Left => ":---",
            Align::Center => ":---:",
            Align::Right => "---:",
        })
        .collect();
    format!("| {} |", parts.join(" | "))
}

/// `false` means the table should be removed from the note.
pub fn mutate(table: &mut Table, row: usize, col: usize, action: Action) -> bool {
    let cols = table.aligns.len();
    if cols == 0 || table.rows.is_empty() {
        return false;
    }
    let row = row.min(table.rows.len() - 1);
    let col = col.min(cols - 1);
    match action {
        Action::AddRowAbove => {
            table.rows.insert(row, empty_row(cols));
        }
        Action::AddRowBelow => {
            table.rows.insert(row + 1, empty_row(cols));
        }
        Action::MoveRowUp => {
            if row > 0 {
                table.rows.swap(row - 1, row);
            }
        }
        Action::DuplicateRow => {
            let copy = table.rows[row].clone();
            table.rows.insert(row + 1, copy);
        }
        Action::DeleteRow => {
            if table.rows.len() == 1 {
                return false;
            }
            table.rows.remove(row);
        }
        Action::AddColLeft => insert_col(table, col),
        Action::AddColRight => insert_col(table, col + 1),
        Action::MoveColLeft => {
            if col > 0 {
                swap_col(table, col - 1, col);
            }
        }
        Action::Align(align) => table.aligns[col] = align,
        Action::DuplicateCol => {
            for row in &mut table.rows {
                let value = row.get(col).cloned().unwrap_or_default();
                let at = (col + 1).min(row.len());
                row.insert(at, value);
            }
            table.aligns.insert(col + 1, table.aligns[col]);
        }
        Action::DeleteCol => {
            if cols <= 1 {
                return true;
            }
            for row in &mut table.rows {
                if col < row.len() {
                    row.remove(col);
                }
            }
            table.aligns.remove(col);
        }
        Action::SortAsc => sort_body(table, col, true),
        Action::SortDesc => sort_body(table, col, false),
        Action::SetCell(text) => {
            let clean = sanitize_cell(&text);
            if let Some(cell) = table.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
                *cell = clean;
            }
        }
    }
    true
}

fn empty_row(cols: usize) -> Vec<String> {
    vec![String::new(); cols]
}

fn insert_col(table: &mut Table, at: usize) {
    let at = at.min(table.aligns.len());
    for row in &mut table.rows {
        let at = at.min(row.len());
        row.insert(at, String::new());
    }
    table.aligns.insert(at, Align::Left);
}

fn swap_col(table: &mut Table, a: usize, b: usize) {
    table.aligns.swap(a, b);
    for row in &mut table.rows {
        if a < row.len() && b < row.len() {
            row.swap(a, b);
        }
    }
}

fn sort_body(table: &mut Table, col: usize, asc: bool) {
    if table.rows.len() < 2 {
        return;
    }
    table.rows[1..].sort_by(|a, b| {
        let ka = a.get(col).map(|s| s.to_lowercase()).unwrap_or_default();
        let kb = b.get(col).map(|s| s.to_lowercase()).unwrap_or_default();
        if asc { ka.cmp(&kb) } else { kb.cmp(&ka) }
    });
}

fn sanitize_cell(text: &str) -> String {
    text.replace(['|', '\n'], " ")
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MENU_KEY), None::<MenuState>));
}

const CELL_H_PAD: f32 = 16.0;
const CARET_SLACK: f32 = 8.0;
const CELL_FOCUS_KEY: &str = "bluelink_table_cell_focus";
/// Hover band just outside the grid. The plus is drawn only while the pointer is in it.
const ADD_GUTTER: f32 = 24.0;
const ADD_BTN: f32 = 22.0;

pub struct TableEdit {
    pub cell: Option<(usize, usize, String)>,
    pub add_row: bool,
    pub add_col: bool,
}

/// Apply a cell edit, then a trailing row or column from the hover plus.
pub fn apply_edit(table: &mut Table, edit: TableEdit) -> bool {
    if let Some((row, col, text)) = edit.cell {
        if !mutate(table, row, col, Action::SetCell(text)) {
            return false;
        }
    }
    if edit.add_row {
        let row = table.rows.len().saturating_sub(1);
        if !mutate(table, row, 0, Action::AddRowBelow) {
            return false;
        }
    }
    if edit.add_col {
        let col = table.aligns.len().saturating_sub(1);
        if !mutate(table, 0, col, Action::AddColRight) {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// Next cell for an arrow that is leaving the current one. No wrap at the edges.
fn neighbor(rows: usize, cols: usize, row: usize, col: usize, arrow: Arrow) -> Option<(usize, usize)> {
    if rows == 0 || cols == 0 || row >= rows || col >= cols {
        return None;
    }
    match arrow {
        Arrow::Left if col > 0 => Some((row, col - 1)),
        Arrow::Right if col + 1 < cols => Some((row, col + 1)),
        Arrow::Up if row > 0 => Some((row - 1, col)),
        Arrow::Down if row + 1 < rows => Some((row + 1, col)),
        _ => None,
    }
}

fn cell_id(start: usize, row: usize, col: usize) -> egui::Id {
    egui::Id::new(("tbl_cell", start, row, col))
}

pub fn set_cell_focused(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(CELL_FOCUS_KEY), on));
}

pub fn cell_focused(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(CELL_FOCUS_KEY)).unwrap_or(false))
}

struct Jump {
    row: usize,
    col: usize,
    at_end: bool,
}

/// Leave the focused cell when an arrow is at the text edge (left/right) or any vertical arrow.
fn arrow_jump(
    ui: &Ui,
    id: egui::Id,
    text: &str,
    rows: usize,
    cols: usize,
    row: usize,
    col: usize,
) -> Option<Jump> {
    if !ui.memory(|m| m.has_focus(id)) {
        return None;
    }
    let plain = ui.input(|i| !i.modifiers.command && !i.modifiers.alt && !i.modifiers.shift);
    if !plain {
        return None;
    }
    let arrow = ui.input(|i| {
        if i.key_pressed(egui::Key::ArrowLeft) {
            Some(Arrow::Left)
        } else if i.key_pressed(egui::Key::ArrowRight) {
            Some(Arrow::Right)
        } else if i.key_pressed(egui::Key::ArrowUp) {
            Some(Arrow::Up)
        } else if i.key_pressed(egui::Key::ArrowDown) {
            Some(Arrow::Down)
        } else {
            None
        }
    })?;
    let (primary, secondary) = caret_indices(ui.ctx(), id);
    let len = text.chars().count();
    let collapsed = primary == secondary;
    let leave = match arrow {
        Arrow::Left => collapsed && primary == 0,
        Arrow::Right => collapsed && primary >= len,
        Arrow::Up | Arrow::Down => true,
    };
    if !leave {
        return None;
    }
    let (nr, nc) = neighbor(rows, cols, row, col, arrow)?;
    let key = match arrow {
        Arrow::Left => egui::Key::ArrowLeft,
        Arrow::Right => egui::Key::ArrowRight,
        Arrow::Up => egui::Key::ArrowUp,
        Arrow::Down => egui::Key::ArrowDown,
    };
    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
    Some(Jump {
        row: nr,
        col: nc,
        at_end: arrow == Arrow::Left,
    })
}

fn caret_indices(ctx: &egui::Context, id: egui::Id) -> (usize, usize) {
    let Some(state) = egui::text_edit::TextEditState::load(ctx, id) else {
        return (0, 0);
    };
    let Some(range) = state.cursor.char_range() else {
        return (0, 0);
    };
    (range.primary.index, range.secondary.index)
}

fn place_cursor(ctx: &egui::Context, id: egui::Id, index: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
        egui::text::CCursor::new(index),
    )));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Equal shares of `avail` until a column's text is wider, then that column grows.
fn column_widths(avail: f32, text_px: &[f32]) -> Vec<f32> {
    let n = text_px.len().max(1) as f32;
    let share = if avail.is_finite() {
        (avail / n).max(0.0)
    } else {
        0.0
    };
    text_px
        .iter()
        .map(|w| (w + CELL_H_PAD + CARET_SLACK).max(share))
        .collect()
}

fn widest_text(ui: &Ui, table: &Table, col: usize) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    table
        .rows
        .iter()
        .map(|row| {
            let text = row.get(col).map(String::as_str).unwrap_or("");
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::TRANSPARENT)
                    .size()
                    .x
            })
        })
        .fold(0.0_f32, f32::max)
}

/// Cell edit, if the user typed. Right-click opens the menu.
/// Hover just below or to the right of the grid reveals a plus.
pub fn show(
    ui: &mut Ui,
    start: usize,
    table: &Table,
    focus_first: bool,
    editable: bool,
) -> Option<TableEdit> {
    let mut cell_edit = None;
    let mut focus_used = focus_first;
    let mut jump: Option<Jump> = None;
    let mut any_focused = false;
    let gutter = if editable { ADD_GUTTER } else { 0.0 };
    let avail = (ui.available_width() - gutter).max(48.0);
    let text_px: Vec<f32> = (0..table.aligns.len())
        .map(|c| widest_text(ui, table, c))
        .collect();
    let widths = column_widths(avail, &text_px);
    let mut add_row = false;
    let mut add_col = false;
    let mut table_width = avail;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let frame = egui::Frame::NONE
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::STROKE))
            .corner_radius(crate::theme::round())
            .show(ui, |ui| {
            egui::Grid::new(("md_table", start))
                .num_columns(table.aligns.len())
                .spacing(Vec2::ZERO)
                .show(ui, |ui| {
                    for (r, row) in table.rows.iter().enumerate() {
                        for (c, cell) in row.iter().enumerate() {
                            let align = match table.aligns.get(c).copied().unwrap_or(Align::Left) {
                                Align::Left => egui::Align::LEFT,
                                Align::Center => egui::Align::Center,
                                Align::Right => egui::Align::RIGHT,
                            };
                            let mut text = cell.clone();
                            let fill = if r == 0 {
                                crate::theme::FAINT
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let inner = (widths.get(c).copied().unwrap_or(0.0) - CELL_H_PAD).max(8.0);
                            egui::Frame::NONE
                                .fill(fill)
                                .stroke(egui::Stroke::new(1.0_f32, crate::theme::STROKE))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.set_min_width(inner);
                                    ui.set_max_width(inner);
                                    let id = cell_id(start, r, c);
                                    if editable {
                                        if let Some(next) = arrow_jump(
                                            ui,
                                            id,
                                            &text,
                                            table.rows.len(),
                                            table.aligns.len(),
                                            r,
                                            c,
                                        ) {
                                            let idx = cursor_index(table, next.row, next.col, next.at_end);
                                            place_cursor(ui.ctx(), cell_id(start, next.row, next.col), idx);
                                            jump = Some(next);
                                        }
                                    }
                                    let out = ui
                                        .add_enabled_ui(editable, |ui| {
                                            egui::TextEdit::singleline(&mut text)
                                                .id(id)
                                                .desired_width(inner)
                                                .horizontal_align(align)
                                                .frame(false)
                                                .show(ui)
                                        })
                                        .inner;
                                    if let Some(next) = &jump {
                                        if next.row == r && next.col == c {
                                            out.response.request_focus();
                                            let idx = cursor_index(table, r, c, next.at_end);
                                            let mut state = out.state;
                                            state.cursor.set_char_range(Some(
                                                egui::text::CCursorRange::one(egui::text::CCursor::new(idx)),
                                            ));
                                            state.store(ui.ctx(), out.response.id);
                                        }
                                    }
                                    if out.response.has_focus() {
                                        any_focused = true;
                                    }
                                    if focus_used && r == 0 && c == 0 {
                                        out.response.request_focus();
                                        focus_used = false;
                                    }
                                    if out.response.changed() {
                                        cell_edit = Some((r, c, text.clone()));
                                    }
                                    if out.response.secondary_clicked() {
                                        if let Some(pos) = out.response.interact_pointer_pos() {
                                            open_menu(
                                                ui.ctx(),
                                                pos,
                                                start,
                                                r,
                                                c,
                                                table.aligns.len(),
                                            );
                                        }
                                    }
                                });
                        }
                        ui.end_row();
                    }
                });
        });
        table_width = frame.response.rect.width();
        if editable {
            add_col = hover_plus(
                ui,
                Vec2::new(ADD_GUTTER, frame.response.rect.height()),
                ("tbl_add_col", start),
            );
        }
    });
    if editable {
        add_row = hover_plus(ui, Vec2::new(table_width, ADD_GUTTER), ("tbl_add_row", start));
    }
    if any_focused || jump.is_some() {
        set_cell_focused(ui.ctx(), true);
    }
    if cell_edit.is_none() && !add_row && !add_col {
        None
    } else {
        Some(TableEdit {
            cell: cell_edit,
            add_row,
            add_col,
        })
    }
}

fn hover_plus(ui: &mut Ui, size: Vec2, salt: impl std::hash::Hash) -> bool {
    ui.push_id(salt, |ui| {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if response.hovered() {
            let btn = Rect::from_center_size(rect.center(), Vec2::splat(ADD_BTN));
            ui.painter()
                .rect_filled(btn, crate::theme::round_sm(), crate::theme::FAINT);
            ui.painter().rect_stroke(
                btn,
                crate::theme::round_sm(),
                egui::Stroke::new(1.0_f32, crate::theme::STROKE),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                btn.center(),
                egui::Align2::CENTER_CENTER,
                regular::PLUS,
                egui::FontId::new(14.0, crate::theme::display_family()),
                crate::theme::TEXT,
            );
            response.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        response.clicked()
    })
    .inner
}

fn cursor_index(table: &Table, row: usize, col: usize, at_end: bool) -> usize {
    if !at_end {
        return 0;
    }
    table
        .rows
        .get(row)
        .and_then(|cells| cells.get(col))
        .map(|text| text.chars().count())
        .unwrap_or(0)
}

fn open_menu(
    ctx: &egui::Context,
    pos: Pos2,
    start: usize,
    row: usize,
    col: usize,
    cols: usize,
) {
    crate::editor_menu::close(ctx);
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new(MENU_KEY),
            Some(MenuState {
                pos,
                start,
                row,
                col,
                cols,
                sub: None,
            }),
        )
    });
}

pub fn menu_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| {
        d.get_temp::<Option<MenuState>>(egui::Id::new(MENU_KEY))
            .flatten()
            .is_some()
    })
}

pub struct MenuChoice {
    pub start: usize,
    pub row: usize,
    pub col: usize,
    pub action: Action,
}

pub fn show_menu(ctx: &egui::Context) -> Option<MenuChoice> {
    let Some(mut menu) = ctx.data(|d| d.get_temp::<Option<MenuState>>(egui::Id::new(MENU_KEY)).flatten()) else {
        return None;
    };
    let mut action = None;
    let mut hit: Vec<Rect> = Vec::new();
    let main = egui::Area::new(egui::Id::new("bluelink_table_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                if row(ui, regular::ROWS, "Row", true) {
                    menu.sub = Some(SubMenu::Row);
                }
                if row(ui, regular::COLUMNS, "Column", true) {
                    menu.sub = Some(SubMenu::Column);
                }
                ui.separator();
                if row(ui, regular::SORT_ASCENDING, "Sort by column (A to Z)", false) {
                    action = Some(Action::SortAsc);
                }
                if row(ui, regular::SORT_DESCENDING, "Sort by column (Z to A)", false) {
                    action = Some(Action::SortDesc);
                }
            });
        });
    hit.push(main.response.rect);

    if let Some(sub) = menu.sub {
        let sub_pos = Pos2::new(main.response.rect.right() + 4.0, main.response.rect.top());
        let sub_area = egui::Area::new(egui::Id::new("bluelink_table_submenu"))
            .order(egui::Order::Foreground)
            .fixed_pos(sub_pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    match sub {
                        SubMenu::Row => {
                            if row(ui, regular::ROWS, "Add row above", false) {
                                action = Some(Action::AddRowAbove);
                            }
                            if row(ui, regular::ROWS, "Add row below", false) {
                                action = Some(Action::AddRowBelow);
                            }
                            ui.add_enabled_ui(menu.row > 0, |ui| {
                                if row(ui, regular::ARROW_UP, "Move row up", false) {
                                    action = Some(Action::MoveRowUp);
                                }
                            });
                            ui.separator();
                            if row(ui, regular::COPY, "Duplicate row", false) {
                                action = Some(Action::DuplicateRow);
                            }
                            if row(ui, regular::TRASH, "Delete row", false) {
                                action = Some(Action::DeleteRow);
                            }
                        }
                        SubMenu::Column => {
                            if row(ui, regular::ARROW_LINE_LEFT, "Add column to the left", false) {
                                action = Some(Action::AddColLeft);
                            }
                            if row(ui, regular::ARROW_LINE_RIGHT, "Add column to the right", false) {
                                action = Some(Action::AddColRight);
                            }
                            ui.add_enabled_ui(menu.col > 0, |ui| {
                                if row(ui, regular::ARROW_LEFT, "Move column left", false) {
                                    action = Some(Action::MoveColLeft);
                                }
                            });
                            ui.separator();
                            if row(ui, regular::TEXT_ALIGN_LEFT, "Align left", false) {
                                action = Some(Action::Align(Align::Left));
                            }
                            if row(ui, regular::TEXT_ALIGN_CENTER, "Align center", false) {
                                action = Some(Action::Align(Align::Center));
                            }
                            if row(ui, regular::TEXT_ALIGN_RIGHT, "Align right", false) {
                                action = Some(Action::Align(Align::Right));
                            }
                            ui.separator();
                            if row(ui, regular::COPY, "Duplicate column", false) {
                                action = Some(Action::DuplicateCol);
                            }
                            ui.add_enabled_ui(menu.cols > 1, |ui| {
                                if row(ui, regular::TRASH, "Delete column", false) {
                                    action = Some(Action::DeleteCol);
                                }
                            });
                        }
                    }
                });
            });
        hit.push(sub_area.response.rect);
    }

    if action.is_some() {
        close(ctx);
        return action.map(|action| MenuChoice {
            start: menu.start,
            row: menu.row,
            col: menu.col,
            action,
        });
    }

    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MENU_KEY), Some(menu)));

    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let click_outside = ctx.input(|i| {
        i.pointer.primary_clicked()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| hit.iter().all(|r| !r.contains(p)))
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Table {
        parse_slice(&[
            "| b | a |".into(),
            "| --- | --- |".into(),
            "| d | c |".into(),
            "| f | e |".into(),
        ])
        .unwrap()
    }

    #[test]
    fn hover_plus_appends_row_and_column() {
        let mut table = parse_slice(&[
            "| a |".into(),
            "| --- |".into(),
            "| b |".into(),
        ])
        .unwrap();
        assert!(apply_edit(
            &mut table,
            TableEdit {
                cell: None,
                add_row: true,
                add_col: false,
            },
        ));
        assert_eq!(table.rows.len(), 3);
        assert!(table.rows[2].iter().all(|cell| cell.is_empty()));
        assert!(apply_edit(
            &mut table,
            TableEdit {
                cell: None,
                add_row: false,
                add_col: true,
            },
        ));
        assert_eq!(table.aligns.len(), 2);
        assert!(table.rows.iter().all(|row| row.len() == 2 && row[1].is_empty()));
    }

    #[test]
    fn arrows_move_between_cells() {
        assert_eq!(neighbor(2, 2, 0, 0, Arrow::Right), Some((0, 1)));
        assert_eq!(neighbor(2, 2, 0, 1, Arrow::Right), None);
        assert_eq!(neighbor(2, 2, 0, 1, Arrow::Down), Some((1, 1)));
        assert_eq!(neighbor(2, 2, 1, 0, Arrow::Left), None);
        assert_eq!(neighbor(2, 2, 1, 1, Arrow::Left), Some((1, 0)));
        assert_eq!(neighbor(2, 2, 1, 0, Arrow::Up), Some((0, 0)));
        assert_eq!(neighbor(2, 2, 0, 0, Arrow::Up), None);
    }

    #[test]
    fn columns_start_equal_and_grow_with_text() {
        let avail = 200.0;
        let empty = column_widths(avail, &[0.0, 0.0]);
        assert_eq!(empty, vec![100.0, 100.0]);

        let grown = column_widths(avail, &[10.0, 150.0]);
        assert_eq!(grown[0], 100.0);
        assert!(grown[1] > 100.0);
        assert_eq!(grown[1], 150.0 + CELL_H_PAD + CARET_SLACK);
    }

    #[test]
    fn parse_default_snippet_and_places() {
        let lines: Vec<String> = SNIPPET.split('\n').map(str::to_string).collect();
        let places = places(&lines);
        assert_eq!(places[0], Place::Head { end: 3 });
        assert_eq!(places[1], Place::Tail);
        assert_eq!(places[2], Place::Tail);
        let table = parse_slice(&lines).unwrap();
        assert_eq!(table.aligns, vec![Align::Left, Align::Left]);
        assert_eq!(table.rows.len(), 2);
    }

    #[test]
    fn add_column_align_and_sort() {
        let mut table = sample();
        assert!(mutate(&mut table, 0, 1, Action::AddColRight));
        assert_eq!(table.aligns.len(), 3);
        assert!(table.rows.iter().all(|r| r.len() == 3));

        assert!(mutate(&mut table, 0, 0, Action::Align(Align::Center)));
        let md = to_markdown(&table);
        assert!(md.contains(":---:"), "{md}");

        let mut table = sample();
        assert!(mutate(&mut table, 1, 0, Action::SortAsc));
        assert_eq!(table.rows[0][0], "b");
        assert_eq!(table.rows[1][0], "d");
        assert_eq!(table.rows[2][0], "f");

        assert!(mutate(&mut table, 1, 0, Action::SortDesc));
        assert_eq!(table.rows[1][0], "f");
        assert_eq!(table.rows[2][0], "d");
    }

    #[test]
    fn delete_last_row_removes_table_and_last_column_stays() {
        let mut table = parse_slice(&["| a |".into(), "| --- |".into()]).unwrap();
        assert!(!mutate(&mut table, 0, 0, Action::DeleteRow));
        assert!(mutate(&mut table, 0, 0, Action::DeleteCol));
        assert_eq!(table.aligns.len(), 1);
    }
}
