//! Shared egui UI for BlueLink (desktop + Android later).

mod editor;
mod editor_menu;
mod find_bar;
mod fold;
mod footnote;
mod local_graph;
mod math_block;
mod media;
mod note_history;
mod outline;
mod preview;
mod properties_panel;
mod quick_switcher;
mod sync_panels;
mod table;
mod tabs;
mod theme;
mod tree;
mod vault_registry;

pub mod sync_ui;
pub use sync_ui::{
    SyncStatus, SyncUiCommand, SyncUiEvent, SyncUiHandle, TrustedDevice, PAIRING_ATTEMPTS,
    PAIRING_TTL_MS,
};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bluelink_core::{
    block_line, create_path_for_link, extract_wikilinks, heading_line, link_fragment,
    rewrite_heading_links, title_from_content, validate_rel_path, NoteMeta, Vault, VaultError,
    WikiIndex,
};
use eframe::egui;
use egui_phosphor::regular;
use vault_registry::{validate_vault_name, VaultRegistry};

enum Dialog {
    NewVault { name: String },
    CreateLink { rel_path: String, new_tab: bool },
    /// Dirty buffer vs newer on-disk content — Reload discards, Keep keeps local.
    ExternalEdit { path: String, disk_sha: String },
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

/// File picker for attaching media (desktop uses `rfd`).
pub type FilePicker = Box<dyn FnMut() -> Option<PathBuf> + Send>;

/// One strip tab: a note (with back/forward history) or the local graph.
enum OpenTab {
    Note {
        path: String,
        history: Vec<String>,
        history_index: usize,
    },
    Graph {
        center_path: String,
    },
}

/// State held only while the quick switcher is visible.
#[derive(Default)]
struct QuickSwitcher {
    query: String,
    results: Vec<quick_switcher::SearchResult>,
    selected: usize,
    request_focus: bool,
}

/// Ctrl+F find / replace strip above the note body.
#[derive(Default)]
struct FindBar {
    open: bool,
    query: String,
    replace: String,
    /// Index into `find_matches` for the open query.
    cursor: usize,
    request_focus: bool,
}

impl OpenTab {
    fn note(path: &str) -> Self {
        Self::Note {
            path: path.to_string(),
            history: vec![path.to_string()],
            history_index: 0,
        }
    }

    fn graph(center_path: &str) -> Self {
        Self::Graph {
            center_path: center_path.to_string(),
        }
    }

    fn note_path(&self) -> Option<&str> {
        match self {
            Self::Note { path, .. } => Some(path.as_str()),
            Self::Graph { .. } => None,
        }
    }

    fn is_graph(&self) -> bool {
        matches!(self, Self::Graph { .. })
    }
}

pub struct App {
    vault: Option<Vault>,
    notes: Vec<NoteMeta>,
    /// Relative folder paths (including empty).
    folders: Vec<String>,
    /// Open tabs (notes + optional local graph).
    open_tabs: Vec<OpenTab>,
    /// Index into `open_tabs` for the focused tab.
    active_tab: Option<usize>,
    selected: Option<String>,
    buffer: String,
    dirty: bool,
    readonly: bool,
    last_edit: Option<Instant>,
    /// Line currently shown as raw Markdown in the live editor.
    active_line: Option<usize>,
    /// Live / Reading / Source — one buffer, different widgets.
    view_mode: editor::ViewMode,
    status: String,
    dialog: Option<Dialog>,
    /// Sidebar inline rename (note or folder).
    renaming: Option<tree::InlineRename>,
    reload_flag: Arc<AtomicBool>,
    pick_folder: Option<FolderPicker>,
    pick_file: Option<FilePicker>,
    /// SHA of selected note as last loaded/saved — used to detect external edits.
    selected_sha: Option<String>,
    /// Ignore notify reloads until this instant (our own saves).
    suppress_reload_until: Option<Instant>,
    /// Disk SHA the user already chose Keep for — skip re-prompt until it changes.
    acked_external_sha: Option<String>,
    registry: VaultRegistry,
    undo_stack: Vec<UndoItem>,
    quick_switcher: Option<QuickSwitcher>,
    /// Recently opened note paths (most recent first), capped for quick switcher.
    recent: Vec<String>,
    local_graph: local_graph::LocalGraphUi,
    /// Desktop sync bridge (Devices / Sync panels — Agent D). `None` without BLE runtime.
    sync_ui: Option<SyncUiHandle>,
    /// Notes / Devices / Sync screen (ТЗ table).
    screen: sync_panels::Screen,
    /// Mirrored sync runtime state (drained from `sync_ui` each frame).
    sync_panel: sync_panels::SyncPanelState,
    /// Per-note typing undo/redo (separate from vault delete undo).
    note_history: note_history::NoteHistory,
    /// Line the live editor should scroll to (consumed via `LiveResult`).
    scroll_to_line: Option<usize>,
    find_bar: FindBar,
    outline_renaming: Option<outline::OutlineRename>,
    /// Group property-panel keystrokes into one NoteHistory checkpoint.
    props_burst: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        vault_path: Option<PathBuf>,
        reload_flag: Arc<AtomicBool>,
        pick_folder: Option<FolderPicker>,
        pick_file: Option<FilePicker>,
        sync_ui: Option<SyncUiHandle>,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let mut registry = VaultRegistry::load();
        registry.prune_missing();

