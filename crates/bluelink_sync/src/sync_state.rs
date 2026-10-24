use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use bluelink_core::VaultError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StateFile {
    /// rel_path -> sha256 hex at last successful Commit with this peer.
    notes: HashMap<String, String>,
}

pub struct SyncState {
    path: PathBuf,
    notes: HashMap<String, String>,
}

impl SyncState {
    pub fn load(vault_root: &Path, peer_id: &str) -> Result<Self, VaultError> {
        let dir = vault_root.join(".bluelink").join("sync_state");
        fs::create_dir_all(&dir)?;
        // peer_id is uuid — safe as filename
        let path = dir.join(format!("{peer_id}.json"));
        let notes = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let file: StateFile =
                serde_json::from_str(&raw).map_err(|_| VaultError::CorruptMeta)?;
            file.notes
        } else {
            HashMap::new()
        };
        Ok(Self { path, notes })
    }

    pub fn get(&self, rel_path: &str) -> Option<&str> {
        self.notes.get(rel_path).map(|s| s.as_str())
    }

    pub fn notes(&self) -> &HashMap<String, String> {
        &self.notes
    }

    /// Replace entire baseline after a successful Commit.
    pub fn commit(&mut self, notes: HashMap<String, String>) -> Result<(), VaultError> {
        self.notes = notes;
        self.save()
    }

    fn save(&self) -> Result<(), VaultError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = StateFile {
            notes: self.notes.clone(),
        };
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
        fs::write(&self.path, json)?;
        Ok(())
    }
}
