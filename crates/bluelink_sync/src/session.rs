use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use bluelink_core::{
    max_bytes_for_rel, validate_vault_rel_path, NoteMeta, Vault,
};

use crate::conflict::conflict_path;
use crate::error::SyncError;
use crate::noise_session::NoiseSession;
use crate::plan::{build_plan, Action};
use crate::proto::{
    decode, encode, ErrorCode, Message, NoteEntry, APP_VERSION, MANIFEST_CHUNK, MAX_NOTES,
    PROTOCOL_VERSION, PUT_CHUNK_BYTES,
};
use crate::sync_state::SyncState;
use crate::transport::Transport;
use crate::trust::TrustStore;

const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const SYNC_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Phone: builds the plan and drives Get/Put/Delete.
    Initiator,
    /// PC: serves requests until Commit.
    Responder,
}

pub struct SyncOptions<'a> {
    pub role: Role,
    /// Paths open in editor with unsaved edits (initiator).
    pub unsaved: &'a HashSet<String>,
    /// When set, checked between steps; abort leaves sync_state unchanged.
    pub cancel: Option<&'a AtomicBool>,
}

impl<'a> SyncOptions<'a> {
    pub fn initiator(unsaved: &'a HashSet<String>) -> Self {
        Self {
            role: Role::Initiator,
            unsaved,
            cancel: None,
        }
    }

    pub fn responder(unsaved: &'a HashSet<String>) -> Self {
        Self {
            role: Role::Responder,
            unsaved,
            cancel: None,
        }
    }
}

/// Run a full sync session over an already-connected transport.
pub async fn run_sync(
    transport: &mut dyn Transport,
    vault: &mut Vault,
    opts: SyncOptions<'_>,
) -> Result<(), SyncError> {
    let cancel = opts.cancel;
    check_cancel(cancel)?;

    let trust = TrustStore::load(vault.root())?;
    let mut noise = match opts.role {
        Role::Initiator => {
            NoiseSession::handshake_initiator(transport, vault.device(), &trust).await?
        }
        Role::Responder => {
            NoiseSession::handshake_responder(transport, vault.device(), &trust).await?
        }
    };

    let peer = trust
        .find_by_public_key(&noise.remote_public_key_b64)
        .ok_or(SyncError::Untrusted)?
        .clone();

    // Whole-sync bound from ТЗ (10 min). Idle gaps use IDLE_TIMEOUT on recv.
    let body = async {
        match opts.role {
            Role::Initiator => {
                initiator_body(
                    transport,
                    &mut noise,
                    vault,
                    &peer.device_id,
                    &peer.name,
                    opts.unsaved,
                    cancel,
                )
                .await
            }
            Role::Responder => {
                responder_body(transport, &mut noise, vault, &peer.device_id, cancel).await
            }
        }
    };
    tokio::time::timeout(SYNC_TIMEOUT, body)
        .await
        .map_err(|_| SyncError::Timeout)?
}