        let mut app = Self {
            vault: None,
            notes: Vec::new(),
            folders: Vec::new(),
            open_tabs: Vec::new(),
            active_tab: None,
            selected: None,
            buffer: String::new(),
            dirty: false,
            readonly: false,
            last_edit: None,
            active_line: None,
            view_mode: editor::ViewMode::Live,
            status: String::new(),
            dialog: None,
            renaming: None,
            reload_flag,
            pick_folder,
            pick_file,
            selected_sha: None,
            suppress_reload_until: None,
            acked_external_sha: None,
            registry,
            undo_stack: Vec::new(),
            quick_switcher: None,
            recent: Vec::new(),
            local_graph: local_graph::LocalGraphUi::default(),
            sync_ui,
            screen: sync_panels::Screen::Notes,
            sync_panel: sync_panels::SyncPanelState::default(),
            note_history: note_history::NoteHistory::new(),
            scroll_to_line: None,
            find_bar: FindBar::default(),
            outline_renaming: None,
            props_burst: false,
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

    pub fn sync_ui(&self) -> Option<&SyncUiHandle> {
        self.sync_ui.as_ref()
    }

    /// Drain runtime → UI events (must stay off BLE/tokio; channel only).
    fn drain_sync_events(&mut self) {
        let Some(handle) = self.sync_ui.as_ref() else {
            return;
        };
        let events = handle.drain_events();
        for event in events {
            self.sync_panel.apply_event(event);
        }
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
                self.active_tab = None;
                self.undo_stack.clear();
                self.note_history.clear();
                self.scroll_to_line = None;
                self.find_bar = FindBar::default();
                self.outline_renaming = None;
                self.props_burst = false;
                self.recent.clear();
                self.selected = None;
                self.buffer.clear();
                self.dirty = false;
                self.readonly = false;
                self.selected_sha = None;
                self.active_line = None;
                self.view_mode = editor::ViewMode::Live;
                self.local_graph = local_graph::LocalGraphUi::default();
                self.refresh_notes();
            }
            Err(e) => {
                self.status = format!("open failed: {e}");
                self.vault = None;
                self.folders.clear();
                self.open_tabs.clear();
                self.active_tab = None;
                self.undo_stack.clear();
                self.note_history.clear();
                self.recent.clear();
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
        self.active_tab = None;
        self.undo_stack.clear();
        self.note_history.clear();
        self.scroll_to_line = None;
        self.find_bar = FindBar::default();
        self.outline_renaming = None;
        self.props_burst = false;
        self.recent.clear();
        self.selected = None;
        self.buffer.clear();
        self.dirty = false;
        self.readonly = false;
        self.selected_sha = None;
        self.active_line = None;
        self.view_mode = editor::ViewMode::Live;
        self.local_graph = local_graph::LocalGraphUi::default();
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

    /// Host file picker → `attachments/` + insert `![[rel]]` into the open note.
    fn pick_attach_file(&mut self) {
        if self.readonly || self.selected.is_none() {
            return;
        }
        if self.pick_file.is_none() {
            self.status = "attach file unavailable".into();
            return;
        }
        let path = self.pick_file.as_mut().and_then(|pick| pick());
        let Some(path) = path else {
            return;
        };
        let note = self.selected.clone().unwrap_or_default();
        let active = self.active_line;
        let outcome = match self.vault.as_ref() {
            Some(vault) => media::ingest_path(vault, &path, &note),
            None => return,
        };
        match outcome {
            Ok(rel) => {
                media::insert_embed(&mut self.buffer, active, &rel);
                self.dirty = true;
                self.last_edit = Some(Instant::now());
                self.status = format!("attached {rel}");
            }
            Err(e) => self.status = format!("attach failed: {e}"),
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
        self.persist_current(true);
    }

    /// Idle autosave — no fsync, no full vault rescan (keeps the UI thread free).
    fn autosave_current(&mut self) {
        self.persist_current(false);
    }

    fn persist_current(&mut self, durable: bool) {
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
        // Always clear idle timer so a failed save cannot retry every frame.
        self.last_edit = None;
        let result = if durable {
            vault.write(&rel, &content)
        } else {
            vault.write_soft(&rel, &content)
        };
        match result {
            Ok(meta) => {
                self.dirty = false;
                self.selected_sha = Some(meta.sha256.clone());
                self.acked_external_sha = None;
                self.status.clear();
                if let Some(n) = self.notes.iter_mut().find(|n| n.rel_path == rel) {
                    *n = meta;
                } else {
                    self.notes.push(meta);
                }
                // Notify will fire for our own write — skip the echo reload briefly.
                self.suppress_reload_until =
                    Some(Instant::now() + Duration::from_millis(750));
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn tab_views(&self) -> Vec<tabs::TabView> {
        self.open_tabs
            .iter()
            .map(|tab| match tab {
                OpenTab::Note { path, .. } => tabs::TabView {
                    title: self.note_display_title(path),
                    kind: tabs::TabKind::Note,
                },
                OpenTab::Graph { center_path } => {
                    let name = self.note_display_title(center_path);
                    tabs::TabView {
                        title: format!("Graph: {name}"),
                        kind: tabs::TabKind::Graph,
                    }
                }
            })
            .collect()
    }

    fn is_graph_active(&self) -> bool {
        self.active_tab
            .and_then(|i| self.open_tabs.get(i))
            .is_some_and(|t| t.is_graph())
    }

    fn graph_center(&self) -> Option<&str> {
        self.active_tab
            .and_then(|i| self.open_tabs.get(i))
            .and_then(|t| match t {
                OpenTab::Graph { center_path } => Some(center_path.as_str()),
                OpenTab::Note { .. } => None,
            })
    }

    fn focus_tab(&mut self, idx: usize) {
        let Some(tab) = self.open_tabs.get(idx) else {
            return;
        };
        match tab {
            OpenTab::Note { path, .. } => {
                let path = path.clone();
                self.active_tab = Some(idx);
                if self.selected.as_deref() != Some(path.as_str()) {
                    let _ = self.load_note_content(&path);
                }
            }
            OpenTab::Graph { .. } => {
                if self.dirty {
                    self.save_current();
                }
                self.active_tab = Some(idx);
            }
        }
    }

    fn clamp_active_tab(&mut self) {
        if self.open_tabs.is_empty() {
            self.active_tab = None;
            return;
        }
        if let Some(i) = self.active_tab {
            if i >= self.open_tabs.len() {
                self.active_tab = Some(self.open_tabs.len() - 1);
            }
        }
    }

    /// Open / focus local graph tab centered on the current note.
    fn toggle_local_graph(&mut self) {
        if let Some(i) = self.open_tabs.iter().position(|t| t.is_graph()) {
            if self.active_tab == Some(i) {
                self.close_tab_at(i);
            } else {
                self.focus_tab(i);
            }
            return;
        }
        let Some(center) = self.selected.clone() else {
            return;
        };
        self.open_tabs.push(OpenTab::graph(&center));
        self.focus_tab(self.open_tabs.len() - 1);
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
        if let Some(i) = self
            .open_tabs
            .iter()
            .position(|t| t.note_path() == Some(rel_path))
        {
            self.focus_tab(i);
            self.push_recent(rel_path);
            if record_history {
                self.record_nav(rel_path);
            }
            return;
        }

        if new_tab {
            self.open_tabs.push(OpenTab::note(rel_path));
            self.active_tab = Some(self.open_tabs.len() - 1);
        } else if let Some(i) = self.active_tab {
            match self.open_tabs.get_mut(i) {
                Some(OpenTab::Note { path, .. }) => {
                    *path = rel_path.to_string();
                }
                Some(OpenTab::Graph { .. }) | None => {
                    // Don't replace a graph tab — open the note beside it.
                    self.open_tabs.push(OpenTab::note(rel_path));
                    self.active_tab = Some(self.open_tabs.len() - 1);
                }
            }
        } else {
            self.open_tabs.push(OpenTab::note(rel_path));
            self.active_tab = Some(self.open_tabs.len() - 1);
        }

        if self.selected.as_deref() == Some(rel_path) && !self.is_graph_active() {
            self.push_recent(rel_path);
            return;
        }
        if !self.load_note_content(rel_path) {
            return;
        }
        self.push_recent(rel_path);
        if record_history {
            self.record_nav(rel_path);
        }
    }

    fn push_recent(&mut self, path: &str) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_string());
        self.recent.truncate(20);
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
                self.view_mode = editor::ViewMode::Live;
                // NoteHistory undo + redo stacks; vault delete undo is separate.
                self.note_history.clear();
                self.scroll_to_line = None;
                self.outline_renaming = None;
                self.props_burst = false;
                self.find_bar.cursor = 0;
                self.selected = Some(rel_path.to_string());
                self.acked_external_sha = None;
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
                self.view_mode = editor::ViewMode::Live;
                self.note_history.clear();
                self.scroll_to_line = None;
                self.outline_renaming = None;
                self.selected = Some(rel_path.to_string());
                self.acked_external_sha = None;
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
        let Some(tab) = self
            .open_tabs
            .iter_mut()
            .find(|t| t.note_path() == Some(path))
        else {
            return;
        };
        let OpenTab::Note {
            history,
            history_index,
            ..
        } = tab
        else {
            return;
        };
        if history.get(*history_index).map(String::as_str) == Some(path) {
            return;
        }
        history.truncate(history_index.saturating_add(1));
        history.push(path.to_string());
        *history_index = history.len() - 1;
    }

    fn can_go_back(&self) -> bool {
        self.active_tab.is_some_and(|i| match &self.open_tabs[i] {
            OpenTab::Note { history_index, .. } => *history_index > 0,
            OpenTab::Graph { .. } => false,
        })
    }

    fn can_go_forward(&self) -> bool {
        self.active_tab.is_some_and(|i| match &self.open_tabs[i] {
            OpenTab::Note {
                history,
                history_index,
                ..
            } => history_index + 1 < history.len(),
            OpenTab::Graph { .. } => false,
        })
    }

    fn go_back(&mut self) {
        let Some(tab_i) = self.active_tab else {
            return;
        };
        loop {
            let OpenTab::Note {
                path,
                history,
                history_index,
            } = &mut self.open_tabs[tab_i]
            else {
                return;
            };
            if *history_index == 0 {
                return;
            }
            *history_index -= 1;
            let next = history[*history_index].clone();
            if self.notes.iter().any(|n| n.rel_path == next) {
                *path = next.clone();
                let _ = self.load_note_content(&next);
                return;
            }
        }
    }

    fn go_forward(&mut self) {
        let Some(tab_i) = self.active_tab else {
            return;
        };
        loop {
            let OpenTab::Note {
                path,
                history,
                history_index,
            } = &mut self.open_tabs[tab_i]
            else {
                return;
            };
            if *history_index + 1 >= history.len() {
                return;
            }
            *history_index += 1;
            let next = history[*history_index].clone();
            if self.notes.iter().any(|n| n.rel_path == next) {
                *path = next.clone();
                let _ = self.load_note_content(&next);
                return;
            }
        }
    }

    fn show_nav_buttons(&mut self, ui: &mut egui::Ui) {
        let back = toolbar_icon(ui, regular::ARROW_LEFT, self.can_go_back(), "Back (Alt+←)");
        let fwd = toolbar_icon(
            ui,
            regular::ARROW_RIGHT,
            self.can_go_forward(),
            "Forward (Alt+→)",
        );
        if back.clicked() {
            self.go_back();
        }
        if fwd.clicked() {
            self.go_forward();
        }
    }

    fn cycle_next_tab(&mut self) {
        if self.open_tabs.len() < 2 {
            return;
        }
        let idx = self.active_tab.unwrap_or(0);
        let next = (idx + 1) % self.open_tabs.len();
        self.focus_tab(next);
    }

    fn close_tab_at(&mut self, idx: usize) {
        if idx >= self.open_tabs.len() {
            return;
        }
        let was_active = self.active_tab == Some(idx);
        let closing_note = self.open_tabs[idx].note_path().map(str::to_string);
        if was_active && closing_note.is_some() && self.dirty {
            self.save_current();
        }
        self.open_tabs.remove(idx);

        if !was_active {
            if let Some(active) = self.active_tab {
                if active > idx {
                    self.active_tab = Some(active - 1);
                }
            }
            return;
        }

        self.active_tab = None;
        if closing_note.is_some() {
            self.selected = None;
            self.buffer.clear();
            self.dirty = false;
            self.readonly = false;
            self.selected_sha = None;
            self.active_line = None;
            self.view_mode = editor::ViewMode::Live;
        }

        if self.open_tabs.is_empty() {
            self.selected = None;
            self.buffer.clear();
            self.dirty = false;
            self.readonly = false;
            self.selected_sha = None;
            self.active_line = None;
            self.view_mode = editor::ViewMode::Live;
            return;
        }

        let next = idx.min(self.open_tabs.len() - 1);
        self.focus_tab(next);
    }

    fn retarget_tab(&mut self, from: &str, to: &str) {
        for tab in &mut self.open_tabs {
            match tab {
                OpenTab::Note {
                    path, history, ..
                } => {
                    if path == from {
                        *path = to.to_string();
                    }
                    for entry in history {
                        if entry == from {
                            *entry = to.to_string();
                        }
                    }
                }
                OpenTab::Graph { center_path } => {
                    if center_path == from {
                        *center_path = to.to_string();
                    }
                }
            }
        }
    }

    fn drop_tabs_with_prefix(&mut self, prefix: &str) {
        let p = format!("{prefix}/");
        let keep_graph = self.is_graph_active();
        let keep_note = self.selected.clone();
        self.open_tabs.retain(|t| match t {
            OpenTab::Note { path, .. } => path != prefix && !path.starts_with(&p),
            OpenTab::Graph { center_path } => {
                center_path != prefix && !center_path.starts_with(&p)
            }
        });
        self.active_tab = if keep_graph {
            self.open_tabs.iter().position(|t| t.is_graph())
        } else {
            keep_note.as_deref().and_then(|sel| {
                self.open_tabs
                    .iter()
                    .position(|t| t.note_path() == Some(sel))
            })
        };
        self.clamp_active_tab();
    }

    fn handle_wikilink(&mut self, raw: &str, new_tab: bool, fragment: Option<String>) {
        let fragment = fragment.or_else(|| link_fragment(raw));
        let Some(vault) = &self.vault else {
            return;
        };
        let source = self.selected.as_deref();
        if let Some(path) = vault.index().resolve(raw, source) {
            let path = path.clone();
            if new_tab {
                self.open_note_in_new_tab(&path);
            } else {
                self.select_note(&path);
            }
            // Scroll even when the note was already open (open_note early-returns).
            if let Some(frag) = fragment {
                self.scroll_to_fragment(&frag);
            }
            return;
        }

        // Bare `[[Note]]` → create next to the current note (Obsidian openLinkText).
        let rel = create_path_for_link(raw, source);
        self.dialog = Some(Dialog::CreateLink {
            rel_path: rel,
            new_tab,
        });
    }

    fn scroll_to_fragment(&mut self, fragment: &str) {
        let line = if fragment.starts_with('^') {
            block_line(&self.buffer, fragment)
        } else {
            heading_line(&self.buffer, fragment)
        };
        let Some(line) = line else {
            self.status = format!("not found: #{fragment}");
            return;
        };
        self.scroll_to_line = Some(line);
        self.active_line = Some(line);
        self.view_mode = editor::ViewMode::Live;
    }

    fn line_of_byte(text: &str, byte: usize) -> usize {
        text.get(..byte)
            .map(|prefix| prefix.bytes().filter(|&b| b == b'\n').count())
            .unwrap_or(0)
    }

    fn find_jump_current(&mut self) {
        let matches = find_bar::find_matches(&self.buffer, &self.find_bar.query);
        if matches.is_empty() {
            self.status = "no matches".into();
            return;
        }
        self.find_bar.cursor %= matches.len();
        let (start, _) = matches[self.find_bar.cursor];
        let line = Self::line_of_byte(&self.buffer, start);
        self.scroll_to_line = Some(line);
        self.active_line = Some(line);
        self.view_mode = editor::ViewMode::Live;
        self.status = format!("match {}/{}", self.find_bar.cursor + 1, matches.len());
    }

    fn find_next(&mut self, forward: bool) {
        let n = find_bar::find_matches(&self.buffer, &self.find_bar.query).len();
        if n == 0 {
            self.status = "no matches".into();
            return;
        }
        if forward {
            self.find_bar.cursor = (self.find_bar.cursor + 1) % n;
        } else {
            self.find_bar.cursor = self.find_bar.cursor.checked_sub(1).unwrap_or(n - 1);
        }
        self.find_jump_current();
    }

    fn find_replace_one(&mut self, ctx: &egui::Context) {
        if self.readonly || self.find_bar.query.is_empty() {
            return;
        }
        let q = self.find_bar.query.clone();
        let repl = self.find_bar.replace.clone();
        let idx = self.find_bar.cursor;
        let Some(&(start, end)) = find_bar::find_matches(&self.buffer, &q).get(idx) else {
            return;
        };
        self.note_history.checkpoint(&self.buffer);
        self.shift_note_folds(ctx, start, end, &repl);
        self.buffer.replace_range(start..end, &repl);
        self.dirty = true;
        self.last_edit = Some(Instant::now());
        self.find_jump_current();
    }

    fn find_replace_all(&mut self, ctx: &egui::Context) {
        if self.readonly || self.find_bar.query.is_empty() {
            return;
        }
        let q = self.find_bar.query.clone();
        let repl = self.find_bar.replace.clone();
        let matches = find_bar::find_matches(&self.buffer, &q);
        let n = matches.len();
        if n == 0 {
            return;
        }
        self.note_history.checkpoint(&self.buffer);
        // End to start, so an earlier match's bytes stay put while later folds move.
        for &(start, end) in matches.iter().rev() {
            self.shift_note_folds(ctx, start, end, &repl);
            self.buffer.replace_range(start..end, &repl);
        }
        self.dirty = true;
        self.last_edit = Some(Instant::now());
        self.find_bar.cursor = 0;
        self.status = format!("replaced {n}");
    }

    fn shift_note_folds(&self, ctx: &egui::Context, start: usize, end: usize, new_text: &str) {
        let Some(path) = self.selected.as_deref() else {
            return;
        };
        let key = egui::Id::new(("bluelink_folds", path));
        let mut folds: Vec<usize> = ctx.data(|d| d.get_temp(key)).unwrap_or_default();
        if folds.is_empty() {
            return;
        }
        editor::shift_folds_in_text(&mut folds, &self.buffer, start, end, new_text);
        ctx.data_mut(|d| d.insert_temp(key, folds));
    }

    /// History restores text, not line indexes, so session folds would point at the wrong rows.
    fn clear_note_folds(&self, ctx: &egui::Context) {
        let Some(path) = self.selected.as_deref() else {
            return;
        };
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new(("bluelink_folds", path)), Vec::<usize>::new());
        });
    }

    fn undo_note_or_file(&mut self, ctx: &egui::Context) {
        if self.note_history.undo(&mut self.buffer) {
            self.clear_note_folds(ctx);
            self.dirty = true;
            self.last_edit = Some(Instant::now());
            self.active_line = None;
            self.view_mode = editor::ViewMode::Live;
            self.status = "undo".into();
            return;
        }
        self.undo();
    }

    fn redo_note(&mut self, ctx: &egui::Context) {
        if self.note_history.redo(&mut self.buffer) {
            self.clear_note_folds(ctx);
            self.dirty = true;
            self.last_edit = Some(Instant::now());
            self.active_line = None;
            self.view_mode = editor::ViewMode::Live;
            self.status = "redo".into();
        }
    }

    fn note_stem(path: &str) -> &str {
        Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
    }

    fn apply_heading_rename(&mut self, line: usize, old: &str, new: &str) {
        if self.readonly || old == new || new.is_empty() {
            return;
        }
        let Some(note_path) = self.selected.clone() else {
            return;
        };
        let stem = Self::note_stem(&note_path).to_string();
        self.note_history.checkpoint(&self.buffer);
        if !outline::rewrite_heading_line(&mut self.buffer, line, new) {
            return;
        }
        self.buffer = rewrite_heading_links(&self.buffer, &stem, old, new);
        self.dirty = true;
        self.last_edit = Some(Instant::now());

        // Other notes in the vault that link to this heading.
        let paths: Vec<String> = self
            .notes
            .iter()
            .map(|n| n.rel_path.clone())
            .filter(|p| p != &note_path)
            .collect();
        for path in paths {
            let updated = {
                let Some(vault) = &self.vault else {
                    return;
                };
                match vault.read(&path) {
                    Ok(content) => {
                        let next = rewrite_heading_links(&content, &stem, old, new);
                        (next != content).then_some(next)
                    }
                    Err(_) => None,
                }
            };
            if let Some(content) = updated {
                if let Some(vault) = self.vault.as_mut() {
                    let _ = vault.write(&path, &content);
                }
            }
        }
        self.status = format!("renamed heading to {new}");
    }

    fn create_note(&mut self, rel_path: &str, initial: &str, new_tab: bool) {
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
                if new_tab {
                    self.open_note_in_new_tab(rel_path);
                } else {
                    self.select_note(rel_path);
                }
                self.status = format!("created {rel_path}");
            }
            Err(e) => self.status = format!("create failed: {e}"),
        }
    }

