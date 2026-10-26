use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::error::VaultError;
use crate::meta::{self, DeviceMeta};
use crate::path::{self, MAX_NOTE_BYTES};
use crate::wiki::{
    notes_after_renames, rewrite_wikilinks_in_content, WikiIndex,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMeta {
    /// Always uses `/` as separator, e.g. `inbox/todo.md`.
    pub rel_path: String,
    /// Display title = first non-empty line (ATX `#` markers stripped).
    pub title: String,
    /// Hex SHA-256 of file contents.
    pub sha256: String,
    pub size: u64,
    /// Unix milliseconds — UI only, not used by sync logic.
    pub modified: u64,
}

/// First non-empty line of a note, with leading `#` heading markers stripped.
/// Falls back to the `.md` stem of `rel_path` when the body has no usable line.
pub fn title_from_content(content: &str, rel_path: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let body = trimmed.trim_start_matches('#').trim();
        if !body.is_empty() {
            return body.to_string();
        }
    }
    Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string()
}

pub struct Vault {
    root: PathBuf,
    device: DeviceMeta,
    index: WikiIndex,
}

impl Vault {
    pub fn open(root: &Path) -> Result<Self, VaultError> {
        let meta = fs::metadata(root).map_err(VaultError::from_io)?;
        if !meta.is_dir() {
            return Err(VaultError::NotADirectory);
        }

        let root = root.canonicalize().map_err(VaultError::from_io)?;
        let device = meta::load_or_create(&root)?;

        let mut vault = Self {
            root,
            device,
            index: WikiIndex::new(),
        };
        vault.rebuild_index()?;
        Ok(vault)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn device(&self) -> &DeviceMeta {
        &self.device
    }

    pub fn index(&self) -> &WikiIndex {
        &self.index
    }

    pub fn list(&self) -> Result<Vec<NoteMeta>, VaultError> {
        let mut notes = Vec::new();
        self.walk_notes(&self.root, "", &mut notes)?;
        notes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(notes)
    }

    /// Relative folder paths under the vault (including empty ones).
    pub fn list_folders(&self) -> Result<Vec<String>, VaultError> {
        let mut folders = Vec::new();
        self.walk_folders(&self.root, "", &mut folders)?;
        folders.sort();
        folders.dedup();
        Ok(folders)
    }

    /// Create a folder (and parents) inside the vault.
    pub fn create_folder(&self, rel_path: &str) -> Result<(), VaultError> {
        let rel = rel_path.trim().trim_matches('/');
        path::validate_folder_rel(rel)?;
        let abs = self.root.join(rel);
        // Lexical containment before mkdir (no `..` after validate).
        if !abs.starts_with(&self.root) {
            return Err(VaultError::InvalidPath);
        }
        fs::create_dir_all(&abs).map_err(VaultError::from_io)?;
        let canon = abs.canonicalize().map_err(VaultError::from_io)?;
        if !(canon == self.root || canon.starts_with(&self.root)) {
            return Err(VaultError::InvalidPath);
        }
        Ok(())
    }

    pub fn read(&self, rel_path: &str) -> Result<String, VaultError> {
        let abs = path::resolve_inside_vault(&self.root, rel_path)?;
        if !abs.is_file() {
            return Err(VaultError::NotFound);
        }
        let bytes = fs::read(&abs).map_err(VaultError::from_io)?;
        if bytes.len() as u64 > MAX_NOTE_BYTES {
            return Err(VaultError::TooLarge);
        }
        String::from_utf8(bytes).map_err(|_| VaultError::NotUtf8)
    }

    pub fn write(&mut self, rel_path: &str, content: &str) -> Result<NoteMeta, VaultError> {
        self.write_inner(rel_path, content, true)
    }

    /// Autosave path: atomic rename without `fsync`, so the UI thread stays responsive.
    pub fn write_soft(&mut self, rel_path: &str, content: &str) -> Result<NoteMeta, VaultError> {
        self.write_inner(rel_path, content, false)
    }

    fn write_inner(
        &mut self,
        rel_path: &str,
        content: &str,
        sync: bool,
    ) -> Result<NoteMeta, VaultError> {
        if content.len() as u64 > MAX_NOTE_BYTES {
            return Err(VaultError::TooLarge);
        }
        path::validate_rel_path(rel_path)?;

        let abs = self.root.join(rel_path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).map_err(VaultError::from_io)?;
        }

        // Re-validate after parents exist (canonicalize containment).
        let abs = path::resolve_inside_vault(&self.root, rel_path)?;

        atomic_write(&abs, content.as_bytes(), sync)?;

        self.index.set_note(rel_path, content);
        note_meta_for(&abs, rel_path)
    }