async fn initiator_body(
    transport: &mut dyn Transport,
    noise: &mut NoiseSession,
    vault: &mut Vault,
    peer_id: &str,
    peer_name: &str,
    unsaved: &HashSet<String>,
    cancel: Option<&AtomicBool>,
) -> Result<(), SyncError> {
    send_msg(
        noise,
        transport,
        &Message::Hello {
            protocol: PROTOCOL_VERSION,
            device_id: vault.device().device_id.clone(),
            app_version: APP_VERSION.into(),
        },
    )
    .await?;

    match recv_msg(noise, transport).await? {
        Message::HelloAck { protocol, .. } => {
            if protocol != PROTOCOL_VERSION {
                return Err(SyncError::ProtocolMismatch);
            }
        }
        Message::Error {
            code: ErrorCode::ProtocolMismatch,
            ..
        } => return Err(SyncError::ProtocolMismatch),
        other => {
            return Err(SyncError::Protocol(format!("expected HelloAck, got {other:?}")));
        }
    }

    check_cancel(cancel)?;
    let local_notes = list_as_map(vault)?;
    send_manifest(noise, transport, &local_notes).await?;
    let remote_notes = recv_manifest(noise, transport).await?;

    let mut state = SyncState::load(vault.root(), peer_id)?;
    let base: HashMap<String, String> = state.notes().clone();

    let plan = build_plan(&local_notes, &remote_notes, &base, unsaved);
    check_cancel(cancel)?;

    for action in &plan {
        check_cancel(cancel)?;
        match action {
            Action::Noop { .. } => {}
            Action::Push {
                rel_path,
                local_hash,
            } => {
                if let Some(hash) = local_hash {
                    push_file(noise, transport, vault, rel_path, hash).await?;
                } else {
                    // delete on remote — need expected hash from remote manifest
                    let expected = remote_notes
                        .get(rel_path)
                        .cloned()
                        .unwrap_or_default();
                    send_msg(
                        noise,
                        transport,
                        &Message::Delete {
                            rel_path: rel_path.clone(),
                            expected_sha256: expected,
                        },
                    )
                    .await?;
                    // wait ack via empty Put? Responder applies silently; use Commit-time only.
                    // Responder replies with a tiny Ack using Put last empty? Simpler: Delete is fire-and-forget
                    // and responder confirms with Message::CommitAck style — add DeleteAck as implicit by continuing.
                }
            }
            Action::Pull {
                rel_path,
                remote_hash,
            } => {
                if let Some(hash) = remote_hash {
                    pull_file(noise, transport, vault, rel_path, hash).await?;
                } else {
                    // delete local (md or media)
                    if vault.read_bytes(rel_path).is_ok() {
                        vault.delete(rel_path)?;
                    }
                }
            }
            Action::Conflict { rel_path } => {
                // Fetch remote → conflict sibling; keep local; push local so both converge.
                let remote_hash = remote_notes
                    .get(rel_path)
                    .ok_or_else(|| SyncError::Protocol("conflict missing remote".into()))?;
                let remote_bytes =
                    fetch_bytes(noise, transport, rel_path, remote_hash).await?;
                let cpath = conflict_path(rel_path, peer_name);
                write_synced(vault, &cpath, remote_bytes)?;
                let local_hash = local_notes
                    .get(rel_path)
                    .ok_or_else(|| SyncError::Protocol("conflict missing local".into()))?;
                push_file(noise, transport, vault, rel_path, local_hash).await?;
            }
        }
    }

    check_cancel(cancel)?;
    send_msg(noise, transport, &Message::Commit).await?;
    match recv_msg(noise, transport).await? {
        Message::CommitAck => {}
        other => {
            return Err(SyncError::Protocol(format!("expected CommitAck, got {other:?}")));
        }
    }

    // Persist new baseline from post-sync local vault.
    let after = list_as_map(vault)?;
    // Exclude conflict siblings from baseline? Keep them — they're real notes.
    state.commit(after)?;
    Ok(())
}

async fn responder_body(
    transport: &mut dyn Transport,
    noise: &mut NoiseSession,
    vault: &mut Vault,
    peer_id: &str,
    cancel: Option<&AtomicBool>,
) -> Result<(), SyncError> {
    match recv_msg(noise, transport).await? {
        Message::Hello { protocol, .. } => {
            if protocol != PROTOCOL_VERSION {
                send_msg(
                    noise,
                    transport,
                    &Message::Error {
                        code: ErrorCode::ProtocolMismatch,
                        message: "protocol mismatch".into(),
                    },
                )
                .await
                .ok();
                return Err(SyncError::ProtocolMismatch);
            }
            send_msg(
                noise,
                transport,
                &Message::HelloAck {
                    protocol: PROTOCOL_VERSION,
                    device_id: vault.device().device_id.clone(),
                    app_version: APP_VERSION.into(),
                },
            )
            .await?;
        }
        other => {
            return Err(SyncError::Protocol(format!("expected Hello, got {other:?}")));
        }
    }

    check_cancel(cancel)?;
    let _remote_manifest = recv_manifest(noise, transport).await?;
    let local_notes = list_as_map(vault)?;
    send_manifest(noise, transport, &local_notes).await?;

    // Serve until Commit
    loop {
        check_cancel(cancel)?;
        match recv_msg(noise, transport).await? {
            Message::Get { rel_path, sha256 } => {
                serve_get(noise, transport, vault, &rel_path, &sha256).await?;
            }
            Message::Put {
                rel_path,
                sha256,
                offset,
                last,
                data,
            } => {
                // Accumulate in a side buffer keyed by path — for simplicity require sequential Puts for one file
                serve_put_start(
                    noise,
                    transport,
                    vault,
                    PutAcc {
                        rel_path,
                        sha256,
                        offset,
                        last,
                        data,
                    },
                )
                .await?;
            }
            Message::Delete {
                rel_path,
                expected_sha256,
            } => {
                if let Ok(bytes) = vault.read_bytes(&rel_path) {
                    let hash = hex_sha256(&bytes);
                    if hash == expected_sha256 || expected_sha256.is_empty() {
                        vault.delete(&rel_path)?;
                    }
                }
            }
            Message::Commit => {
                send_msg(noise, transport, &Message::CommitAck).await?;
                let after = list_as_map(vault)?;
                let mut state = SyncState::load(vault.root(), peer_id)?;
                state.commit(after)?;
                return Ok(());
            }
            Message::Error { code, .. } => {
                return Err(map_error_code(code));
            }
            other => {
                return Err(SyncError::Protocol(format!("unexpected {other:?}")));
            }
        }
    }
}

