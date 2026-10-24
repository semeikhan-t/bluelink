use std::fs;
use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::VaultError;

pub const META_DIR: &str = ".bluelink";
pub const DEVICE_FILE: &str = "device.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMeta {
    pub device_id: String,
    pub name: String,
    /// Base64-encoded 32-byte X25519 private key.
    pub private_key: String,
}

impl DeviceMeta {
    pub fn public_key_bytes(&self) -> Result<[u8; 32], VaultError> {
        let secret = self.static_secret()?;
        Ok(PublicKey::from(&secret).to_bytes())
    }

    pub fn static_secret(&self) -> Result<StaticSecret, VaultError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.private_key)
            .map_err(|_| VaultError::CorruptMeta)?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| VaultError::CorruptMeta)?;
        Ok(StaticSecret::from(arr))
    }
}

pub fn load_or_create(root: &Path) -> Result<DeviceMeta, VaultError> {
    let meta_dir = root.join(META_DIR);
    fs::create_dir_all(&meta_dir).map_err(VaultError::from_io)?;

    let path = meta_dir.join(DEVICE_FILE);
    if path.exists() {
        let raw = fs::read_to_string(&path).map_err(VaultError::from_io)?;
        let meta: DeviceMeta =
            serde_json::from_str(&raw).map_err(|_| VaultError::CorruptMeta)?;
        // Touch private key decode early so CorruptMeta surfaces on open.
        let _ = meta.static_secret()?;
        Ok(meta)
    } else {
        let meta = generate_device_meta()?;
        let json =
            serde_json::to_string_pretty(&meta).map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
        fs::write(&path, json).map_err(VaultError::from_io)?;
        Ok(meta)
    }
}

fn generate_device_meta() -> Result<DeviceMeta, VaultError> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| {
        VaultError::Io(std::io::Error::other(format!("getrandom failed: {e}")))
    })?;
    let secret = StaticSecret::from(seed);
    let private_key =
        base64::engine::general_purpose::STANDARD.encode(secret.to_bytes());

    let name = hostname::fallback_name();

    Ok(DeviceMeta {
        device_id: Uuid::new_v4().to_string(),
        name,
        private_key,
    })
}

mod hostname {
    pub fn fallback_name() -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "bluelink-device".into())
    }
}
