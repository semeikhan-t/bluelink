//! Shared egui UI for BlueLink (desktop + Android later).

mod editor;
mod preview;
mod theme;
mod tree;
mod vault_registry;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bluelink_core::{validate_rel_path, NoteMeta, Vault, VaultError};
use eframe::egui;
use egui_phosphor::regular;
use vault_registry::{validate_vault_name, VaultRegistry};

enum Dialog {
    NewNote { name: String },
    NewFolder { name: String },
    NewVault { name: String },
    /// `from` is the current note/folder path; `name` is the editable target.
    Rename { from: String, name: String },
    CreateLink { rel_path: String },
    ConfirmDelete { path: String },
}

/// Folder picker supplied by the host (desktop uses `rfd`).
pub type FolderPicker = Box<dyn FnMut() -> Option<PathBuf> + Send>;

pub struct App {
    vault: Option<Vault>,
    notes: Vec<NoteMeta>,
    /// Relative folder paths (including empty).
    folders: Vec<String>,
    selected: Option<String>,
    buffer: String,
    dirty: bool,
    readonly: bool,
    last_edit: Option<Instant>,
    /// Line currently shown as raw Markdown in the live editor.
    active_line: Option<usize>,
    /// Full-document multiline edit (entered via Ctrl+A).
    doc_edit: bool,
    status: String,
    dialog: Option<Dialog>,
    reload_flag: Arc<AtomicBool>,
    pick_folder: Option<FolderPicker>,
    /// SHA of selected note as last loaded/saved — used to detect external edits.
    selected_sha: Option<String>,
    registry: VaultRegistry,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        vault_path: Option<PathBuf>,
        reload_flag: Arc<AtomicBool>,
        pick_folder: Option<FolderPicker>,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);

        let mut registry = VaultRegistry::load();
        registry.prune_missing();

        let mut app = Self {
            vault: None,
            notes: Vec::new(),
            folders: Vec::new(),
            selected: None,
            buffer: String::new(),
            dirty: false,
            readonly: false,
            last_edit: None,
            active_line: None,
            doc_edit: false,
            status: String::new(),
            dialog: None,
            reload_flag,
            pick_folder,
            selected_sha: None,
            registry,
        };

        // --vault wins; otherwise reopen last known vault.
        if let Some(path) = vault_path {
            app.open_vault_path(&path);
        } else if let Some(last) = app.registry.last_existing() {
            app.open_vault_path(&last);
        }
        let _ = app.registry.save();
        app
    }

    pub fn reload_flag(&self) -> Arc<AtomicBool> {
        self.reload_flag.clone()
    }

    pub fn vault_root(&self) -> Option<&Path> {
        self.vault.as_ref().map(|v| v.root())
    }

    /// Open (or create) a vault at `path`. Used by host after folder picker.
    pub fn open_vault(&mut self, path: &Path) {
        self.open_vault_path(path);
    }

    fn open_vault_path(&mut self, path: &Path) {
        if self.dirty {
            self.save_current();
        }
        if let Err(e) = std::fs::create_dir_all(path) {
            self.status = format!("cannot create vault: {e}");
            return;
        }
        match Vault::open(path) {
            Ok(vault) => {
                self.status = format!("opened {}", vault.root().display());
                self.registry.remember(vault.root());
                let _ = self.registry.save();
                self.vault = Some(vault);
                self.selected = None;
                self.buffer.clear();
                self.dirty = false;
                self.readonly = false;
                self.selected_sha = None;
                self.active_line = None;
                self.doc_edit = false;
                self.refresh_notes();
            }
            Err(e) => {
                self.status = format!("open failed: {e}");
                self.vault = None;
                self.folders.clear();
            }
        }
    }

    fn close_vault(&mut self) {
        if self.dirty {
            self.save_current();
        }
        self.vault = None;
        self.notes.clear();
        self.folders.clear();
        self.selected = None;
        self.buffer.clear();
        self.dirty = false;
        self.readonly = false;
        self.selected_sha = None;
        self.active_line = None;
        self.doc_edit = false;
    }

    fn pick_open_vault(&mut self) {
        let Some(pick) = self.pick_folder.as_mut() else {
            self.status = "pass --vault <path>".into();
            return;
        };
        if let Some(path) = pick() {
            self.open_vault_path(&path);
        }
    }

    fn create_named_vault(&mut self, name: &str) {
        if let Err(e) = validate_vault_name(name) {
            self.status = e;
            return;
        }
        let path = vault_registry::default_vaults_parent().join(name.trim());
        if path.exists() && !path.is_dir() {
            self.status = format!("{} exists and is not a folder", path.display());
            return;
        }
        self.open_vault_path(&path);
    }

    fn forget_vault(&mut self, path: &Path) {
        let active = self.vault_root().map(|r| r.to_path_buf());
        self.registry.forget(path);
        let _ = self.registry.save();
        let closing = active.as_ref().is_some_and(|a| {
            a == path
                || a.canonicalize().ok() == path.canonicalize().ok()
        });
        if closing {
            self.close_vault();
            self.status = "vault forgotten".into();
        }
    }

    fn refresh_notes(&mut self) {
        if let Some(vault) = &self.vault {
            match vault.list() {
                Ok(notes) => self.notes = notes,
                Err(e) => self.status = format!("list failed: {e}"),
            }
            match vault.list_folders() {
                Ok(folders) => self.folders = folders,
                Err(e) => self.status = format!("list folders failed: {e}"),
            }
        }
    }

    fn create_folder(&mut self, rel: &str) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        match vault.create_folder(rel) {
            Ok(()) => {
                self.status = format!("created folder {rel}");
                self.refresh_notes();
            }
            Err(e) => self.status = format!("folder failed: {e}"),
        }
    }

    fn save_current(&mut self) {
        if !self.dirty || self.readonly {
            return;
        }
        let Some(rel) = self.selected.clone() else {
            return;
        };
        let content = self.buffer.clone();
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        match vault.write(&rel, &content) {
            Ok(meta) => {
                self.dirty = false;
                self.last_edit = None;
                self.selected_sha = Some(meta.sha256);
                self.status.clear();
                if let Ok(notes) = vault.list() {
                    self.notes = notes;
                }
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn select_note(&mut self, rel_path: &str) {
        if self.selected.as_deref() == Some(rel_path) {
            return;
        }
        if self.dirty {
            self.save_current();
        }
        let Some(vault) = &self.vault else {
            return;
        };
        match vault.read(rel_path) {
            Ok(text) => {
                self.buffer = text;
                self.readonly = false;
                self.dirty = false;
                self.last_edit = None;
                self.active_line = None;
                self.doc_edit = false;
                self.selected = Some(rel_path.to_string());
                self.selected_sha = self
                    .notes
                    .iter()
                    .find(|n| n.rel_path == rel_path)
                    .map(|n| n.sha256.clone());
                self.status.clear();
            }
            Err(VaultError::NotUtf8) => {
                self.buffer.clear();
                self.readonly = true;
                self.dirty = false;
                self.last_edit = None;
                self.active_line = None;
                self.doc_edit = false;
                self.selected = Some(rel_path.to_string());
                self.selected_sha = None;
                self.status = "note is not UTF-8 (read-only)".into();
            }
            Err(e) => {
                self.status = format!("read failed: {e}");
            }
        }
    }

    fn handle_wikilink(&mut self, raw: &str) {
        let Some(vault) = &self.vault else {
            return;
        };
        if let Some(path) = vault.index().resolve(raw) {
            let path = path.clone();
            self.select_note(&path);
            return;
        }

        let target = bluelink_core::normalize_link_target(raw);
        let rel = if target.ends_with(".md") {
            target
        } else {
            format!("{target}.md")
        };
        self.dialog = Some(Dialog::CreateLink { rel_path: rel });
    }

    fn create_note(&mut self, rel_path: &str, initial: &str) {
        if let Err(e) = validate_rel_path(rel_path) {
            self.status = format!("invalid path: {e}");
            return;
        }
        if self.dirty {
            self.save_current();
        }
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        match vault.write(rel_path, initial) {
            Ok(_) => {
                self.refresh_notes();
                self.select_note(rel_path);
                self.status = format!("created {rel_path}");
            }
            Err(e) => self.status = format!("create failed: {e}"),
        }
    }

    fn rename_path(&mut self, from: &str, new_name: &str) {
        if self.folders.iter().any(|f| f == from) {
            let to = new_name.trim().trim_matches('/').to_string();
            if to.is_empty() || to == from {
                return;
            }
            let Some(vault) = self.vault.as_mut() else {
                return;
            };
            match vault.rename_folder(from, &to) {
                Ok(()) => {
                    let prefix = format!("{from}/");
                    let selected_was_inside = self
                        .selected
                        .as_ref()
                        .is_some_and(|s| s.starts_with(&prefix));
                    let new_selected = self.selected.as_ref().and_then(|s| {
                        s.strip_prefix(&prefix)
                            .map(|rest| format!("{to}/{rest}"))
                    });
                    self.refresh_notes();
                    if selected_was_inside {
                        if let Some(path) = new_selected {
                            self.select_note(&path);
                        }
                    }
                    self.status = format!("renamed folder to {to}");
                }
                Err(e) => self.status = format!("rename folder failed: {e}"),
            }
            return;
        }
        let to = normalize_note_path(new_name);
        self.move_note(from, &to);
    }

    fn move_note(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        if self.dirty && self.selected.as_deref() == Some(from) {
            self.save_current();
        }
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        match vault.rename(from, to) {
            Ok(_) => {
                self.refresh_notes();
                if self.selected.as_deref() == Some(from) {
                    self.select_note(to);
                }
                self.status = format!("moved to {to}");
            }
            Err(e) => self.status = format!("move failed: {e}"),
        }
    }

    fn delete_path(&mut self, rel: &str) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        if self.folders.iter().any(|f| f == rel) {
            let prefix = format!("{rel}/");
            let clear_selected = self
                .selected
                .as_ref()
                .is_some_and(|s| s.starts_with(&prefix));
            match vault.delete_folder(rel) {
                Ok(()) => {
                    if clear_selected {
                        self.selected = None;
                        self.buffer.clear();
                        self.dirty = false;
                        self.readonly = false;
                        self.selected_sha = None;
                        self.active_line = None;
                        self.doc_edit = false;
                    }
                    self.refresh_notes();
                    self.status = format!("deleted folder {rel}");
                }
                Err(e) => self.status = format!("delete folder failed: {e}"),
            }
            return;
        }
        match vault.delete(rel) {
            Ok(()) => {
                if self.selected.as_deref() == Some(rel) {
                    self.selected = None;
                    self.buffer.clear();
                    self.dirty = false;
                    self.readonly = false;
                    self.selected_sha = None;
                    self.active_line = None;
                    self.doc_edit = false;
                }
                self.refresh_notes();
                self.status = format!("deleted {rel}");
            }
            Err(e) => self.status = format!("delete failed: {e}"),
        }
    }

    fn apply_external_reload(&mut self) {
        if !self.reload_flag.swap(false, Ordering::SeqCst) {
            return;
        }
        // Always refresh the tree; only reload buffer when not dirty.
        self.refresh_notes();
        if self.dirty {
            return;
        }
        let Some(rel) = self.selected.clone() else {
            return;
        };
        let new_sha = self
            .notes
            .iter()
            .find(|n| n.rel_path == rel)
            .map(|n| n.sha256.clone());
        if new_sha.is_none() {
            // File vanished externally.
            self.selected = None;
            self.buffer.clear();
            self.selected_sha = None;
            self.status = "note removed on disk".into();
            return;
        }
        if new_sha == self.selected_sha {
            return;
        }
        let Some(vault) = &self.vault else {
            return;
        };
        match vault.read(&rel) {
            Ok(text) => {
                self.buffer = text;
                self.readonly = false;
                self.selected_sha = new_sha;
                self.active_line = None;
                self.doc_edit = false;
                self.status = format!("reloaded from disk {}", hhmm_now());
            }
            Err(VaultError::NotUtf8) => {
                self.buffer.clear();
                self.readonly = true;
                self.selected_sha = new_sha;
                self.active_line = None;
                self.doc_edit = false;
                self.status = "note is not UTF-8 (read-only)".into();
            }
            Err(e) => self.status = format!("reload failed: {e}"),
        }
    }

    fn show_no_vault(&mut self, ctx: &egui::Context) {
        self.show_dialogs(ctx);

        let mut open_path: Option<PathBuf> = None;
        let mut forget_path: Option<PathBuf> = None;
        let mut do_pick = false;
        let mut do_create = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.label(
                    egui::RichText::new("BlueLink")
                        .size(42.0)
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Vaults are folders of Markdown notes.")
                        .size(18.0)
                        .color(theme::WEAK),
                );
                ui.add_space(24.0);

                ui.horizontal(|ui| {
                    let open = egui::Button::new(
                        egui::RichText::new(format!("{}  Open", regular::FOLDER_OPEN)).size(16.0),
                    )
                    .min_size(egui::vec2(140.0, 40.0));
                    if ui.add(open).clicked() {
                        do_pick = true;
                    }
                    let create = egui::Button::new(
                        egui::RichText::new(format!("{}  Create", regular::PLUS)).size(16.0),
                    )
                    .min_size(egui::vec2(140.0, 40.0));
                    if ui.add(create).clicked() {
                        do_create = true;
                    }
                });

                if !self.registry.entries.is_empty() {
                    ui.add_space(28.0);
                    ui.label(
                        egui::RichText::new("Known vaults")
                            .size(15.0)
                            .color(theme::WEAK),
                    );
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for entry in &self.registry.entries {
                                let name = VaultRegistry::display_name(&entry.path);
                                ui.horizontal(|ui| {
                                    let label = format!("{}  {name}", regular::FOLDER);
                                    if ui
                                        .add_sized(
                                            [280.0, 32.0],
                                            egui::SelectableLabel::new(false, label),
                                        )
                                        .on_hover_text(entry.path.display().to_string())
                                        .clicked()
                                    {
                                        open_path = Some(entry.path.clone());
                                    }
                                    if ui
                                        .small_button(regular::X.to_string())
                                        .on_hover_text("Forget (keeps files on disk)")
                                        .clicked()
                                    {
                                        forget_path = Some(entry.path.clone());
                                    }
                                });
                            }
                        });
                }

                if !self.status.is_empty() {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(&self.status).color(theme::WARN));
                }
            });
        });

        if do_pick {
            self.pick_open_vault();
        }
        if do_create {
            self.dialog = Some(Dialog::NewVault {
                name: "Notes".into(),
            });
        }
        if let Some(path) = open_path {
            self.open_vault_path(&path);
        }
        if let Some(path) = forget_path {
            self.forget_vault(&path);
        }
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut confirm_create: Option<String> = None;
        let mut confirm_new: Option<String> = None;
        let mut confirm_rename: Option<(String, String)> = None;
        let mut do_delete: Option<String> = None;

        let mut confirm_vault: Option<String> = None;
        let mut confirm_folder: Option<String> = None;

        match &mut self.dialog {
            Some(Dialog::NewNote { name }) => {
                egui::Window::new("New note")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Name (e.g. inbox/idea) — .md is added automatically:");
                        ui.text_edit_singleline(name);
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() {
                                confirm_new = Some(name.clone());
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::NewFolder { name }) => {
                egui::Window::new("New folder")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Name (e.g. inbox or projects/work):");
                        ui.text_edit_singleline(name);
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() {
                                confirm_folder = Some(name.clone());
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::NewVault { name }) => {
                egui::Window::new("New vault")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Name — created under ~/BlueLink/:");
                        ui.text_edit_singleline(name);
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() {
                                confirm_vault = Some(name.clone());
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::Rename { from, name }) => {
                egui::Window::new("Rename")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.text_edit_singleline(name);
                        ui.horizontal(|ui| {
                            if ui.button("Rename").clicked() {
                                confirm_rename = Some((from.clone(), name.clone()));
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::CreateLink { rel_path }) => {
                let path = rel_path.clone();
                egui::Window::new("Create note")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(format!("Note `{path}` does not exist. Create it?"));
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() {
                                confirm_create = Some(path.clone());
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::ConfirmDelete { path }) => {
                let name = path.clone();
                let kind = if self.folders.iter().any(|f| f == &name) {
                    "folder"
                } else {
                    "note"
                };
                egui::Window::new(format!("Delete {kind}"))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(format!("Delete `{name}`?"));
                        ui.horizontal(|ui| {
                            if ui.button("Delete").clicked() {
                                do_delete = Some(name.clone());
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            None => {}
        }

        if close {
            self.dialog = None;
        }
        if let Some(path) = confirm_create {
            let title = path
                .trim_end_matches(".md")
                .rsplit('/')
                .next()
                .unwrap_or("Note");
            self.create_note(&path, &format!("# {title}\n"));
        }
        if let Some(name) = confirm_new {
            let path = normalize_note_path(&name);
            let title = path
                .trim_end_matches(".md")
                .rsplit('/')
                .next()
                .unwrap_or("Note");
            self.create_note(&path, &format!("# {title}\n"));
        }
        if let Some(name) = confirm_vault {
            self.create_named_vault(&name);
        }
        if let Some(name) = confirm_folder {
            let rel = name.trim().trim_matches('/').to_string();
            self.create_folder(&rel);
        }
        if let Some((from, name)) = confirm_rename {
            self.rename_path(&from, &name);
        }
        if let Some(path) = do_delete {
            self.delete_path(&path);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_external_reload();

        if self.vault.is_none() {
            self.show_no_vault(ctx);
            return;
        }

        // Autosave after 1s idle.
        if self.dirty {
            if let Some(t) = self.last_edit {
                if t.elapsed() >= Duration::from_secs(1) {
                    self.save_current();
                } else {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            }
        }

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save_current();
        }

        self.show_dialogs(ctx);

        egui::TopBottomPanel::bottom("status")
            .exact_height(36.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new(&self.status).color(theme::WEAK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // right-to-left: time first → rightmost; then saved/unsaved to its left
                        if let Some(t) = self.selected_mtime_hhmm() {
                            ui.label(egui::RichText::new(t).color(theme::WEAK));
                            ui.add_space(10.0);
                        }
                        if self.dirty {
                            ui.colored_label(theme::WARN, format!("{} unsaved", regular::CIRCLE));
                        } else {
                            ui.colored_label(theme::WEAK, format!("{} saved", regular::CHECK));
                        }
                    });
                });
            });

        let mut switch_to: Option<PathBuf> = None;
        let mut forget_active = false;
        let mut pick_vault = false;
        let mut create_vault = false;

        egui::SidePanel::left("tree")
            .default_width(theme::SIDEBAR_WIDTH)
            .min_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("BlueLink")
                        .size(20.0)
                        .strong()
                        .color(theme::ACCENT),
                );

                let vault_name = self
                    .vault_root()
                    .map(VaultRegistry::display_name)
                    .unwrap_or_else(|| "Vault".into());
                egui::ComboBox::from_id_salt("vault_switcher")
                    .width(ui.available_width() - 8.0)
                    .selected_text(format!("{}  {vault_name}", regular::FOLDER))
                    .show_ui(ui, |ui| {
                        for entry in &self.registry.entries {
                            let name = VaultRegistry::display_name(&entry.path);
                            let selected = self.vault_root().is_some_and(|r| {
                                r == entry.path.as_path()
                                    || r.canonicalize().ok() == entry.path.canonicalize().ok()
                            });
                            if ui
                                .selectable_label(selected, &name)
                                .on_hover_text(entry.path.display().to_string())
                                .clicked()
                            {
                                switch_to = Some(entry.path.clone());
                            }
                        }
                        ui.separator();
                        if ui
                            .button(format!("{}  Open…", regular::FOLDER_OPEN))
                            .clicked()
                        {
                            pick_vault = true;
                        }
                        if ui.button(format!("{}  Create…", regular::PLUS)).clicked() {
                            create_vault = true;
                        }
                        if ui
                            .button(format!("{}  Forget current", regular::X))
                            .clicked()
                        {
                            forget_active = true;
                        }
                    });

                ui.label(egui::RichText::new("Notes").size(14.0).color(theme::WEAK));
                ui.add_space(8.0);
                let action =
                    tree::show_tree(ui, &self.notes, &self.folders, self.selected.as_deref());
                if let Some(path) = action.select {
                    self.select_note(&path);
                }
                if action.new_note {
                    self.dialog = Some(Dialog::NewNote {
                        name: "untitled".into(),
                    });
                }
                if action.new_folder {
                    self.dialog = Some(Dialog::NewFolder {
                        name: "inbox".into(),
                    });
                }
                if let Some(path) = action.rename {
                    self.dialog = Some(Dialog::Rename {
                        from: path.clone(),
                        name: path,
                    });
                }
                if let Some(path) = action.delete {
                    self.dialog = Some(Dialog::ConfirmDelete { path });
                }
                if let Some((from, to)) = action.move_note {
                    self.move_note(&from, &to);
                }
            });

        if let Some(path) = switch_to {
            self.open_vault_path(&path);
        }
        if pick_vault {
            self.pick_open_vault();
        }
        if create_vault {
            self.dialog = Some(Dialog::NewVault {
                name: "Notes".into(),
            });
        }
        if forget_active {
            if let Some(root) = self.vault_root().map(|p| p.to_path_buf()) {
                self.forget_vault(&root);
            }
        }

        let mut open_backlink: Option<String> = None;
        egui::SidePanel::right("backlinks")
            .default_width(240.0)
            .min_width(180.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Backlinks")
                        .size(16.0)
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                match (self.selected.as_ref(), self.vault.as_ref()) {
                    (Some(sel), Some(vault)) => {
                        let links = vault.index().backlinks(sel);
                        if links.is_empty() {
                            ui.label(
                                egui::RichText::new("No backlinks")
                                    .size(14.0)
                                    .color(theme::WEAK),
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for path in links {
                                        let name = path
                                            .rsplit('/')
                                            .next()
                                            .unwrap_or(&path)
                                            .trim_end_matches(".md");
                                        let label = format!("{}  {name}", regular::LINK);
                                        if ui
                                            .add_sized(
                                                [ui.available_width(), 28.0],
                                                egui::SelectableLabel::new(false, label),
                                            )
                                            .on_hover_text(&path)
                                            .clicked()
                                        {
                                            open_backlink = Some(path);
                                        }
                                    }
                                });
                        }
                    }
                    _ => {
                        ui.label(
                            egui::RichText::new("Select a note")
                                .size(14.0)
                                .color(theme::WEAK),
                        );
                    }
                }
            });
        if let Some(path) = open_backlink {
            self.select_note(&path);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(sel) = &self.selected {
                    ui.label(egui::RichText::new(sel).size(15.0).strong());
                } else {
                    ui.label(egui::RichText::new("Note").size(15.0).color(theme::WEAK));
                }
                if self.readonly {
                    ui.separator();
                    ui.colored_label(theme::WARN, "read-only");
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(8.0);

            if self.selected.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Select or create a note")
                            .size(20.0)
                            .color(theme::WEAK),
                    );
                });
                return;
            }

            // Take vault temporarily so we can mutate buffer alongside an immutable vault borrow.
            let vault = self.vault.take();
            let mut wiki_click = None;
            if let Some(ref v) = vault {
                let live = editor::show_live(
                    ui,
                    v,
                    &mut self.buffer,
                    &mut self.active_line,
                    &mut self.doc_edit,
                    self.readonly,
                );
                if live.changed {
                    self.dirty = true;
                    self.last_edit = Some(Instant::now());
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
                wiki_click = live.wikilink;
            }
            self.vault = vault;
            if let Some(raw) = wiki_click {
                self.handle_wikilink(&raw);
            }
        });
    }
}

impl App {
    /// Last-modified clock time of the open note (from file mtime / NoteMeta).
    fn selected_mtime_hhmm(&self) -> Option<String> {
        let rel = self.selected.as_ref()?;
        let ms = self
            .notes
            .iter()
            .find(|n| n.rel_path == *rel)
            .map(|n| n.modified)?;
        Some(hhmm_from_unix_ms(ms))
    }
}

fn normalize_note_path(input: &str) -> String {
    let trimmed = input.trim().trim_start_matches('/');
    if trimmed.ends_with(".md") {
        trimmed.to_string()
    } else {
        format!("{trimmed}.md")
    }
}

fn hhmm_from_unix_ms(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms as i64) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.format("%H:%M").to_string()
        }
        chrono::LocalResult::None => "--:--".into(),
    }
}

fn hhmm_now() -> String {
    use chrono::Local;
    Local::now().format("%H:%M").to_string()
}