struct PutAcc {
    rel_path: String,
    sha256: String,
    offset: u64,
    last: bool,
    data: Vec<u8>,
}

/// Handle a Put stream: first chunk may start mid-protocol; we buffer until last.
/// For Loopback tests Puts for one file are contiguous.
async fn serve_put_start(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    vault: &mut Vault,
    first: PutAcc,
) -> Result<(), SyncError> {
    validate_vault_rel_path(&first.rel_path).map_err(|_| SyncError::InvalidPath)?;
    let max = max_bytes_for_rel(&first.rel_path);
    let mut buf = Vec::new();
    if first.offset != 0 {
        return Err(SyncError::Protocol("put offset must start at 0".into()));
    }
    buf.extend_from_slice(&first.data);
    let sha = first.sha256;
    let path = first.rel_path;
    let mut last = first.last;

    while !last {
        if buf.len() as u64 > max {
            return Err(SyncError::TooLarge);
        }
        match recv_msg(noise, transport).await? {
            Message::Put {
                rel_path,
                sha256,
                offset,
                last: is_last,
                data,
            } => {
                if rel_path != path || sha256 != sha {
                    return Err(SyncError::Protocol("interleaved put".into()));
                }
                if offset as usize != buf.len() {
                    return Err(SyncError::Protocol("put gap".into()));
                }
                buf.extend_from_slice(&data);
                last = is_last;
            }
            other => {
                return Err(SyncError::Protocol(format!("expected Put, got {other:?}")));
            }
        }
    }

    if buf.len() as u64 > max {
        return Err(SyncError::TooLarge);
    }
    let got = hex_sha256(&buf);
    if got != sha {
        return Err(SyncError::IntegrityError);
    }
    write_synced(vault, &path, buf)?;
    Ok(())
}

async fn serve_get(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    vault: &Vault,
    rel_path: &str,
    sha256: &str,
) -> Result<(), SyncError> {
    let content = vault.read_bytes(rel_path)?;
    let hash = hex_sha256(&content);
    if hash != sha256 {
        return Err(SyncError::IntegrityError);
    }
    send_put_chunks(noise, transport, rel_path, &hash, &content).await
}

async fn push_file(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    vault: &Vault,
    rel_path: &str,
    hash: &str,
) -> Result<(), SyncError> {
    let content = vault.read_bytes(rel_path)?;
    let got = hex_sha256(&content);
    if got != hash {
        return Err(SyncError::IntegrityError);
    }
    send_put_chunks(noise, transport, rel_path, hash, &content).await
}

async fn pull_file(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    vault: &mut Vault,
    rel_path: &str,
    hash: &str,
) -> Result<(), SyncError> {
    let bytes = fetch_bytes(noise, transport, rel_path, hash).await?;
    write_synced(vault, rel_path, bytes)?;
    Ok(())
}

async fn fetch_bytes(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    rel_path: &str,
    hash: &str,
) -> Result<Vec<u8>, SyncError> {
    send_msg(
        noise,
        transport,
        &Message::Get {
            rel_path: rel_path.to_string(),
            sha256: hash.to_string(),
        },
    )
    .await?;

    let max = max_bytes_for_rel(rel_path);
    let mut buf = Vec::new();
    loop {
        match recv_msg(noise, transport).await? {
            Message::Put {
                rel_path: p,
                sha256,
                offset,
                last,
                data,
            } => {
                if p != rel_path || sha256 != hash {
                    return Err(SyncError::Protocol("get/put mismatch".into()));
                }
                if offset as usize != buf.len() {
                    return Err(SyncError::Protocol("put gap".into()));
                }
                buf.extend_from_slice(&data);
                if buf.len() as u64 > max {
                    return Err(SyncError::TooLarge);
                }
                if last {
                    break;
                }
            }
            other => {
                return Err(SyncError::Protocol(format!("expected Put, got {other:?}")));
            }
        }
    }
    if hex_sha256(&buf) != hash {
        return Err(SyncError::IntegrityError);
    }
    Ok(buf)
}

async fn send_put_chunks(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    rel_path: &str,
    hash: &str,
    data: &[u8],
) -> Result<(), SyncError> {
    if data.is_empty() {
        send_msg(
            noise,
            transport,
            &Message::Put {
                rel_path: rel_path.to_string(),
                sha256: hash.to_string(),
                offset: 0,
                last: true,
                data: Vec::new(),
            },
        )
        .await?;
        return Ok(());
    }
    let mut offset = 0usize;
    while offset < data.len() {
        let end = (offset + PUT_CHUNK_BYTES).min(data.len());
        let last = end == data.len();
        send_msg(
            noise,
            transport,
            &Message::Put {
                rel_path: rel_path.to_string(),
                sha256: hash.to_string(),
                offset: offset as u64,
                last,
                data: data[offset..end].to_vec(),
            },
        )
        .await?;
        offset = end;
    }
    Ok(())
}