    /// Next free `untitled.md` / `untitled1.md` / `untitled2.md` …
    fn next_untitled_rel(&self) -> String {
        let stem = next_available_name("untitled", |stem| {
            let path = format!("{stem}.md");
            self.notes.iter().any(|n| n.rel_path == path)
                || self
                    .open_tabs
                    .iter()
                    .any(|t| t.note_path() == Some(path.as_str()))
        });
        format!("{stem}.md")
    }

    fn next_inbox_rel(&self) -> String {
        next_available_name("inbox", |candidate| {
            self.folders.iter().any(|folder| folder == candidate)
        })
    }

    /// Create an untitled note immediately. Ctrl+T opens it in a new tab.
    fn create_untitled_note(&mut self, new_tab: bool) {
        let path = self.next_untitled_rel();
        let title = path.trim_end_matches(".md");
        self.create_note(&path, &format!("# {title}\n"), new_tab);
    }

    fn open_quick_switcher(&mut self) {
        self.open_quick_switcher_with("");
    }

    fn open_quick_switcher_with(&mut self, query: &str) {
        if self.vault.is_some() {
            let query = query.to_string();
            let results = self.quick_switch_results(&query);
            self.quick_switcher = Some(QuickSwitcher {
                query,
                results,
                request_focus: true,
                ..Default::default()
            });
        }
    }

