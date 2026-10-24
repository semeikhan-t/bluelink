use std::path::{Component, Path, PathBuf};

use crate::error::VaultError;

pub const MAX_REL_PATH_BYTES: usize = 255;
pub const MAX_NOTE_BYTES: u64 = 1024 * 1024;

/// Validate a vault-relative note path. Does not touch the filesystem.
pub fn validate_rel_path(rel_path: &str) -> Result<(), VaultError> {
    if !rel_path.ends_with(".md") {
        return Err(VaultError::InvalidPath);
    }
    validate_rel_segments(rel_path)
}

/// Validate a vault-relative folder path (`inbox`, `projects/work`). No `.md`.
pub fn validate_folder_rel(rel_path: &str) -> Result<(), VaultError> {
    let rel = rel_path.trim().trim_matches('/');
    if rel.is_empty() || rel.ends_with(".md") {
        return Err(VaultError::InvalidPath);
    }
    validate_rel_segments(rel)
}

fn validate_rel_segments(rel_path: &str) -> Result<(), VaultError> {
    if rel_path.is_empty() || rel_path.len() > MAX_REL_PATH_BYTES {
        return Err(VaultError::InvalidPath);
    }
    if rel_path.starts_with('/') || rel_path.contains('\\') || rel_path.contains('\0') {
        return Err(VaultError::InvalidPath);
    }

    let path = Path::new(rel_path);
    if path.is_absolute() {
        return Err(VaultError::InvalidPath);
    }

    let mut segments = 0;
    for component in path.components() {
        match component {
            Component::Normal(seg) => {
                let s = seg.to_str().ok_or(VaultError::InvalidPath)?;
                // Hidden segments (`.bluelink`, `.something`) are forbidden.
                if s.is_empty() || s.starts_with('.') {
                    return Err(VaultError::InvalidPath);
                }
                segments += 1;
            }
            // No `..` or `.` — paths from peers are a trust boundary.
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(VaultError::InvalidPath);
            }
        }
    }

    if segments == 0 {
        return Err(VaultError::InvalidPath);
    }

    Ok(())
}

/// Resolve `rel_path` under `root`, ensuring the result stays inside the vault.
pub fn resolve_inside_vault(root: &Path, rel_path: &str) -> Result<PathBuf, VaultError> {
    validate_rel_path(rel_path)?;

    let root = root
        .canonicalize()
        .map_err(VaultError::from_io)?;

    let joined = root.join(rel_path);

    // Parent must exist for canonicalize of the final path; check parent containment.
    let parent = joined.parent().ok_or(VaultError::InvalidPath)?;
    if !parent.exists() {
        // Still verify the would-be path cannot escape via .. (already validated)
        // and that joining stays under root by lexical check.
        ensure_under(&root, &joined)?;
        return Ok(joined);
    }

    let canon_parent = parent.canonicalize().map_err(VaultError::from_io)?;
    ensure_under(&root, &canon_parent)?;

    if joined.exists() {
        let canon = joined.canonicalize().map_err(VaultError::from_io)?;
        ensure_under(&root, &canon)?;
        Ok(canon)
    } else {
        let candidate = canon_parent.join(
            joined
                .file_name()
                .ok_or(VaultError::InvalidPath)?,
        );
        ensure_under(&root, &candidate)?;
        Ok(candidate)
    }
}

fn ensure_under(root: &Path, path: &Path) -> Result<(), VaultError> {
    if path == root || path.starts_with(root) {
        Ok(())
    } else {
        Err(VaultError::InvalidPath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_paths() {
        for p in [
            "../x.md",
            "/etc/x.md",
            ".bluelink/trusted.json",
            r"foo\bar.md",
            "note",
            "note.txt",
            "",
            "a/../b.md",
            ".hidden.md",
            "dir/.secret.md",
        ] {
            assert!(
                validate_rel_path(p).is_err(),
                "expected InvalidPath for {p:?}"
            );
        }
    }

    #[test]
    fn accepts_normal_paths() {
        for p in ["note.md", "inbox/todo.md", "Folder/Note.md"] {
            assert!(validate_rel_path(p).is_ok(), "expected ok for {p:?}");
        }
    }

    #[test]
    fn folder_rel() {
        assert!(validate_folder_rel("inbox").is_ok());
        assert!(validate_folder_rel("projects/work").is_ok());
        assert!(validate_folder_rel("inbox/todo.md").is_err());
        assert!(validate_folder_rel("../x").is_err());
        assert!(validate_folder_rel(".hidden").is_err());
        assert!(validate_folder_rel("").is_err());
    }
}