async fn send_manifest(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    notes: &HashMap<String, String>,
) -> Result<(), SyncError> {
    if notes.len() > MAX_NOTES {
        return Err(SyncError::TooManyNotes);
    }
    let mut entries: Vec<NoteEntry> = notes
        .iter()
        .map(|(rel_path, sha256)| NoteEntry {
            rel_path: rel_path.clone(),
            sha256: sha256.clone(),
            size: 0,
        })
        .collect();
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    if entries.is_empty() {
        send_msg(
            noise,
            transport,
            &Message::Manifest {
                part: 1,
                total: 1,
                notes: vec![],
            },
        )
        .await?;
        return Ok(());
    }

    let total = ((entries.len() + MANIFEST_CHUNK - 1) / MANIFEST_CHUNK) as u16;
    for (i, chunk) in entries.chunks(MANIFEST_CHUNK).enumerate() {
        send_msg(
            noise,
            transport,
            &Message::Manifest {
                part: (i + 1) as u16,
                total,
                notes: chunk.to_vec(),
            },
        )
        .await?;
    }
    Ok(())
}

async fn recv_manifest(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
) -> Result<HashMap<String, String>, SyncError> {
    let mut map = HashMap::new();
    let mut total = None;
    loop {
        match recv_msg(noise, transport).await? {
            Message::Manifest { part, total: t, notes } => {
                if let Some(tt) = total {
                    if tt != t {
                        return Err(SyncError::Protocol("manifest total changed".into()));
                    }
                } else {
                    total = Some(t);
                }
                for n in notes {
                    map.insert(n.rel_path, n.sha256);
                }
                if part == t {
                    break;
                }
            }
            Message::Error {
                code: ErrorCode::TooManyNotes,
                ..
            } => return Err(SyncError::TooManyNotes),
            other => {
                return Err(SyncError::Protocol(format!("expected Manifest, got {other:?}")));
            }
        }
    }
    if map.len() > MAX_NOTES {
        return Err(SyncError::TooManyNotes);
    }
    Ok(map)
}

async fn send_msg(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
    msg: &Message,
) -> Result<(), SyncError> {
    let bytes = encode(msg).map_err(|e| SyncError::Protocol(e.to_string()))?;
    noise.send(transport, &bytes).await
}

async fn recv_msg(
    noise: &mut NoiseSession,
    transport: &mut dyn Transport,
) -> Result<Message, SyncError> {
    let bytes = tokio::time::timeout(IDLE_TIMEOUT, noise.recv(transport))
        .await
        .map_err(|_| SyncError::Timeout)??;
    decode(&bytes).map_err(|e| SyncError::Protocol(e.to_string()))
}

fn list_as_map(vault: &Vault) -> Result<HashMap<String, String>, SyncError> {
    // Inventory is md + media; notes UI still uses Vault::list() (md-only).
    let files: Vec<NoteMeta> = vault.list_files()?;
    if files.len() > MAX_NOTES {
        return Err(SyncError::TooManyNotes);
    }
    Ok(files
        .into_iter()
        .map(|n| (n.rel_path, n.sha256))
        .collect())
}

/// Write sync payload: `.md` must be UTF-8 via [`Vault::write`]; media via bytes.
fn write_synced(vault: &mut Vault, rel_path: &str, buf: Vec<u8>) -> Result<(), SyncError> {
    if is_markdown_rel(rel_path) {
        let text = String::from_utf8(buf).map_err(|_| SyncError::IntegrityError)?;
        vault.write(rel_path, &text)?;
    } else {
        vault.write_bytes(rel_path, &buf)?;
    }
    Ok(())
}

fn is_markdown_rel(rel_path: &str) -> bool {
    std::path::Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), SyncError> {
    if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
        Err(SyncError::Cancelled)
    } else {
        Ok(())
    }
}

fn map_error_code(code: ErrorCode) -> SyncError {
    match code {
        ErrorCode::Untrusted => SyncError::Untrusted,
        ErrorCode::ProtocolMismatch => SyncError::ProtocolMismatch,
        ErrorCode::InvalidPath => SyncError::InvalidPath,
        ErrorCode::TooLarge => SyncError::TooLarge,
        ErrorCode::IntegrityError => SyncError::IntegrityError,
        ErrorCode::TooManyNotes => SyncError::TooManyNotes,
        ErrorCode::Cancelled => SyncError::Cancelled,
        ErrorCode::Other => SyncError::Protocol("peer error".into()),
    }
}

