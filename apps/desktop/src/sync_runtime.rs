//! Tokio + BlueZ SyncRuntime: pairing host + sync Responder.
//!
//! Desktop never initiates sync (phone does). UI drives ShowCode / Revoke / Cancel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bluelink_ble::{BluezPeripheral, BluezPeripheralConfig};
use bluelink_core::Vault;
use bluelink_sync::{
    host_accept, host_session, run_sync, Role, SyncOptions, TrustStore, PAIRING_TTL_MS,
};
use bluelink_ui::sync_ui::{SyncRuntimeIo, SyncStatus, SyncUiCommand, SyncUiEvent};

enum Control {
    SetVault(Option<PathBuf>),
    Shutdown,
}

/// Background BLE + sync worker. Lifetime owned by [`super::DesktopApp`].
pub struct SyncRuntime {
    ctrl_tx: mpsc::Sender<Control>,
    _thread: JoinHandle<()>,
}

impl SyncRuntime {
    pub fn start(io: SyncRuntimeIo) -> Self {
        let (ctrl_tx, ctrl_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("bluelink-sync".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("bluelink-sync-worker")
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        io.emit(SyncUiEvent::Error(format!("tokio runtime: {e}")));
                        io.emit(SyncUiEvent::Status(SyncStatus::Error));
                        return;
                    }
                };
                rt.block_on(runtime_loop(io, ctrl_rx));
            })
            .expect("spawn sync runtime thread");
        Self {
            ctrl_tx,
            _thread: thread,
        }
    }

    pub fn set_vault(&self, path: Option<PathBuf>) {
        let _ = self.ctrl_tx.send(Control::SetVault(path));
    }
}

impl Drop for SyncRuntime {
    fn drop(&mut self) {
        let _ = self.ctrl_tx.send(Control::Shutdown);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn emit_log(io: &SyncRuntimeIo, text: impl Into<String>) {
    io.emit(SyncUiEvent::Log {
        ts_ms: now_ms(),
        text: text.into(),
    });
}

fn emit_trusted(io: &SyncRuntimeIo, root: &Path) {
    match TrustStore::load(root) {
        Ok(store) => io.emit(SyncUiEvent::Trusted(store.list().to_vec())),
        Err(e) => io.emit(SyncUiEvent::Error(format!("trust store: {e}"))),
    }
}

async fn poll_cmd(rx: &Receiver<SyncUiCommand>) -> Option<SyncUiCommand> {
    loop {
        match rx.try_recv() {
            Ok(cmd) => return Some(cmd),
            Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(25)).await,
            Err(TryRecvError::Disconnected) => return None,
        }
    }
}

async fn poll_ctrl(rx: &Receiver<Control>) -> Control {
    loop {
        match rx.try_recv() {
            Ok(c) => return c,
            Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(25)).await,
            Err(TryRecvError::Disconnected) => return Control::Shutdown,
        }
    }
}

async fn start_ble(io: &SyncRuntimeIo) -> Result<BluezPeripheral, String> {
    emit_log(io, "starting BlueZ peripheral…");
    BluezPeripheral::start(BluezPeripheralConfig {
        local_name: "BlueLink".into(),
        pairing_open: false,
    })
    .await
    .map_err(|e| format!("BLE adapter unavailable: {e}"))
}

struct Session {
    root: PathBuf,
    peripheral: Option<BluezPeripheral>,
    ble_failed: bool,
}

async fn apply_vault(
    io: &SyncRuntimeIo,
    path: Option<PathBuf>,
    cancel: &AtomicBool,
) -> Option<Session> {
    cancel.store(true, Ordering::SeqCst);
    cancel.store(false, Ordering::SeqCst);

    let Some(root) = path else {
        io.emit(SyncUiEvent::Status(SyncStatus::Idle));
        io.emit(SyncUiEvent::Trusted(Vec::new()));
        return None;
    };

    emit_log(io, format!("vault {}", root.display()));
    emit_trusted(io, &root);

    match start_ble(io).await {
        Ok(p) => {
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
            emit_log(io, "BLE advertising (waiting for phone)");
            Some(Session {
                root,
                peripheral: Some(p),
                ble_failed: false,
            })
        }
        Err(msg) => {
            io.emit(SyncUiEvent::Error(msg.clone()));
            io.emit(SyncUiEvent::Status(SyncStatus::Error));
            emit_log(io, msg);
            Some(Session {
                root,
                peripheral: None,
                ble_failed: true,
            })
        }
    }
}