    pub fn delete(&mut self, rel_path: &str) -> Result<(), VaultError> {
        let abs = path::resolve_inside_vault(&self.root, rel_path)?;
        if !abs.is_file() {
            return Err(VaultError::NotFound);
        }
        fs::remove_file(&abs).map_err(VaultError::from_io)?;
        self.index.remove_note(rel_path);
        Ok(())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<NoteMeta, VaultError> {
        path::validate_rel_path(from)?;
        path::validate_rel_path(to)?;

        let abs_from = path::resolve_inside_vault(&self.root, from)?;
        if !abs_from.is_file() {
            return Err(VaultError::NotFound);
        }

        let abs_to = self.root.join(to);
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(VaultError::from_io)?;
        }
        let abs_to = path::resolve_inside_vault(&self.root, to)?;

        if abs_to.exists() {
            return Err(VaultError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "target already exists",
            )));
        }

        let renames = HashMap::from([(from.to_string(), to.to_string())]);
        let pending = self.collect_wikilink_rewrites(&renames)?;

        fs::rename(&abs_from, &abs_to).map_err(VaultError::from_io)?;
        self.index.rename_note(from, to);
        self.apply_wikilink_rewrites(&renames, pending)?;
        note_meta_for(&abs_to, to)
    }

    /// Rename a folder and rewrite index paths for notes inside it.
    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<(), VaultError> {
        let from = from.trim().trim_matches('/');
        let to = to.trim().trim_matches('/');
        path::validate_folder_rel(from)?;
        path::validate_folder_rel(to)?;
        if from == to {
            return Ok(());
        }

        let abs_from = self.folder_abs(from)?;
        if !abs_from.is_dir() {
            return Err(VaultError::NotFound);
        }
        let abs_to = self.folder_abs(to)?;
        if abs_to.exists() {
            return Err(VaultError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "target already exists",
            )));
        }
        if let Some(parent) = abs_to.parent() {
            fs::create_dir_all(parent).map_err(VaultError::from_io)?;
        }

        let prefix = format!("{from}/");
        let notes = self.list()?;
        let mut renames = HashMap::new();
        for note in &notes {
            if let Some(rest) = note.rel_path.strip_prefix(&prefix) {
                renames.insert(note.rel_path.clone(), format!("{to}/{rest}"));
            }
        }
        let pending = self.collect_wikilink_rewrites(&renames)?;

        fs::rename(&abs_from, &abs_to).map_err(VaultError::from_io)?;
        for (old, new) in &renames {
            self.index.rename_note(old, new);
        }
        self.apply_wikilink_rewrites(&renames, pending)?;
        Ok(())
    }

    /// Notes that link to any `renames` key, with bodies rewritten for the post-rename set.
    /// Paths are still pre-rename; call after index/fs rename with [`Self::apply_wikilink_rewrites`].
    fn collect_wikilink_rewrites(
        &self,
        renames: &HashMap<String, String>,
    ) -> Result<Vec<(String, String)>, VaultError> {
        if renames.is_empty() {
            return Ok(Vec::new());
        }

        let mut sources = HashSet::new();
        for old in renames.keys() {
            for src in self.index.backlinks(old) {
                sources.insert(src);
            }
        }
        if sources.is_empty() {
            return Ok(Vec::new());
        }

        let note_paths = self.index.note_paths();
        let notes_after = notes_after_renames(&note_paths, renames);

        let mut pending = Vec::new();
        for src in sources {
            let content = match self.read(&src) {
                Ok(c) => c,
                Err(VaultError::NotUtf8) => continue,
                Err(e) => return Err(e),
            };
            let rewritten =
                rewrite_wikilinks_in_content(&content, &src, &self.index, renames, &notes_after);
            if rewritten != content {
                pending.push((src, rewritten));
            }
        }
        Ok(pending)
    }

    fn apply_wikilink_rewrites(
        &mut self,
        renames: &HashMap<String, String>,
        pending: Vec<(String, String)>,
    ) -> Result<(), VaultError> {
        for (old_src, content) in pending {
            let dest = renames
                .get(&old_src)
                .map(String::as_str)
                .unwrap_or(&old_src);
            self.write_inner(dest, &content, true)?;
        }
        Ok(())
    }

    /// Delete a folder and every note under it.
    pub fn delete_folder(&mut self, rel: &str) -> Result<(), VaultError> {
        let rel = rel.trim().trim_matches('/');
        path::validate_folder_rel(rel)?;
        let abs = self.folder_abs(rel)?;
        if !abs.is_dir() {
            return Err(VaultError::NotFound);
        }

        let prefix = format!("{rel}/");
        let notes = self.list()?;
        for note in notes {
            if note.rel_path.starts_with(&prefix) {
                self.index.remove_note(&note.rel_path);
            }
        }
        fs::remove_dir_all(&abs).map_err(VaultError::from_io)?;
        Ok(())
    }

    fn folder_abs(&self, rel: &str) -> Result<PathBuf, VaultError> {
        let abs = self.root.join(rel);
        if !abs.starts_with(&self.root) {
            return Err(VaultError::InvalidPath);
        }
        Ok(abs)
    }

    fn rebuild_index(&mut self) -> Result<(), VaultError> {
        self.index.clear();
        let notes = self.list()?;
        for note in notes {
            match self.read(&note.rel_path) {
                Ok(content) => self.index.set_note(&note.rel_path, &content),
                Err(VaultError::NotUtf8) => {
                    // Index path only; no outgoing links.
                    self.index.set_note(&note.rel_path, "");
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn walk_notes(
        &self,
        dir: &Path,
        rel_prefix: &str,
        out: &mut Vec<NoteMeta>,
    ) -> Result<(), VaultError> {
        let entries = fs::read_dir(dir).map_err(VaultError::from_io)?;
        for entry in entries {
            let entry = entry.map_err(VaultError::from_io)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let rel = if rel_prefix.is_empty() {
                name.to_string()
            } else {
                format!("{rel_prefix}/{name}")
            };
            let path = entry.path();
            let ft = entry.file_type().map_err(VaultError::from_io)?;
            if ft.is_dir() {
                self.walk_notes(&path, &rel, out)?;
            } else if ft.is_file() && name.ends_with(".md") {
                // Skip invalid paths rather than failing the whole list.
                if path::validate_rel_path(&rel).is_ok() {
                    out.push(note_meta_for(&path, &rel)?);
                }
            }
        }
        Ok(())
    }

    fn walk_folders(
        &self,
        dir: &Path,
        rel_prefix: &str,
        out: &mut Vec<String>,
    ) -> Result<(), VaultError> {
        let entries = fs::read_dir(dir).map_err(VaultError::from_io)?;
        for entry in entries {
            let entry = entry.map_err(VaultError::from_io)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let ft = entry.file_type().map_err(VaultError::from_io)?;
            if !ft.is_dir() {
                continue;
            }
            let rel = if rel_prefix.is_empty() {
                name.to_string()
            } else {
                format!("{rel_prefix}/{name}")
            };
            if path::validate_folder_rel(&rel).is_ok() {
                out.push(rel.clone());
                self.walk_folders(&entry.path(), &rel, out)?;
            }
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, data: &[u8], sync: bool) -> Result<(), VaultError> {
    let parent = path.parent().ok_or(VaultError::InvalidPath)?;
    let tmp_name = format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("note.md"),
        std::process::id()
    );
    let tmp_path = parent.join(tmp_name);

    let result = (|| {
        let mut file = File::create(&tmp_path).map_err(VaultError::from_io)?;
        file.write_all(data).map_err(VaultError::from_io)?;
        if sync {
            file.sync_all().map_err(VaultError::from_io)?;
        }
        fs::rename(&tmp_path, path).map_err(VaultError::from_io)?;
        // Best-effort directory fsync for durability on crash.
        if sync {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

fn note_meta_for(abs: &Path, rel_path: &str) -> Result<NoteMeta, VaultError> {
    let meta = fs::metadata(abs).map_err(VaultError::from_io)?;
    let bytes = fs::read(abs).map_err(VaultError::from_io)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let modified = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let rel = rel_path.replace('\\', "/");
    let title = match std::str::from_utf8(&bytes) {
        Ok(text) => title_from_content(text, &rel),
        Err(_) => title_from_content("", &rel),
    };
    Ok(NoteMeta {
        rel_path: rel,
        title,
        sha256,
        size: meta.len(),
        modified,
    })
}

#[cfg(test)]
mod title_tests {
    use super::title_from_content;

    #[test]
    fn strips_heading_markers() {
        assert_eq!(title_from_content("# Hello\nbody", "a.md"), "Hello");
        assert_eq!(title_from_content("### sdfdfs\n", "x.md"), "sdfdfs");
        assert_eq!(title_from_content("plain title\nmore", "a.md"), "plain title");
    }

    #[test]
    fn falls_back_to_stem_when_empty() {
        assert_eq!(title_from_content("\n\n", "inbox/todo.md"), "todo");
        assert_eq!(title_from_content("###\n", "note.md"), "note");
    }
}
