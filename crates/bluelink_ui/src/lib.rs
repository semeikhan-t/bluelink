//! Shared egui UI for BlueLink (desktop + Android later).

mod editor;
mod preview;
mod tabs;
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
    CreateLink { rel_path: String },
}

/// One undoable vault change (delete for now).
enum UndoItem {
    DeletedNote { path: String, content: String },
    DeletedFolder {
        path: String,
        /// Notes that lived under the folder, with contents.
        notes: Vec<(String, String)>,
    },
}

/// Folder picker supplied by the host (desktop uses `rfd`).
pub type FolderPicker = Box<dyn FnMut() -> Option<PathBuf> + Send>;

/// One editor tab with its own back/forward history.
struct OpenTab {
    path: String,
    history: Vec<String>,
    history_index: usize,
}

impl OpenTab {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            history: vec![path.to_string()],
            history_index: 0,
        }
    }
}

pub struct App {
    vault: Option<Vault>,
    notes: Vec<NoteMeta>,
    /// Relative folder paths (including empty).
    folders: Vec<String>,
    /// Open note tabs (browser-style), active = `selected`.
    open_tabs: Vec<OpenTab>,
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
    /// Sidebar inline rename (note or folder).
    renaming: Option<tree::InlineRename>,
    reload_flag: Arc<AtomicBool>,
    pick_folder: Option<FolderPicker>,
    /// SHA of selected note as last loaded/saved — used to detect external edits.
    selected_sha: Option<String>,
    registry: VaultRegistry,
    undo_stack: Vec<UndoItem>,
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
            open_tabs: Vec::new(),
            selected: None,
            buffer: String::new(),
            dirty: false,
            readonly: false,
            last_edit: None,
            active_line: None,
            doc_edit: false,
            status: String::new(),
            dialog: None,
            renaming: None,
            reload_flag,
            pick_folder,
            selected_sha: None,
            registry,
            undo_stack: Vec::new(),
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
                self.open_tabs.clear();
                self.undo_stack.clear();
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
                self.open_tabs.clear();
                self.undo_stack.clear();
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
        self.open_tabs.clear();
        self.undo_stack.clear();
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

    fn tab_paths(&self) -> Vec<String> {
        self.open_tabs.iter().map(|t| t.path.clone()).collect()
    }

    fn active_tab_index(&self) -> Option<usize> {
        let sel = self.selected.as_deref()?;
        self.open_tabs.iter().position(|t| t.path == sel)
    }

    /// Open a note in the current tab (replace), or switch to it if already open.
    fn select_note(&mut self, rel_path: &str) {
        self.open_note(rel_path, false, true);
    }

    /// Open a note in a new tab (or focus existing tab for that path).
    fn open_note_in_new_tab(&mut self, rel_path: &str) {
        self.open_note(rel_path, true, true);
    }

    fn open_note(&mut self, rel_path: &str, new_tab: bool, record_history: bool) {
        let already_open = self.open_tabs.iter().any(|t| t.path == rel_path);
        if already_open {
            // Just focus the existing tab.
        } else if new_tab {
            self.open_tabs.push(OpenTab::new(rel_path));
        } else if let Some(cur) = self.selected.clone() {
            // In-place: replace the active tab's path; keep its history stack.
            if let Some(tab) = self.open_tabs.iter_mut().find(|t| t.path == cur) {
                tab.path = rel_path.to_string();
            } else {
                self.open_tabs.push(OpenTab::new(rel_path));
            }
        } else {
            self.open_tabs.push(OpenTab::new(rel_path));
        }

        if self.selected.as_deref() == Some(rel_path) {
            return;
        }
        if !self.load_note_content(rel_path) {
            return;
        }
        if record_history {
            self.record_nav(rel_path);
        }
    }

