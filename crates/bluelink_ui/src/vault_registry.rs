//! Known vault folders remembered across launches.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultRegistry {
    #[serde(default)]
    pub last: Option<PathBuf>,
    #[serde(default)]
    pub entries: Vec<VaultEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub path: PathBuf,
}

impl VaultRegistry {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }

    /// Display name = folder basename.
    pub fn display_name(path: &Path) -> String {
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("vault")
            .to_string()
    }

    /// Add or refresh `path` as last-used. Dedupes by normalized path string.
    pub fn remember(&mut self, path: &Path) {
        let path = normalize_path(path);
        self.entries.retain(|e| normalize_path(&e.path) != path);
        self.entries.insert(0, VaultEntry { path: path.clone() });
        self.last = Some(path);
    }

    pub fn forget(&mut self, path: &Path) {
        let path = normalize_path(path);
        self.entries.retain(|e| normalize_path(&e.path) != path);
        if self.last.as_ref().map(|p| normalize_path(p)) == Some(path) {
            self.last = self.entries.first().map(|e| e.path.clone());
        }
    }

    /// `last` if the directory still exists.
    pub fn last_existing(&self) -> Option<PathBuf> {
        let last = self.last.as_ref()?;
        if last.is_dir() {
            Some(last.clone())
        } else {
            None
        }
    }

    /// Drop entries whose folders no longer exist.
    pub fn prune_missing(&mut self) {
        self.entries.retain(|e| e.path.is_dir());
        if let Some(last) = &self.last {
            if !last.is_dir() {
                self.last = self.entries.first().map(|e| e.path.clone());
            }
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("bluelink").join("vaults.json");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("bluelink")
        .join("vaults.json")
}

/// Default parent for Create vault: `~/BlueLink`.
pub fn default_vaults_parent() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("BlueLink")
}

/// Reject empty, `.`, `..`, separators, and nulls.
pub fn validate_vault_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("name must not contain path separators".into());
    }
    if name == "." || name == ".." {
        return Err("invalid name".into());
    }
    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    // Prefer canonicalize when the path exists; otherwise clean obvious `./` noise.
    if let Ok(c) = path.canonicalize() {
        return c;
    }
    path.components().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn remember_dedupes_and_sets_last() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        let mut reg = VaultRegistry::default();
        reg.remember(&a);
        reg.remember(&b);
        reg.remember(&a);
        assert_eq!(reg.entries.len(), 2);
        assert_eq!(
            reg.last.as_ref().map(|p| normalize_path(p)),
            Some(normalize_path(&a))
        );
        assert_eq!(normalize_path(&reg.entries[0].path), normalize_path(&a));
    }

    #[test]
    fn forget_updates_last() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        let mut reg = VaultRegistry::default();
        reg.remember(&a);
        reg.remember(&b);
        reg.forget(&b);
        assert_eq!(reg.entries.len(), 1);
        assert_eq!(
            reg.last.as_ref().map(|p| normalize_path(p)),
            Some(normalize_path(&a))
        );
    }

    #[test]
    fn validate_name() {
        assert!(validate_vault_name("Work").is_ok());
        assert!(validate_vault_name("").is_err());
        assert!(validate_vault_name("a/b").is_err());
        assert!(validate_vault_name("..").is_err());
    }
}
