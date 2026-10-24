use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use bluelink_core::VaultError;

const TRUSTED_FILE: &str = "trusted.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: String,
    pub name: String,
    /// Base64-encoded X25519 public key.
    pub public_key: String,
    /// Unix milliseconds.
    pub paired_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TrustedFile {
    devices: Vec<TrustedDevice>,
}

pub struct TrustStore {
    path: PathBuf,
    devices: Vec<TrustedDevice>,
}

impl TrustStore {
    pub fn load(vault_root: &Path) -> Result<Self, VaultError> {
        let path = vault_root.join(".bluelink").join(TRUSTED_FILE);
        let devices = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let file: TrustedFile =
                serde_json::from_str(&raw).map_err(|_| VaultError::CorruptMeta)?;
            file.devices
        } else {
            Vec::new()
        };
        Ok(Self { path, devices })
    }

    pub fn list(&self) -> &[TrustedDevice] {
        &self.devices
    }

    pub fn get(&self, device_id: &str) -> Option<&TrustedDevice> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }

    pub fn find_by_public_key(&self, public_key_b64: &str) -> Option<&TrustedDevice> {
        self.devices.iter().find(|d| d.public_key == public_key_b64)
    }

    pub fn upsert(&mut self, device: TrustedDevice) -> Result<(), VaultError> {
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|d| d.device_id == device.device_id)
        {
            *existing = device;
        } else {
            self.devices.push(device);
        }
        self.save()
    }

    pub fn revoke(&mut self, device_id: &str) -> Result<bool, VaultError> {
        let before = self.devices.len();
        self.devices.retain(|d| d.device_id != device_id);
        let removed = self.devices.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    fn save(&self) -> Result<(), VaultError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = TrustedFile {
            devices: self.devices.clone(),
        };
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
        fs::write(&self.path, json)?;
        Ok(())
    }
}
