use std::path::{Component, Path, PathBuf};

use crate::error::VaultError;

pub const MAX_REL_PATH_BYTES: usize = 255;
pub const MAX_NOTE_BYTES: u64 = 1024 * 1024;
pub const MAX_MEDIA_BYTES: u64 = 32 * 1024 * 1024;

/// Fixed vault-relative folder for pasted/dropped attachments (Obsidian-style).
pub const ATTACHMENTS_DIR: &str = "attachments";

/// Obsidian-compatible media extensions (lowercase, no leading dot).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif", "mp4", "webm", "ogv",
    "mov", "mkv", "mp3", "wav", "m4a", "flac", "ogg", "opus", "pdf",
];

/// Validate a vault-relative note path (`.md` only). Does not touch the filesystem.
pub fn validate_rel_path(rel_path: &str) -> Result<(), VaultError> {
    if !rel_path.ends_with(".md") {
        return Err(VaultError::InvalidPath);
    }
    validate_rel_segments(rel_path)
}

/// Validate a vault-relative path for notes or media (allowlisted extension).
pub fn validate_vault_rel_path(rel_path: &str) -> Result<(), VaultError> {
    validate_rel_segments(rel_path)?;
    if !has_allowed_extension(rel_path) {
        return Err(VaultError::InvalidPath);
    }
    Ok(())
}

/// Validate a vault-relative folder path (`inbox`, `projects/work`). No `.md`.
pub fn validate_folder_rel(rel_path: &str) -> Result<(), VaultError> {
    let rel = rel_path.trim().trim_matches('/');
    if rel.is_empty() || rel.ends_with(".md") {
        return Err(VaultError::InvalidPath);
    }
    validate_rel_segments(rel)
}

/// True when `ext` is in the media allowlist (`png`, `.PNG`, …).
pub fn is_media_extension(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    MEDIA_EXTENSIONS.iter().any(|m| *m == e)
}

/// Byte limit for a vault-relative path: notes 1 MiB, media 32 MiB.
pub fn max_bytes_for_rel(rel_path: &str) -> u64 {
    let is_md = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if is_md {
        MAX_NOTE_BYTES
    } else {
        MAX_MEDIA_BYTES
    }
}

fn has_allowed_extension(rel_path: &str) -> bool {
    let Some(ext) = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
    else {
        return false;
    };
    ext == "md" || MEDIA_EXTENSIONS.iter().any(|m| *m == ext)
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
/// Accepts note and media paths ([`validate_vault_rel_path`]).
pub fn resolve_inside_vault(root: &Path, rel_path: &str) -> Result<PathBuf, VaultError> {
    validate_vault_rel_path(rel_path)?;

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

    #[test]
    fn vault_rel_accepts_media_and_md() {
        for p in [
            "note.md",
            "attachments/photo.png",
            "attachments/Pasted image 20240101120000.JPEG",
            "clips/demo.mp4",
            "doc.PDF",
        ] {
            assert!(
                validate_vault_rel_path(p).is_ok(),
                "expected ok for {p:?}"
            );
        }
    }

    #[test]
    fn vault_rel_rejects_bad_ext_and_dot_segments() {
        for p in [
            "note.txt",
            "attachments/.hidden.png",
            ".bluelink/x.png",
            "../x.png",
            "a/../b.png",
            "noext",
        ] {
            assert!(
                validate_vault_rel_path(p).is_err(),
                "expected InvalidPath for {p:?}"
            );
        }
    }

    #[test]
    fn media_ext_helper() {
        assert!(is_media_extension("png"));
        assert!(is_media_extension(".PNG"));
        assert!(!is_media_extension("md"));
        assert!(!is_media_extension("exe"));
    }
}
