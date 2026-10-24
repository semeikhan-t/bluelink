use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

use bluelink_core::{DeviceMeta, Vault};

use crate::error::PairingError;
use crate::transport::Transport;
use crate::trust::{TrustStore, TrustedDevice};

type HmacSha256 = Hmac<Sha256>;

pub const PAIRING_TTL_MS: u64 = 120_000;
pub const PAIRING_ATTEMPTS: u8 = 3;

const ID_A: &[u8] = b"bluelink-join";
const ID_B: &[u8] = b"bluelink-host";
const CONFIRM_LABEL: &[u8] = b"bluelink-pairing-confirm-v1";
const IDENTITY_NONCE: &[u8; 12] = b"bluelink-id1"; // fixed nonce OK: one message per K

#[derive(Debug, Clone, Serialize, Deserialize)]
enum PairWire {
    /// Join → Host: SPAKE2 message A
    SpakeA(Vec<u8>),
    /// Host → Join: SPAKE2 message B
    SpakeB(Vec<u8>),
    /// HMAC confirmation
    Confirm(Vec<u8>),
    /// ChaCha20-Poly1305 sealed DeviceIdentity under K
    Identity(Vec<u8>),
    /// Pairing rejected / closed
    Fail { reason: FailReason },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum FailReason {
    WrongCode,
    Expired,
    NotInPairingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceIdentity {
    device_id: String,
    name: String,
    public_key: String,
}

#[derive(Debug, Clone)]
pub struct HostSessionInfo {
    pub code: String,
    pub expires_at: u64,
    pub attempts_left: u8,
}

/// Generate a cryptographically strong 6-digit pairing code.
pub fn generate_code() -> Result<String, PairingError> {
    let mut buf = [0u8; 4];
    getrandom::getrandom(&mut buf).map_err(|e| PairingError::Crypto(e.to_string()))?;
    let n = u32::from_le_bytes(buf) % 1_000_000;
    Ok(format!("{n:06}"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn encode(msg: &PairWire) -> Result<Vec<u8>, PairingError> {
    postcard::to_allocvec(msg).map_err(|e| PairingError::Protocol(e.to_string()))
}

fn decode(bytes: &[u8]) -> Result<PairWire, PairingError> {
    postcard::from_bytes(bytes).map_err(|e| PairingError::Protocol(e.to_string()))
}

async fn send(t: &mut dyn Transport, msg: &PairWire) -> Result<(), PairingError> {
    t.send(&encode(msg)?).await?;
    Ok(())
}

async fn recv(t: &mut dyn Transport) -> Result<PairWire, PairingError> {
    let bytes = t.recv().await?;
    decode(&bytes)
}

fn hmac_confirm(key: &[u8], transcript: &[u8]) -> Result<Vec<u8>, PairingError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key)
        .map_err(|e| PairingError::Crypto(e.to_string()))?;
    mac.update(CONFIRM_LABEL);
    mac.update(transcript);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn seal_identity(key: &[u8], id: &DeviceIdentity) -> Result<Vec<u8>, PairingError> {
    let k = key_32(key)?;
    let cipher = ChaCha20Poly1305::new((&k).into());
    let plain =
        postcard::to_allocvec(id).map_err(|e| PairingError::Protocol(e.to_string()))?;
    cipher
        .encrypt(Nonce::from_slice(IDENTITY_NONCE), plain.as_ref())
        .map_err(|e| PairingError::Crypto(e.to_string()))
}

fn open_identity(key: &[u8], blob: &[u8]) -> Result<DeviceIdentity, PairingError> {
    let k = key_32(key)?;
    let cipher = ChaCha20Poly1305::new((&k).into());
    let plain = cipher
        .decrypt(Nonce::from_slice(IDENTITY_NONCE), blob)
        .map_err(|_| PairingError::WrongCode)?;
    postcard::from_bytes(&plain).map_err(|e| PairingError::Protocol(e.to_string()))
}

fn key_32(key: &[u8]) -> Result<[u8; 32], PairingError> {
    if key.len() < 32 {
        return Err(PairingError::Crypto("SPAKE2 key too short".into()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&key[..32]);
    Ok(out)
}

fn public_key_b64(device: &DeviceMeta) -> Result<String, PairingError> {
    let bytes = device.public_key_bytes()?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn my_identity(device: &DeviceMeta) -> Result<DeviceIdentity, PairingError> {
    Ok(DeviceIdentity {
        device_id: device.device_id.clone(),
        name: device.name.clone(),
        public_key: public_key_b64(device)?,
    })
}

/// Host (PC): wait for a join attempt with the given code.
///
/// Call after advertising pairing-open. Decrements attempts on WrongCode.
pub async fn host_accept(
    transport: &mut dyn Transport,
    vault: &Vault,
    code: &str,
    expires_at: u64,
    attempts_left: &mut u8,
) -> Result<TrustedDevice, PairingError> {
    if now_ms() > expires_at || *attempts_left == 0 {
        send(transport, &PairWire::Fail {
            reason: FailReason::Expired,
        })
        .await
        .ok();
        return Err(PairingError::Expired);
    }

    let msg = recv(transport).await?;
    let PairWire::SpakeA(msg_a) = msg else {
        return Err(PairingError::Protocol("expected SpakeA".into()));
    };

    let password = Password::new(code.as_bytes());
    let (state, msg_b) = Spake2::<Ed25519Group>::start_b(
        &password,
        &Identity::new(ID_A),
        &Identity::new(ID_B),
    );
    send(transport, &PairWire::SpakeB(msg_b.clone())).await?;

    let key = match state.finish(&msg_a) {
        Ok(k) => k,
        Err(_) => {
            *attempts_left = attempts_left.saturating_sub(1);
            let reason = if *attempts_left == 0 || now_ms() > expires_at {
                FailReason::Expired
            } else {
                FailReason::WrongCode
            };
            send(transport, &PairWire::Fail { reason }).await.ok();
            return Err(match reason {
                FailReason::Expired => PairingError::Expired,
                _ => PairingError::WrongCode,
            });
        }
    };

    let mut transcript = Vec::with_capacity(msg_a.len() + msg_b.len());
    transcript.extend_from_slice(&msg_a);
    transcript.extend_from_slice(&msg_b);
    let expected = hmac_confirm(&key, &transcript)?;

    // Exchange confirms: join sends first, then host
    let peer_confirm = match recv(transport).await? {
        PairWire::Confirm(c) => c,
        PairWire::Fail { reason } => {
            return Err(fail_to_err(reason));
        }
        _ => return Err(PairingError::Protocol("expected Confirm".into())),
    };

    if peer_confirm != expected {
        *attempts_left = attempts_left.saturating_sub(1);
        let reason = if *attempts_left == 0 {
            FailReason::Expired
        } else {
            FailReason::WrongCode
        };
        send(transport, &PairWire::Fail { reason }).await.ok();
        return Err(match reason {
            FailReason::Expired => PairingError::Expired,
            _ => PairingError::WrongCode,
        });
    }

    send(transport, &PairWire::Confirm(expected)).await?;

    // Identity exchange: join sends first
    let peer_blob = match recv(transport).await? {
        PairWire::Identity(b) => b,
        PairWire::Fail { reason } => return Err(fail_to_err(reason)),
        _ => return Err(PairingError::Protocol("expected Identity".into())),
    };
    let peer = open_identity(&key, &peer_blob)?;

    let mine = my_identity(vault.device())?;
    send(
        transport,
        &PairWire::Identity(seal_identity(&key, &mine)?),
    )
    .await?;

    let trusted = TrustedDevice {
        device_id: peer.device_id,
        name: peer.name,
        public_key: peer.public_key,
        paired_at: now_ms(),
    };
    let mut store = TrustStore::load(vault.root())?;
    store.upsert(trusted.clone())?;
    Ok(trusted)
}

/// Join (phone): connect and pair using the 6-digit code.
pub async fn join(
    transport: &mut dyn Transport,
    vault: &Vault,
    code: &str,
) -> Result<TrustedDevice, PairingError> {
    let password = Password::new(code.as_bytes());
    let (state, msg_a) = Spake2::<Ed25519Group>::start_a(
        &password,
        &Identity::new(ID_A),
        &Identity::new(ID_B),
    );
    send(transport, &PairWire::SpakeA(msg_a.clone())).await?;

    let msg_b = match recv(transport).await? {
        PairWire::SpakeB(b) => b,
        PairWire::Fail { reason } => return Err(fail_to_err(reason)),
        _ => return Err(PairingError::Protocol("expected SpakeB".into())),
    };

    let key = state
        .finish(&msg_b)
        .map_err(|_| PairingError::WrongCode)?;

    let mut transcript = Vec::with_capacity(msg_a.len() + msg_b.len());
    transcript.extend_from_slice(&msg_a);
    transcript.extend_from_slice(&msg_b);
    let confirm = hmac_confirm(&key, &transcript)?;
    send(transport, &PairWire::Confirm(confirm.clone())).await?;

    match recv(transport).await? {
        PairWire::Confirm(peer) => {
            if peer != confirm {
                return Err(PairingError::WrongCode);
            }
        }
        PairWire::Fail { reason } => return Err(fail_to_err(reason)),
        _ => return Err(PairingError::Protocol("expected Confirm".into())),
    }

    let mine = my_identity(vault.device())?;
    send(
        transport,
        &PairWire::Identity(seal_identity(&key, &mine)?),
    )
    .await?;

    let peer_blob = match recv(transport).await? {
        PairWire::Identity(b) => b,
        PairWire::Fail { reason } => return Err(fail_to_err(reason)),
        _ => return Err(PairingError::Protocol("expected Identity".into())),
    };
    let peer = open_identity(&key, &peer_blob)?;

    let trusted = TrustedDevice {
        device_id: peer.device_id,
        name: peer.name,
        public_key: peer.public_key,
        paired_at: now_ms(),
    };
    let mut store = TrustStore::load(vault.root())?;
    store.upsert(trusted.clone())?;
    Ok(trusted)
}

fn fail_to_err(reason: FailReason) -> PairingError {
    match reason {
        FailReason::WrongCode => PairingError::WrongCode,
        FailReason::Expired => PairingError::Expired,
        FailReason::NotInPairingMode => PairingError::NotInPairingMode,
    }
}

/// Convenience: create host session metadata (code + TTL).
pub fn host_session() -> Result<HostSessionInfo, PairingError> {
    let code = generate_code()?;
    Ok(HostSessionInfo {
        code,
        expires_at: now_ms() + PAIRING_TTL_MS,
        attempts_left: PAIRING_ATTEMPTS,
    })
}