    fn quick_switch_results(&self, query: &str) -> Vec<quick_switcher::SearchResult> {
        let selected = self.selected.as_deref();
        let dirty_buffer = self.dirty.then_some(self.buffer.as_str());
        let Some(vault) = &self.vault else {
            return Vec::new();
        };
        quick_switcher::search(&self.notes, query, &self.recent, |path| {
            if selected == Some(path) {
                if let Some(buffer) = dirty_buffer {
                    return Some(buffer.to_string());
                }
            }
            vault.read(path).ok()
        })
    }

    /// Obsidian openLinkText: open if resolved, else prompt to create beside the active note.
    fn quick_switch_create_from_query(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        let source = self.selected.as_deref();
        if let Some(vault) = &self.vault {
            if let Some(path) = vault.index().resolve(query, source) {
                let path = path.clone();
                self.select_note(&path);
                return;
            }
        }
        let rel = create_path_for_link(query, source);
        self.dialog = Some(Dialog::CreateLink {
            rel_path: rel,
            new_tab: false,
        });
    }

    fn show_quick_switcher(&mut self, ctx: &egui::Context) {
        let Some(mut switcher) = self.quick_switcher.take() else {
            return;
        };

        let mut open: Option<(String, bool)> = None;
        let mut create_query: Option<String> = None;
        let query_id = egui::Id::new("bluelink_quick_switch_query");

        let modal = egui::Modal::new(egui::Id::new("bluelink_quick_switcher"))
            .show(ctx, |ui| {
                ui.set_width(620.0);
                ui.label(
                    egui::RichText::new("Find a note")
                        .size(22.0)
                        .family(theme::display_family())
                        .color(theme::TEXT),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Titles, paths, and note text — all local to this vault")
                        .size(14.0)
                        .color(theme::WEAK),
                );
                ui.add_space(12.0);

                let response = ui.add(
                    egui::TextEdit::singleline(&mut switcher.query)
                        .id(query_id)
                        .hint_text("Search notes…")
                        .desired_width(f32::INFINITY),
                );
                if switcher.request_focus {
                    response.request_focus();
                    switcher.request_focus = false;
                }
                if response.changed() {
                    switcher.selected = 0;
                    switcher.results = self.quick_switch_results(&switcher.query);
                }

                let results = &switcher.results;
                switcher.selected = switcher.selected.min(results.len().saturating_sub(1));

                let (up, down, open_new, open_current, create) = ctx.input_mut(|input| {
                    let open_new = input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
                    let create = input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter);
                    (
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        open_new,
                        !open_new
                            && !create
                            && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        create,
                    )
                });
                if up && !results.is_empty() {
                    switcher.selected = switcher
                        .selected
                        .checked_sub(1)
                        .unwrap_or(results.len() - 1);
                }
                if down && !results.is_empty() {
                    switcher.selected = (switcher.selected + 1) % results.len();
                }
                if create && !switcher.query.trim().is_empty() {
                    create_query = Some(switcher.query.clone());
                } else if (open_current || open_new) && !results.is_empty() {
                    open = Some((results[switcher.selected].path.clone(), open_new));
                }

                ui.add_space(10.0);
                let empty_query = switcher.query.trim().is_empty();
                let count = if empty_query && !self.recent.is_empty() && !results.is_empty() {
                    format!("{} recent", results.len())
                } else if results.len() == 1 {
                    "1 note".into()
                } else {
                    format!("{} notes", results.len())
                };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(count).size(13.0).color(theme::WEAK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Enter open  ·  Ctrl+Enter new tab  ·  Shift+Enter create  ·  Esc",
                            )
                            .size(13.0)
                            .color(theme::WEAK),
                        );
                    });
                });
                ui.add_space(6.0);
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if results.is_empty() {
                            let q = switcher.query.trim();
                            if q.is_empty() {
                                ui.add_space(18.0);
                                ui.centered_and_justified(|ui| {
                                    ui.label(
                                        egui::RichText::new("No notes yet").color(theme::WEAK),
                                    );
                                });
                                ui.add_space(18.0);
                            } else {
                                ui.add_space(8.0);
                                let label = format!("Create \"{q}\"");
                                let row = egui::Frame::NONE
                                    .fill(theme::FAINT)
                                    .inner_margin(egui::Margin::symmetric(10, 10))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.label(
                                            egui::RichText::new(label)
                                                .size(17.0)
                                                .color(theme::TEXT),
                                        );
                                        ui.label(
                                            egui::RichText::new("Shift+Enter")
                                                .size(13.0)
                                                .color(theme::WEAK),
                                        );
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if row.clicked() {
                                    create_query = Some(switcher.query.clone());
                                }
                            }
                        }
                        for (index, result) in results.iter().enumerate() {
                            let selected = index == switcher.selected;
                            let fill = if selected {
                                theme::FAINT
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let row = egui::Frame::NONE
                                .fill(fill)
                                .inner_margin(egui::Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&result.title)
                                                .size(17.0)
                                                .color(theme::TEXT),
                                        );
                                        ui.label(
                                            egui::RichText::new(&result.path)
                                                .size(13.0)
                                                .color(theme::ACCENT),
                                        );
                                        if let Some(snippet) = &result.snippet {
                                            ui.add_space(2.0);
                                            ui.label(
                                                egui::RichText::new(snippet)
                                                    .size(14.0)
                                                    .color(theme::WEAK),
                                            );
                                        }
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if row.hovered() {
                                switcher.selected = index;
                            }
                            if row.clicked() {
                                open = Some((result.path.clone(), false));
                            }
                        }
                    });
            });

        if let Some(query) = create_query {
            self.quick_switch_create_from_query(&query);
            return;
        }
        if let Some((path, new_tab)) = open {
            if new_tab {
                self.open_note_in_new_tab(&path);
            } else {
                self.select_note(&path);
            }
            return;
        }
        if !modal.should_close() {
            self.quick_switcher = Some(switcher);
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
                        match tab {
                            OpenTab::Note {
                                path, history, ..
                            } => {
                                if let Some(rest) = path.strip_prefix(&prefix) {
                                    *path = format!("{new_prefix}{rest}");
                                }
                                for entry in history {
                                    if let Some(rest) = entry.strip_prefix(&prefix) {
                                        *entry = format!("{new_prefix}{rest}");
                                    }
                                }
                            }
                            OpenTab::Graph { center_path } => {
                                if let Some(rest) = center_path.strip_prefix(&prefix) {
                                    *center_path = format!("{new_prefix}{rest}");
                                }
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
                        self.view_mode = editor::ViewMode::Live;
                        self.active_tab = None;
                        if !self.open_tabs.is_empty() {
                            self.focus_tab(0);
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
                let note_idx = self
                    .open_tabs
                    .iter()
                    .position(|t| t.note_path() == Some(rel));
                let was_active = self.active_tab == note_idx
                    || self
                        .active_tab
                        .and_then(|i| self.open_tabs.get(i))
                        .is_some_and(|t| matches!(t, OpenTab::Graph { center_path } if center_path == rel));
                // Drop the note tab; drop graph tab if it was centered on this note.
                self.open_tabs.retain(|t| match t {
                    OpenTab::Note { path, .. } => path != rel,
                    OpenTab::Graph { center_path } => center_path != rel,
                });
                if was_active || self.selected.as_deref() == Some(rel) {
                    self.selected = None;
                    self.buffer.clear();
                    self.dirty = false;
                    self.readonly = false;
                    self.selected_sha = None;
                    self.active_line = None;
                    self.view_mode = editor::ViewMode::Live;
                    self.active_tab = None;
                    if let Some(i) = note_idx {
                        if i < self.open_tabs.len() {
                            self.focus_tab(i);
                        } else if !self.open_tabs.is_empty() {
                            self.focus_tab(self.open_tabs.len() - 1);
                        }
                    } else if !self.open_tabs.is_empty() {
                        self.focus_tab(0);
                    }
                } else {
                    self.clamp_active_tab();
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
        if self
            .suppress_reload_until
            .is_some_and(|t| Instant::now() < t)
        {
            return;
        }
        // Always refresh the tree; buffer reload depends on dirty state.
        self.refresh_notes();
        let Some(rel) = self.selected.clone() else {
            return;
        };
        let new_sha = self
            .notes
            .iter()
            .find(|n| n.rel_path == rel)
            .map(|n| n.sha256.clone());

        if self.dirty {
            match new_sha {
                None => {
                    self.status = "note removed on disk (unsaved edits kept)".into();
                }
                Some(disk_sha)
                    if should_prompt_external_conflict(
                        self.selected_sha.as_deref(),
                        Some(disk_sha.as_str()),
                        self.acked_external_sha.as_deref(),
                    ) =>
                {
                    // Don't clobber an unrelated dialog; surface a status hint instead.
                    let can_prompt = matches!(
                        &self.dialog,
                        None | Some(Dialog::ExternalEdit { .. })
                    );
                    if can_prompt {
                        self.dialog = Some(Dialog::ExternalEdit {
                            path: rel,
                            disk_sha,
                        });
                    } else {
                        self.status = "note changed on disk (unsaved edits kept)".into();
                    }
                }
                Some(_) => {}
            }
            return;
        }

        if new_sha.is_none() {
            // File vanished externally.
            self.selected = None;
            self.buffer.clear();
            self.selected_sha = None;
            self.acked_external_sha = None;
            self.status = "note removed on disk".into();
            return;
        }
        if new_sha == self.selected_sha {
            return;
        }
        self.reload_selected_buffer(rel, new_sha);
    }

    /// Discard buffer and load `rel` from disk (clean external reload or Reload choice).
    fn reload_selected_buffer(&mut self, rel: String, new_sha: Option<String>) {
        let Some(vault) = &self.vault else {
            return;
        };
        match vault.read(&rel) {
            Ok(text) => {
                self.buffer = text;
                self.readonly = false;
                self.dirty = false;
                self.last_edit = None;
                self.selected_sha = new_sha;
                self.acked_external_sha = None;
                self.active_line = None;
                self.view_mode = editor::ViewMode::Live;
                self.note_history.clear();
                self.scroll_to_line = None;
                self.outline_renaming = None;
                self.status = format!("reloaded from disk {}", hhmm_now());
            }
            Err(VaultError::NotUtf8) => {
                self.buffer.clear();
                self.readonly = true;
                self.dirty = false;
                self.last_edit = None;
                self.selected_sha = new_sha;
                self.acked_external_sha = None;
                self.active_line = None;
                self.view_mode = editor::ViewMode::Live;
                self.note_history.clear();
                self.scroll_to_line = None;
                self.status = "note is not UTF-8 (read-only)".into();
            }
            Err(e) => self.status = format!("reload failed: {e}"),
        }
    }

    fn show_find_bar(&mut self, ui: &mut egui::Ui) {
        let mut close = false;
        let mut go_next = false;
        let mut go_prev = false;
        let mut do_replace = false;
        let mut do_replace_all = false;
        let mut query_changed = false;

        egui::Frame::NONE
            .fill(theme::PANEL)
            .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
            .corner_radius(theme::round_sm())
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Find")
                            .size(13.0)
                            .color(theme::WEAK),
                    );
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.find_bar.query)
                            .desired_width(160.0)
                            .hint_text("Find…"),
                    );
                    if self.find_bar.request_focus {
                        resp.request_focus();
                        self.find_bar.request_focus = false;
                    }
                    if resp.changed() {
                        query_changed = true;
                    }
                    if ui
                        .small_button(regular::ARROW_DOWN.to_string())
                        .on_hover_text("Next")
                        .clicked()
                    {
                        go_next = true;
                    }
                    if ui
                        .small_button(regular::ARROW_UP.to_string())
                        .on_hover_text("Previous")
                        .clicked()
                    {
                        go_prev = true;
                    }
                    ui.label(
                        egui::RichText::new("Replace")
                            .size(13.0)
                            .color(theme::WEAK),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.find_bar.replace)
                            .desired_width(120.0)
                            .hint_text("Replace…"),
                    );
                    if ui.small_button("Replace").clicked() {
                        do_replace = true;
                    }
                    if ui.small_button("All").on_hover_text("Replace all").clicked() {
                        do_replace_all = true;
                    }
                    if ui.small_button(regular::X.to_string()).on_hover_text("Close").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        close = true;
                    }
                });
            });

        if close {
            self.find_bar.open = false;
            return;
        }
        if query_changed {
            self.find_bar.cursor = 0;
            self.find_jump_current();
        }
        if go_next {
            self.find_next(true);
        }
        if go_prev {
            self.find_next(false);
        }
        if do_replace {
            self.find_replace_one(ui.ctx());
        }
        if do_replace_all {
            self.find_replace_all(ui.ctx());
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
                        .family(theme::display_family())
                        .color(theme::TEXT),
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
        let mut confirm_create: Option<(String, bool)> = None;
        let mut confirm_vault: Option<String> = None;
        let mut confirm_reload: Option<(String, String)> = None;
        let mut confirm_keep: Option<String> = None;

        match &mut self.dialog {
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
            Some(Dialog::CreateLink { rel_path, new_tab }) => {
                let path = rel_path.clone();
                let new_tab = *new_tab;
                egui::Window::new("Create note")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(format!("Note `{path}` does not exist. Create it?"));
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() {
                                confirm_create = Some((path.clone(), new_tab));
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Dialog::ExternalEdit { path, disk_sha }) => {
                let path = path.clone();
                let disk_sha = disk_sha.clone();
                egui::Window::new("Note changed on disk")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "`{path}` was edited outside BlueLink. Reload discards your unsaved edits; Keep keeps them."
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Reload").clicked() {
                                confirm_reload = Some((path.clone(), disk_sha.clone()));
                                close = true;
                            }
                            if ui.button("Keep").clicked() {
                                confirm_keep = Some(disk_sha.clone());
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
        if let Some((path, new_tab)) = confirm_create {
            let title = path
                .trim_end_matches(".md")
                .rsplit('/')
                .next()
                .unwrap_or("Note");
            self.create_note(&path, &format!("# {title}\n"), new_tab);
        }
        if let Some(name) = confirm_vault {
            self.create_named_vault(&name);
        }
        if let Some((path, disk_sha)) = confirm_reload {
            // Only reload if this note is still selected (user may have switched tabs).
            if self.selected.as_deref() == Some(path.as_str()) {
                self.reload_selected_buffer(path, Some(disk_sha));
            }
        }
        if let Some(disk_sha) = confirm_keep {
            self.acked_external_sha = Some(disk_sha);
            self.status = format!("kept local edits {}", hhmm_now());
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // eframe default clear is alpha 180 — seams between panels show the desktop.
        theme::BG.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_sync_events();
        self.apply_external_reload();

        if self.vault.is_none() {
            self.show_no_vault(ctx);
            return;
        }

        // Autosave after 1s idle (soft write — must not block the UI thread).
        if self.dirty {
            if let Some(t) = self.last_edit {
                if t.elapsed() >= Duration::from_secs(1) {
                    self.autosave_current();
                } else {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            }
        }

        // Ctrl+P is intentionally available from the editor. Other bindings must not
        // steal Delete/Z from TextEdit / rename fields or from the quick switcher.
        let can_bind = !ctx.wants_keyboard_input()
            && self.dialog.is_none()
            && self.quick_switcher.is_none();
        let can_open_switcher = self.dialog.is_none() && self.quick_switcher.is_none();
        let has_graph_tab = self.open_tabs.iter().any(|t| t.is_graph());
        let can_toggle_graph = self.vault.is_some()
            && (self.selected.is_some() || has_graph_tab)
            && self.dialog.is_none()
            && self.quick_switcher.is_none();
        let (
            save,
            new_note,
            close_tab,
            next_tab,
            quick_switch,
            toggle_graph,
            delete_note,
            undo,
            redo,
            find,
            nav_back,
            nav_fwd,
        ) = ctx.input_mut(|i| {
            let redo = can_bind
                && i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                );
            let undo = can_bind
                && !redo
                && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::T),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Tab),
                can_open_switcher && i.consume_key(egui::Modifiers::COMMAND, egui::Key::P),
                can_toggle_graph && i.consume_key(egui::Modifiers::COMMAND, egui::Key::G),
                can_bind && i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                undo,
                redo,
                can_open_switcher && i.consume_key(egui::Modifiers::COMMAND, egui::Key::F),
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
            self.create_untitled_note(true);
        }
        if close_tab {
            if let Some(i) = self.active_tab {
                self.close_tab_at(i);
            }
        }
        if next_tab {
            self.cycle_next_tab();
        }
        let on_notes = self.screen == sync_panels::Screen::Notes;
        if quick_switch && on_notes {
            self.open_quick_switcher();
        }
        if toggle_graph && on_notes {
            self.toggle_local_graph();
        }
        if delete_note && on_notes && !self.is_graph_active() {
            if let Some(path) = self.selected.clone() {
                self.delete_path(&path);
            }
        }
        if undo && on_notes {
            self.undo_note_or_file(ctx);
        }
        if redo && on_notes {
            self.redo_note(ctx);
        }
        if find && on_notes && !self.is_graph_active() && self.selected.is_some() {
            self.find_bar.open = true;
            self.find_bar.request_focus = true;
        }
        if nav_back && on_notes {
            self.go_back();
        }
        if nav_fwd && on_notes {
            self.go_forward();
        }

        self.show_dialogs(ctx);

        egui::TopBottomPanel::bottom("status")
            .exact_height(36.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show_separator_line(true)
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

        // Id "tree_sidebar" keeps PanelState clear of the old width-grow "tree" id.
        egui::SidePanel::left("tree_sidebar")
            .default_width(theme::SIDEBAR_WIDTH)
            .min_width(240.0)
            .max_width(380.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 0,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show_separator_line(true)
            .show(ctx, |ui| {
                // Sidebar-local spacing — global item_spacing is too airy for a nav rail.
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);

                // Header keeps a right pad; tree scroll sits flush against the scrollbar.
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 12,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mark = egui::vec2(10.0, 10.0);
                            let (rect, _) = ui.allocate_exact_size(mark, egui::Sense::hover());
                            ui.painter()
                                .rect_filled(rect, theme::round_sm(), theme::ACCENT);
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("BlueLink")
                                    .size(18.0)
                                    .family(theme::display_family())
                                    .strong()
                                    .color(theme::TEXT),
                            );
                        });
                        ui.add_space(10.0);

                        let vault_name = self
                            .vault_root()
                            .map(VaultRegistry::display_name)
                            .unwrap_or_else(|| "Vault".into());
                        let vault_w = ui.available_width();
                        egui::ComboBox::from_id_salt("vault_switcher")
                            .width(vault_w)
                            .selected_text(format!("{}  {vault_name}", regular::FOLDER))
                            .show_ui(ui, |ui| {
                                for entry in &self.registry.entries {
                                    let name = VaultRegistry::display_name(&entry.path);
                                    let selected = self.vault_root().is_some_and(|r| {
                                        r == entry.path.as_path()
                                            || r.canonicalize().ok()
                                                == entry.path.canonicalize().ok()
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
                                if ui.button(format!("{}  Create…", regular::PLUS)).clicked()
                                {
                                    create_vault = true;
                                }
                                if ui
                                    .button(format!("{}  Forget current", regular::X))
                                    .clicked()
                                {
                                    forget_active = true;
                                }
                            });

                        ui.add_space(10.0);
                        sync_panels::show_screen_nav(ui, &mut self.screen);
                    });

                ui.add_space(12.0);

                if self.screen == sync_panels::Screen::Notes {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: 0,
                            right: 12,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("NOTES")
                                    .size(11.0)
                                    .family(theme::display_family())
                                    .color(theme::WEAK),
                            );
                        });
                    ui.add_space(6.0);
                    self.sync_live_title();
                    let can_undo = !self.undo_stack.is_empty();
                    let tree_sel = if self.is_graph_active() {
                        self.graph_center().map(str::to_string)
                    } else {
                        self.selected.clone()
                    };
                    let action = tree::show_tree(
                        ui,
                        &self.notes,
                        &self.folders,
                        tree_sel.as_deref(),
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
                        self.create_untitled_note(false);
                    }
                    if action.new_folder {
                        let path = self.next_inbox_rel();
                        self.create_folder(&path);
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
                        self.undo_note_or_file(ctx);
                    }
                    if let Some((from, to)) = action.move_note {
                        self.move_note(&from, &to);
                    }
                } else {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: 0,
                            right: 12,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            let hint = match self.screen {
                                sync_panels::Screen::Devices => "Trusted devices and pairing",
                                sync_panels::Screen::Sync => "Status and sync log",
                                sync_panels::Screen::Notes => "",
                            };
                            ui.label(
                                egui::RichText::new(hint).size(13.0).color(theme::WEAK),
                            );
                        });
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

        let notes_screen = self.screen == sync_panels::Screen::Notes;
        let mut open_backlink: Option<String> = None;
        let mut outline_jump: Option<usize> = None;
        let mut outline_rename: Option<(usize, String, String)> = None;
        let backlink_target = if notes_screen {
            if self.is_graph_active() {
                self.graph_center().map(str::to_string)
            } else {
                self.selected.clone()
            }
        } else {
            None
        };
        let outline_entries = if notes_screen && !self.is_graph_active() && self.selected.is_some()
        {
            outline::headings(&self.buffer)
        } else {
            Vec::new()
        };
        let backlinks: Vec<(String, Option<String>)> =
            match (backlink_target.as_deref(), self.vault.as_ref()) {
                (Some(sel), Some(vault)) => {
                    let paths = vault.index().backlinks(sel);
                    paths
                        .into_iter()
                        .map(|path| {
                            let snippet = vault
                                .read(&path)
                                .ok()
                                .and_then(|text| backlink_snippet(&text, &path, sel, vault.index()));
                            (path, snippet)
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
        let show_right_rail =
            notes_screen && (!outline_entries.is_empty() || !backlinks.is_empty());
        // show_animated: hide when empty without remounting the panel every note switch
        // (abrupt SidePanel mount/unmount has frozen the Wayland event loop before).
        egui::SidePanel::right("backlinks")
            .default_width(240.0)
            .min_width(180.0)
            .show_animated(ctx, show_right_rail, |ui| {
                ui.add_space(6.0);

                let outline_action =
                    outline::show(ui, &outline_entries, &mut self.outline_renaming);
                if let Some(line) = outline_action.jump {
                    outline_jump = Some(line);
                }
                if let Some((line, title)) = outline_action.start_rename {
                    self.outline_renaming = Some(outline::OutlineRename {
                        line,
                        old: title.clone(),
                        draft: title,
                        request_focus: true,
                    });
                }
                if outline_action.cancel_rename {
                    self.outline_renaming = None;
                }
                if let Some(commit) = outline_action.commit_rename {
                    self.outline_renaming = None;
                    outline_rename = Some(commit);
                }

                if !backlinks.is_empty() {
                    ui.label(
                        egui::RichText::new("Backlinks")
                            .size(16.0)
                            .family(theme::display_family())
                            .color(theme::TEXT),
                    );
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (path, snippet) in &backlinks {
                            let name = self.note_display_title(path);
                            let row = egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(4, 6))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  {name}",
                                                regular::LINK
                                            ))
                                            .size(14.0)
                                            .color(theme::TEXT),
                                        );
                                        if let Some(snippet) = snippet {
                                            ui.label(
                                                egui::RichText::new(snippet)
                                                    .size(12.0)
                                                    .color(theme::WEAK),
                                            );
                                        }
                                    });
                                })
                                .response
                                .interact(egui::Sense::click())
                                .on_hover_text(path);
                            if row.clicked() {
                                open_backlink = Some(path.clone());
                            }
                        }
                    });
            });
        if let Some(path) = open_backlink {
            self.select_note(&path);
        }
        if let Some(line) = outline_jump {
            self.scroll_to_line = Some(line);
            self.active_line = Some(line);
            self.view_mode = editor::ViewMode::Live;
        }
        if let Some((line, old, new)) = outline_rename {
            self.apply_heading_rename(line, &old, &new);
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ctx, |ui| {
            if self.screen != sync_panels::Screen::Sync {
                self.sync_panel.reset_ensure_ble();
            }
            match self.screen {
                sync_panels::Screen::Devices => {
                    ui.add_space(8.0);
                    let handle = self.sync_ui.as_ref();
                    sync_panels::show_devices(
                        ui,
                        ctx,
                        &mut self.sync_panel,
                        handle,
                        hhmm_from_unix_ms,
                    );
                    return;
                }
                sync_panels::Screen::Sync => {
                    ui.add_space(8.0);
                    let handle = self.sync_ui.as_ref();
                    sync_panels::show_sync(ui, &mut self.sync_panel, handle, hhmm_from_unix_ms);
                    return;
                }
                sync_panels::Screen::Notes => {}
            }

            let mut tab_select: Option<usize> = None;
            let mut tab_close: Option<usize> = None;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.set_min_height(tabs::BAR_HEIGHT);
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        self.show_nav_buttons(ui);
                        if toolbar_icon(
                            ui,
                            regular::MAGNIFYING_GLASS,
                            true,
                            "Find a note (Ctrl+P)",
                        )
                        .clicked()
                        {
                            self.open_quick_switcher();
                        }
                        {
                            let has_graph = self.open_tabs.iter().any(|t| t.is_graph());
                            let graph_enabled =
                                self.vault.is_some() && (self.selected.is_some() || has_graph);
                            let tip = if graph_enabled {
                                "Local graph (Ctrl+G)"
                            } else {
                                "Open a note to show its local graph"
                            };
                            if toolbar_icon(ui, regular::SHARE_NETWORK, graph_enabled, tip)
                                .clicked()
                            {
                                self.toggle_local_graph();
                            }
                        }
                        ui.add_space(6.0);
                        if self.open_tabs.is_empty() {
                            ui.label(
                                egui::RichText::new("Note").size(15.0).color(theme::WEAK),
                            );
                        } else {
                            let views = self.tab_views();
                            let tab_action = tabs::show(
                                ui,
                                &views,
                                self.active_tab,
                                self.dirty && !self.is_graph_active(),
                            );
                            tab_select = tab_action.select;
                            tab_close = tab_action.close;
                        }
                        if !self.is_graph_active() && self.selected.is_some() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(self.view_mode.label())
                                    .size(12.0)
                                    .color(theme::WEAK),
                            );
                        }
                        if self.readonly && !self.is_graph_active() {
                            ui.add_space(8.0);
                            ui.colored_label(theme::WARN, "read-only");
                        }
                    },
                );
            });
            ui.add_space(4.0);
            if self.open_tabs.is_empty() {
                ui.separator();
            }
            ui.add_space(8.0);

            if let Some(i) = tab_select {
                self.focus_tab(i);
            }
            if let Some(i) = tab_close {
                self.close_tab_at(i);
            }

            if self.is_graph_active() {
                self.show_local_graph_panel(ui);
                return;
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

            if self.find_bar.open {
                self.show_find_bar(ui);
                ui.add_space(6.0);
            }

            {
                let props_locked =
                    self.readonly || self.view_mode == editor::ViewMode::Reading;
                let props = properties_panel::show(ui, &self.buffer, props_locked);
                if let Some(next) = props.rewritten {
                    if next != self.buffer {
                        if !self.props_burst {
                            self.note_history.checkpoint(&self.buffer);
                            self.props_burst = true;
                        }
                        self.buffer = next;
                        self.dirty = true;
                        self.last_edit = Some(Instant::now());
                    }
                }
                if !props.focused {
                    self.props_burst = false;
                }
                ui.add_space(8.0);
            }

            // Take vault temporarily so we can mutate buffer alongside an immutable vault borrow.
            let vault = self.vault.take();
            let mut wiki_click = None;
            let mut want_attach = false;
            let mut tag_query = None;
            let scroll_to = self.scroll_to_line;
            if let Some(ref v) = vault {
                let live = editor::show_live(
                    ui,
                    v,
                    self.selected.as_deref(),
                    &mut self.buffer,
                    &mut self.active_line,
                    &mut self.view_mode,
                    self.readonly,
                    scroll_to,
                    &mut self.note_history,
                );
                if live.changed {
                    self.dirty = true;
                    self.last_edit = Some(Instant::now());
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
                if live.scroll_consumed {
                    self.scroll_to_line = None;
                }
                if let Some(msg) = live.status {
                    self.status = msg;
                }
                want_attach = live.attach_file;
                wiki_click = live.wikilink;
                tag_query = live.tag_query;
            }
            self.vault = vault;
            if want_attach {
                self.pick_attach_file();
            }
            if let Some(click) = wiki_click {
                self.handle_wikilink(&click.raw, click.new_tab, click.fragment);
            }
            if let Some(query) = tag_query {
                self.open_quick_switcher_with(&query);
            }
        });

        if notes_screen {
            self.show_quick_switcher(ctx);
        }
    }
}