    /// Load buffer/selection for `rel_path`. Returns false if the note could not be read.
    fn load_note_content(&mut self, rel_path: &str) -> bool {
        if self.dirty {
            self.save_current();
        }
        let Some(vault) = &self.vault else {
            return false;
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
                true
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
                true
            }
            Err(e) => {
                self.status = format!("read failed: {e}");
                false
            }
        }
    }

    fn record_nav(&mut self, path: &str) {
        let Some(tab) = self.open_tabs.iter_mut().find(|t| t.path == path) else {
            return;
        };
        if tab.history.get(tab.history_index).map(String::as_str) == Some(path) {
            return;
        }
        tab.history.truncate(tab.history_index.saturating_add(1));
        tab.history.push(path.to_string());
        tab.history_index = tab.history.len() - 1;
    }

    fn can_go_back(&self) -> bool {
        self.active_tab_index()
            .is_some_and(|i| self.open_tabs[i].history_index > 0)
    }

    fn can_go_forward(&self) -> bool {
        self.active_tab_index().is_some_and(|i| {
            let tab = &self.open_tabs[i];
            tab.history_index + 1 < tab.history.len()
        })
    }

    fn go_back(&mut self) {
        let Some(tab_i) = self.active_tab_index() else {
            return;
        };
        while self.open_tabs[tab_i].history_index > 0 {
            self.open_tabs[tab_i].history_index -= 1;
            let path = self.open_tabs[tab_i].history[self.open_tabs[tab_i].history_index].clone();
            if self.notes.iter().any(|n| n.rel_path == path) {
                self.open_tabs[tab_i].path = path.clone();
                let _ = self.load_note_content(&path);
                return;
            }
        }
    }

    fn go_forward(&mut self) {
        let Some(tab_i) = self.active_tab_index() else {
            return;
        };
        while self.open_tabs[tab_i].history_index + 1 < self.open_tabs[tab_i].history.len() {
            self.open_tabs[tab_i].history_index += 1;
            let path = self.open_tabs[tab_i].history[self.open_tabs[tab_i].history_index].clone();
            if self.notes.iter().any(|n| n.rel_path == path) {
                self.open_tabs[tab_i].path = path.clone();
                let _ = self.load_note_content(&path);
                return;
            }
        }
    }

    fn show_nav_buttons(&mut self, ui: &mut egui::Ui) {
        // Nudge down to sit with the tab baseline.
        ui.vertical(|ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let back = ui
                    .add_enabled(
                        self.can_go_back(),
                        egui::Button::new(egui::RichText::new(regular::ARROW_LEFT).size(16.0))
                            .min_size(egui::vec2(32.0, 32.0))
                            .frame(false),
                    )
                    .on_hover_text("Back (Alt+←)");
                let fwd = ui
                    .add_enabled(
                        self.can_go_forward(),
                        egui::Button::new(egui::RichText::new(regular::ARROW_RIGHT).size(16.0))
                            .min_size(egui::vec2(32.0, 32.0))
                            .frame(false),
                    )
                    .on_hover_text("Forward (Alt+→)");
                if back.clicked() {
                    self.go_back();
                }
                if fwd.clicked() {
                    self.go_forward();
                }
            });
        });
    }

    fn cycle_next_tab(&mut self) {
        if self.open_tabs.len() < 2 {
            return;
        }
        let idx = self.active_tab_index().unwrap_or(0);
        let next = (idx + 1) % self.open_tabs.len();
        let path = self.open_tabs[next].path.clone();
        self.select_note(&path);
    }

    fn close_tab(&mut self, path: &str) {
        let idx = self.open_tabs.iter().position(|t| t.path == path);
        let was_active = self.selected.as_deref() == Some(path);
        self.open_tabs.retain(|t| t.path != path);
        if !was_active {
            return;
        }
        if self.dirty {
            self.save_current();
        }
        let next = idx.and_then(|i| {
            if i < self.open_tabs.len() {
                Some(self.open_tabs[i].path.clone())
            } else {
                self.open_tabs.last().map(|t| t.path.clone())
            }
        });
        self.selected = None;
        self.buffer.clear();
        self.dirty = false;
        self.readonly = false;
        self.selected_sha = None;
        self.active_line = None;
        self.doc_edit = false;
        if let Some(path) = next {
            self.select_note(&path);
        }
    }

    fn retarget_tab(&mut self, from: &str, to: &str) {
        for tab in &mut self.open_tabs {
            if tab.path == from {
                tab.path = to.to_string();
            }
            for entry in &mut tab.history {
                if entry == from {
                    *entry = to.to_string();
                }
            }
        }
    }

    fn drop_tabs_with_prefix(&mut self, prefix: &str) {
        let p = format!("{prefix}/");
        self.open_tabs
            .retain(|t| t.path != prefix && !t.path.starts_with(&p));
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

    /// Next free `untitled.md` / `untitled1.md` / `untitled2.md` …
    fn next_untitled_rel(&self) -> String {
        let taken = |stem: &str| {
            let path = format!("{stem}.md");
            self.notes.iter().any(|n| n.rel_path == path)
                || self.open_tabs.iter().any(|t| t.path == path)
        };
        if !taken("untitled") {
            return "untitled.md".into();
        }
        for i in 1..10_000 {
            let stem = format!("untitled{i}");
            if !taken(&stem) {
                return format!("{stem}.md");
            }
        }
        "untitled.md".into()
    }

    /// Ctrl+T: create untitled note and open it in a new tab (no name dialog).
    fn create_untitled_tab(&mut self) {
        let path = self.next_untitled_rel();
        let title = path.trim_end_matches(".md");
        if self.dirty {
            self.save_current();
        }
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        match vault.write(&path, &format!("# {title}\n")) {
            Ok(_) => {
                self.refresh_notes();
                self.open_note_in_new_tab(&path);
                self.status = format!("created {path}");
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
                    let new_prefix = format!("{to}/");
                    for tab in &mut self.open_tabs {
                        if let Some(rest) = tab.path.strip_prefix(&prefix) {
                            tab.path = format!("{new_prefix}{rest}");
                        }
                        for entry in &mut tab.history {
                            if let Some(rest) = entry.strip_prefix(&prefix) {
                                *entry = format!("{new_prefix}{rest}");
                            }
                        }
                    }
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
                self.retarget_tab(from, to);
                self.refresh_notes();
                if self.selected.as_deref() == Some(from) {
                    self.select_note(to);
                }
                self.status = format!("moved to {to}");
            }
            Err(e) => self.status = format!("move failed: {e}"),
        }
    }

    fn push_undo(&mut self, item: UndoItem) {
        self.undo_stack.push(item);
        const LIMIT: usize = 32;
        if self.undo_stack.len() > LIMIT {
            self.undo_stack.remove(0);
        }
    }

    fn delete_path(&mut self, rel: &str) {
        if self.folders.iter().any(|f| f == rel) {
            let prefix = format!("{rel}/");
            // Snapshot note contents before delete for undo.
            let mut notes = Vec::new();
            if let Some(vault) = &self.vault {
                for meta in &self.notes {
                    if meta.rel_path.starts_with(&prefix) {
                        if let Ok(content) = vault.read(&meta.rel_path) {
                            notes.push((meta.rel_path.clone(), content));
                        }
                    }
                }
            }
            let clear_selected = self
                .selected
                .as_ref()
                .is_some_and(|s| s.starts_with(&prefix));
            let result = self
                .vault
                .as_mut()
                .map(|v| v.delete_folder(rel));
            match result {
                Some(Ok(())) => {
                    self.push_undo(UndoItem::DeletedFolder {
                        path: rel.to_string(),
                        notes,
                    });
                    self.drop_tabs_with_prefix(rel);
                    if clear_selected {
                        self.selected = None;
                        self.buffer.clear();
                        self.dirty = false;
                        self.readonly = false;
                        self.selected_sha = None;
                        self.active_line = None;
                        self.doc_edit = false;
                        if let Some(path) = self.open_tabs.first().map(|t| t.path.clone()) {
                            self.select_note(&path);
                        }
                    }
                    self.refresh_notes();
                    self.status = format!("deleted folder {rel} — Ctrl+Z to undo");
                }
                Some(Err(e)) => self.status = format!("delete folder failed: {e}"),
                None => {}
            }
            return;
        }

        // Prefer unsaved buffer if this is the open note.
        if self.dirty && self.selected.as_deref() == Some(rel) {
            self.save_current();
        }
        let content = {
            let Some(vault) = &self.vault else {
                return;
            };
            match vault.read(rel) {
                Ok(c) => c,
                Err(e) => {
                    self.status = format!("delete failed: {e}");
                    return;
                }
            }
        };
        let result = self.vault.as_mut().map(|v| v.delete(rel));
        match result {
            Some(Ok(())) => {
                self.push_undo(UndoItem::DeletedNote {
                    path: rel.to_string(),
                    content,
                });
                let was_active = self.selected.as_deref() == Some(rel);
                let idx = self.open_tabs.iter().position(|t| t.path == rel);
                self.open_tabs.retain(|t| t.path != rel);
                if was_active {
                    self.selected = None;
                    self.buffer.clear();
                    self.dirty = false;
                    self.readonly = false;
                    self.selected_sha = None;
                    self.active_line = None;
                    self.doc_edit = false;
                    let next = idx.and_then(|i| {
                        if i < self.open_tabs.len() {
                            Some(self.open_tabs[i].path.clone())
                        } else {
                            self.open_tabs.last().map(|t| t.path.clone())
                        }
                    });
                    if let Some(path) = next {
                        self.select_note(&path);
                    }
                }
                self.refresh_notes();
                self.status = format!("deleted {rel} — Ctrl+Z to undo");
            }
            Some(Err(e)) => self.status = format!("delete failed: {e}"),
            None => {}
        }
    }

    fn undo(&mut self) {
        let Some(item) = self.undo_stack.pop() else {
            self.status = "nothing to undo".into();
            return;
        };
        match item {
            UndoItem::DeletedNote { path, content } => {
                let title = path
                    .trim_end_matches(".md")
                    .rsplit('/')
                    .next()
                    .unwrap_or("Note");
                let body = if content.is_empty() {
                    format!("# {title}\n")
                } else {
                    content
                };
                let result = self.vault.as_mut().map(|v| v.write(&path, &body));
                match result {
                    Some(Ok(_)) => {
                        self.refresh_notes();
                        self.open_note_in_new_tab(&path);
                        self.status = format!("restored {path}");
                    }
                    Some(Err(e)) => self.status = format!("undo failed: {e}"),
                    None => {}
                }
            }
            UndoItem::DeletedFolder { path, notes } => {
                let restore_err = {
                    let Some(vault) = self.vault.as_mut() else {
                        return;
                    };
                    if let Err(e) = vault.create_folder(&path) {
                        Some(e)
                    } else {
                        notes.iter().find_map(|(note_path, content)| {
                            vault.write(note_path, content).err()
                        })
                    }
                };
                if let Some(e) = restore_err {
                    self.refresh_notes();
                    self.status = format!("undo failed: {e}");
                    return;
                }
                self.refresh_notes();
                if let Some((first, _)) = notes.first() {
                    self.open_note_in_new_tab(first);
                }
                self.status = format!("restored folder {path}");
            }
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

        // Don't steal Delete/Z from TextEdit / rename fields.
        let can_bind = !ctx.wants_keyboard_input() && self.dialog.is_none();
        let (save, new_note, close_tab, next_tab, delete_note, undo, nav_back, nav_fwd) =
            ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::T),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Tab),
                    can_bind && i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                    can_bind && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                    i.modifiers.alt
                        && !i.modifiers.command
                        && i.key_pressed(egui::Key::ArrowLeft),
                    i.modifiers.alt
                        && !i.modifiers.command
                        && i.key_pressed(egui::Key::ArrowRight),
                )
            });
        if save {
            self.save_current();
        }
        if new_note && self.vault.is_some() && self.dialog.is_none() {
            self.create_untitled_tab();
        }
        if close_tab {
            if let Some(path) = self.selected.clone() {
                self.close_tab(&path);
            }
        }
        if next_tab {
            self.cycle_next_tab();
        }
        if delete_note {
            if let Some(path) = self.selected.clone() {
                self.delete_path(&path);
            }
        }
        if undo {
            self.undo();
        }
        if nav_back {
            self.go_back();
        }
        if nav_fwd {
            self.go_forward();
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
                ui.with_layout(
                    egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
                    |ui| {
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
                let can_undo = !self.undo_stack.is_empty();
                let action = tree::show_tree(
                    ui,
                    &self.notes,
                    &self.folders,
                    self.selected.as_deref(),
                    &mut self.renaming,
                    can_undo,
                );
                if let Some(path) = action.select {
                    self.select_note(&path);
                }
                if let Some(path) = action.open_in_new_tab {
                    self.open_note_in_new_tab(&path);
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
                if let Some(path) = action.start_rename {
                    let is_folder = self.folders.iter().any(|f| f == &path);
                    self.renaming = Some(tree::InlineRename {
                        draft: tree::rename_draft(&path, is_folder),
                        from: path,
                        request_focus: true,
                    });
                }
                if action.cancel_rename {
                    self.renaming = None;
                }
                if let Some((from, to)) = action.commit_rename {
                    self.renaming = None;
                    self.rename_path(&from, &to);
                }
                if let Some(path) = action.delete {
                    self.delete_path(&path);
                }
                if action.undo {
                    self.undo();
                }
                if let Some((from, to)) = action.move_note {
                    self.move_note(&from, &to);
                }
                });
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
            let mut tab_select: Option<String> = None;
            let mut tab_close: Option<String> = None;

            ui.horizontal(|ui| {
                ui.add_space(2.0);
                self.show_nav_buttons(ui);
                ui.add_space(4.0);
                if self.open_tabs.is_empty() {
                    ui.label(egui::RichText::new("Note").size(15.0).color(theme::WEAK));
                } else {
                    let paths = self.tab_paths();
                    let tab_action =
                        tabs::show(ui, &paths, self.selected.as_deref(), self.dirty);
                    tab_select = tab_action.select;
                    tab_close = tab_action.close;
                }
                if self.readonly {
                    ui.add_space(8.0);
                    ui.colored_label(theme::WARN, "read-only");
                }
            });
            ui.add_space(4.0);
            if self.open_tabs.is_empty() {
                ui.separator();
            }
            ui.add_space(8.0);

            if let Some(path) = tab_select {
                self.select_note(&path);
            }
            if let Some(path) = tab_close {
                self.close_tab(&path);
            }

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