fn revoke_device(io: &SyncRuntimeIo, root: &Path, device_id: &str) {
    match TrustStore::load(root) {
        Ok(mut store) => match store.revoke(device_id) {
            Ok(true) => {
                emit_log(io, format!("revoked {device_id}"));
                emit_trusted(io, root);
            }
            Ok(false) => {
                io.emit(SyncUiEvent::Error(format!("unknown device {device_id}")));
            }
            Err(e) => io.emit(SyncUiEvent::Error(format!("revoke failed: {e}"))),
        },
        Err(e) => io.emit(SyncUiEvent::Error(format!("trust store: {e}"))),
    }
}

fn handle_cmd_no_ble(io: &SyncRuntimeIo, root: &Path, cmd: SyncUiCommand, cancel: &AtomicBool) {
    match cmd {
        SyncUiCommand::ShowCode => {
            io.emit(SyncUiEvent::Error(
                "BLE adapter unavailable — cannot show pairing code".into(),
            ));
            io.emit(SyncUiEvent::Status(SyncStatus::Error));
        }
        SyncUiCommand::Revoke(id) => revoke_device(io, root, &id),
        SyncUiCommand::Cancel => {
            cancel.store(true, Ordering::SeqCst);
            cancel.store(false, Ordering::SeqCst);
        }
    }
}

async fn run_pairing(
    io: &SyncRuntimeIo,
    peri: &mut BluezPeripheral,
    root: &Path,
    cancel: &Arc<AtomicBool>,
) {
    cancel.store(false, Ordering::SeqCst);

    let session = match host_session() {
        Ok(s) => s,
        Err(e) => {
            io.emit(SyncUiEvent::Error(format!("pairing code: {e}")));
            return;
        }
    };

    peri.set_pairing_open(true);
    io.emit(SyncUiEvent::Status(SyncStatus::Pairing));
    io.emit(SyncUiEvent::Pairing {
        code: session.code.clone(),
        expires_at_ms: session.expires_at,
        attempts_left: session.attempts_left,
    });
    emit_log(
        io,
        format!(
            "pairing code {} ({}s, {} attempts)",
            session.code,
            PAIRING_TTL_MS / 1000,
            session.attempts_left
        ),
    );

    let mut attempts = session.attempts_left;
    let code = session.code.clone();
    let expires = session.expires_at;

    let vault = match Vault::open(root) {
        Ok(v) => v,
        Err(e) => {
            peri.set_pairing_open(false);
            io.emit(SyncUiEvent::Error(format!("open vault: {e}")));
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
            return;
        }
    };

    let ttl = expires.saturating_sub(now_ms()).max(1);
    let outcome = tokio::select! {
        result = host_accept(peri, &vault, &code, expires, &mut attempts) => Some(result),
        _ = tokio::time::sleep(Duration::from_millis(ttl)) => None,
        _ = wait_cancel_or_expiry(cancel, expires) => None,
    };

    peri.set_pairing_open(false);

    match outcome {
        Some(Ok(device)) => {
            emit_log(
                io,
                format!("paired {} ({})", device.name, device.device_id),
            );
            emit_trusted(io, root);
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
        }
        Some(Err(e)) => {
            io.emit(SyncUiEvent::Pairing {
                code,
                expires_at_ms: expires,
                attempts_left: attempts,
            });
            io.emit(SyncUiEvent::Error(format!("pairing failed: {e}")));
            emit_log(io, format!("pairing failed: {e}"));
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
        }
        None => {
            if cancel.load(Ordering::Relaxed) {
                emit_log(io, "pairing cancelled");
            } else {
                emit_log(io, "pairing expired");
                io.emit(SyncUiEvent::Error("pairing code expired".into()));
            }
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
        }
    }
    cancel.store(false, Ordering::SeqCst);
}