impl App {
    fn show_local_graph_panel(&mut self, ui: &mut egui::Ui) {
        let Some(center) = self.graph_center().map(str::to_string) else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No note to center the graph on")
                        .size(20.0)
                        .color(theme::WEAK),
                );
            });
            return;
        };
        if center.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No note to center the graph on")
                        .size(20.0)
                        .color(theme::WEAK),
                );
            });
            return;
        }

        let graph = match &self.vault {
            Some(vault) => vault.index().local_graph(&center, local_graph::MAX_NODES),
            None => return,
        };
        let titles: std::collections::HashMap<String, String> = graph
            .nodes
            .iter()
            .map(|path| (path.clone(), self.note_display_title(path)))
            .collect();

        let clicked = local_graph::show_panel(ui, &mut self.local_graph, &graph, |path| {
            titles
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.to_string())
        });
        if let Some(path) = clicked {
            if let Some(i) = self.active_tab {
                if let Some(OpenTab::Graph { center_path }) = self.open_tabs.get_mut(i) {
                    *center_path = path;
                }
            }
        }
    }

    /// Keep the open note's sidebar/tab title in sync with the buffer's first line.
    fn sync_live_title(&mut self) {
        if self.is_graph_active() {
            return;
        }
        let Some(sel) = self.selected.clone() else {
            return;
        };
        let title = title_from_content(&self.buffer, &sel);
        if let Some(n) = self.notes.iter_mut().find(|n| n.rel_path == sel) {
            n.title = title;
        }
    }

    fn note_display_title(&self, path: &str) -> String {
        if self.selected.as_deref() == Some(path) {
            return title_from_content(&self.buffer, path);
        }
        self.notes
            .iter()
            .find(|n| n.rel_path == path)
            .map(|n| n.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| title_from_content("", path))
    }

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

/// Ghost icon button for the note chrome toolbar — equal hit target, muted ink.
fn toolbar_icon(
    ui: &mut egui::Ui,
    icon: &str,
    enabled: bool,
    tip: &str,
) -> egui::Response {
    let color = if enabled { theme::WEAK } else { theme::STROKE };
    let btn = egui::Button::new(egui::RichText::new(icon).size(15.0).color(color))
        .min_size(egui::vec2(32.0, 32.0))
        .corner_radius(theme::round_sm())
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE);
    let response = ui.add_enabled(enabled, btn);
    if enabled {
        response.on_hover_text(tip)
    } else {
        response.on_disabled_hover_text(tip)
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

fn next_available_name(base: &str, mut is_taken: impl FnMut(&str) -> bool) -> String {
    for index in 0_u64.. {
        let candidate = if index == 0 {
            base.to_string()
        } else {
            format!("{base}{index}")
        };
        if !is_taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("u64 name suffixes are exhausted")
}

/// First line in `source` whose wikilink resolves to `target` (cheap backlink snippet).
fn backlink_snippet(
    content: &str,
    source: &str,
    target: &str,
    index: &WikiIndex,
) -> Option<String> {
    for line in content.lines() {
        for link in extract_wikilinks(line) {
            if index.resolve(&link, Some(source)).as_deref() == Some(target) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    return Some(quick_switcher::ellipsize(trimmed, 96));
                }
            }
        }
    }
    None
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

/// Dirty buffer vs on-disk SHA: prompt when disk differs from last load/save and Keep ack.
fn should_prompt_external_conflict(
    selected_sha: Option<&str>,
    disk_sha: Option<&str>,
    acked_external_sha: Option<&str>,
) -> bool {
    let Some(disk) = disk_sha else {
        return false;
    };
    if Some(disk) == selected_sha {
        return false;
    }
    if Some(disk) == acked_external_sha {
        return false;
    }
    true
}

#[cfg(test)]
mod automatic_name_tests {
    use super::next_available_name;

    #[test]
    fn starts_with_base_and_uses_the_next_available_number() {
        assert_eq!(next_available_name("untitled", |_| false), "untitled");
        assert_eq!(
            next_available_name("inbox", |candidate| candidate == "inbox" || candidate == "inbox1"),
            "inbox2"
        );
        assert_eq!(
            next_available_name("folder", |candidate| {
                candidate == "folder"
                    || candidate
                        .strip_prefix("folder")
                        .and_then(|suffix| suffix.parse::<u32>().ok())
                        .is_some_and(|index| index < 10_000)
            }),
            "folder10000"
        );
    }
}

#[cfg(test)]
mod external_conflict_tests {
    use super::should_prompt_external_conflict;

    #[test]
    fn no_prompt_when_disk_matches_baseline() {
        assert!(!should_prompt_external_conflict(
            Some("aaa"),
            Some("aaa"),
            None
        ));
    }

    #[test]
    fn prompt_when_disk_differs() {
        assert!(should_prompt_external_conflict(
            Some("aaa"),
            Some("bbb"),
            None
        ));
    }

    #[test]
    fn no_prompt_after_keep_ack() {
        assert!(!should_prompt_external_conflict(
            Some("aaa"),
            Some("bbb"),
            Some("bbb")
        ));
    }

    #[test]
    fn prompt_again_when_disk_changes_after_keep() {
        assert!(should_prompt_external_conflict(
            Some("aaa"),
            Some("ccc"),
            Some("bbb")
        ));
    }

    #[test]
    fn no_prompt_when_file_vanished() {
        assert!(!should_prompt_external_conflict(Some("aaa"), None, None));
    }
}