async fn wait_cancel_or_expiry(cancel: &AtomicBool, expires: u64) {
    while !cancel.load(Ordering::Relaxed) && now_ms() <= expires {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn run_responder(
    peri: &mut BluezPeripheral,
    root: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    cancel.store(false, Ordering::SeqCst);
    let mut vault = Vault::open(root).map_err(|e| e.to_string())?;
    let unsaved = HashSet::new();
    let opts = SyncOptions {
        role: Role::Responder,
        unsaved: &unsaved,
        cancel: Some(cancel.as_ref()),
    };
    run_sync(peri, &mut vault, opts)
        .await
        .map_err(|e| e.to_string())
}

async fn runtime_loop(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut session: Option<Session> = None;

    io.emit(SyncUiEvent::Status(SyncStatus::Idle));

    loop {
        let Some(sess) = session.as_mut() else {
            tokio::select! {
                ctrl = poll_ctrl(&ctrl_rx) => {
                    match ctrl {
                        Control::Shutdown => {
                            emit_log(&io, "sync runtime stopped");
                            return;
                        }
                        Control::SetVault(path) => {
                            session = apply_vault(&io, path, &cancel).await;
                        }
                    }
                }
                cmd = poll_cmd(&io.cmd_rx) => {
                    match cmd {
                        None => return,
                        Some(SyncUiCommand::Cancel) => {}
                        Some(other) => {
                            io.emit(SyncUiEvent::Error(format!(
                                "open a vault before {other:?}"
                            )));
                        }
                    }
                }
            }
            continue;
        };

        if sess.ble_failed || sess.peripheral.is_none() {
            tokio::select! {
                ctrl = poll_ctrl(&ctrl_rx) => {
                    match ctrl {
                        Control::Shutdown => return,
                        Control::SetVault(path) => {
                            session = apply_vault(&io, path, &cancel).await;
                        }
                    }
                }
                cmd = poll_cmd(&io.cmd_rx) => {
                    let Some(cmd) = cmd else { return };
                    handle_cmd_no_ble(&io, &sess.root, cmd, &cancel);
                }
            }
            continue;
        }

        let root = sess.root.clone();
        let peri = sess.peripheral.as_mut().expect("checked");

        tokio::select! {
            ctrl = poll_ctrl(&ctrl_rx) => {
                match ctrl {
                    Control::Shutdown => {
                        emit_log(&io, "sync runtime stopped");
                        return;
                    }
                    Control::SetVault(path) => {
                        cancel.store(true, Ordering::SeqCst);
                        session = apply_vault(&io, path, &cancel).await;
                    }
                }
            }
            cmd = poll_cmd(&io.cmd_rx) => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    SyncUiCommand::ShowCode => {
                        cancel.store(true, Ordering::SeqCst);
                        // Drop in-flight responder wait by falling through after cancel;
                        // ShowCode runs pairing on the same peripheral next iteration…
                        // Run pairing immediately instead:
                        cancel.store(false, Ordering::SeqCst);
                        run_pairing(&io, peri, &root, &cancel).await;
                    }
                    SyncUiCommand::Revoke(id) => {
                        revoke_device(&io, &root, &id);
                    }
                    SyncUiCommand::Cancel => {
                        cancel.store(true, Ordering::SeqCst);
                        cancel.store(false, Ordering::SeqCst);
                    }
                }
            }
            result = run_responder(peri, &root, &cancel) => {
                match result {
                    Ok(()) => {
                        emit_log(&io, "sync completed");
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                    }
                    Err(msg) => {
                        let cancelled = msg.to_lowercase().contains("cancel");
                        if cancelled {
                            emit_log(&io, "sync cancelled");
                        } else {
                            io.emit(SyncUiEvent::Error(msg.clone()));
                            emit_log(&io, format!("sync error: {msg}"));
                        }
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                    }
                }
                cancel.store(false, Ordering::SeqCst);
            }
        }
    }
}
