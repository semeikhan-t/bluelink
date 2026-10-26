//! Tokio SyncRuntime: BlueZ peripheral host, or TCP Host/Initiator for local demo.
//!
//! - Default / BLE: ShowCode + Responder sync (phone / Initiator dials).
//! - `--sync-tcp ADDR`: TCP listen; same host UX without BlueZ.
//! - `--initiator --connect ADDR`: Join + Sync buttons; dials the host.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "bluez")]
use bluelink_ble::{BluezPeripheral, BluezPeripheralConfig};
use bluelink_core::Vault;
use bluelink_sync::{
    host_accept, host_session, join, run_sync, Role, SyncOptions, TcpTransport, TrustStore,
    PAIRING_TTL_MS,
};
use bluelink_ui::sync_ui::{SyncRuntimeIo, SyncStatus, SyncUiCommand, SyncUiEvent};
use tokio::net::TcpListener;

/// How the desktop sync worker talks to a peer.
#[derive(Debug, Clone)]
pub enum TransportMode {
    /// BlueZ GATT peripheral (default when feature `bluez` is on).
    Ble,
    /// Listen for TCP Initiator (demo without Android/BLE).
    TcpHost(SocketAddr),
    /// Dial TCP host; UI shows Join + Sync.
    TcpInitiator(SocketAddr),
}

enum Control {
    SetVault(Option<PathBuf>),
    Shutdown,
}

/// Background sync worker. Lifetime owned by [`super::DesktopApp`].
pub struct SyncRuntime {
    ctrl_tx: mpsc::Sender<Control>,
    _thread: JoinHandle<()>,
}

impl SyncRuntime {
    pub fn start(io: SyncRuntimeIo, mode: TransportMode) -> Self {
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
                rt.block_on(runtime_loop(io, ctrl_rx, mode));
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

async fn wait_cancel_or_expiry(cancel: &AtomicBool, expires: u64) {
    while !cancel.load(Ordering::Relaxed) && now_ms() <= expires {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn runtime_loop(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>, mode: TransportMode) {
    match mode {
        TransportMode::Ble => {
            #[cfg(feature = "bluez")]
            {
                ble_host_loop(io, ctrl_rx).await;
            }
            #[cfg(not(feature = "bluez"))]
            {
                io.emit(SyncUiEvent::Error(
                    "desktop built without bluez — use --sync-tcp or --initiator --connect".into(),
                ));
                io.emit(SyncUiEvent::Status(SyncStatus::Error));
                idle_until_shutdown(io, ctrl_rx).await;
            }
        }
        TransportMode::TcpHost(addr) => tcp_host_loop(io, ctrl_rx, addr).await,
        TransportMode::TcpInitiator(addr) => tcp_initiator_loop(io, ctrl_rx, addr).await,
    }
}

async fn idle_until_shutdown(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>) {
    loop {
        match poll_ctrl(&ctrl_rx).await {
            Control::Shutdown => {
                emit_log(&io, "sync runtime stopped");
                return;
            }
            Control::SetVault(_) => {}
        }
    }
}

// --- TCP Host ----------------------------------------------------------------

struct PendingPair {
    code: String,
    expires_at: u64,
    attempts_left: u8,
}

async fn tcp_host_loop(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>, bind: SocketAddr) {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut root: Option<PathBuf> = None;
    let mut pending: Option<PendingPair> = None;

    let listener = match TcpListener::bind(bind).await {
        Ok(l) => {
            emit_log(&io, format!("TCP host listening on {bind}"));
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
            l
        }
        Err(e) => {
            let msg = format!("TCP bind {bind}: {e}");
            io.emit(SyncUiEvent::Error(msg.clone()));
            io.emit(SyncUiEvent::Status(SyncStatus::Error));
            emit_log(&io, msg);
            idle_until_shutdown(io, ctrl_rx).await;
            return;
        }
    };

    io.emit(SyncUiEvent::Status(SyncStatus::Idle));

    loop {
        tokio::select! {
            ctrl = poll_ctrl(&ctrl_rx) => {
                match ctrl {
                    Control::Shutdown => {
                        emit_log(&io, "sync runtime stopped");
                        return;
                    }
                    Control::SetVault(path) => {
                        cancel.store(true, Ordering::SeqCst);
                        cancel.store(false, Ordering::SeqCst);
                        pending = None;
                        root = path;
                        if let Some(r) = &root {
                            emit_log(&io, format!("vault {}", r.display()));
                            emit_trusted(&io, r);
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        } else {
                            io.emit(SyncUiEvent::Trusted(Vec::new()));
                            io.emit(SyncUiEvent::Status(SyncStatus::Idle));
                        }
                    }
                }
            }
            cmd = poll_cmd(&io.cmd_rx) => {
                let Some(cmd) = cmd else { return };
                let Some(r) = root.as_ref() else {
                    io.emit(SyncUiEvent::Error("open a vault first".into()));
                    continue;
                };
                match cmd {
                    SyncUiCommand::ShowCode => {
                        match host_session() {
                            Ok(s) => {
                                pending = Some(PendingPair {
                                    code: s.code.clone(),
                                    expires_at: s.expires_at,
                                    attempts_left: s.attempts_left,
                                });
                                io.emit(SyncUiEvent::Status(SyncStatus::Pairing));
                                io.emit(SyncUiEvent::Pairing {
                                    code: s.code.clone(),
                                    expires_at_ms: s.expires_at,
                                    attempts_left: s.attempts_left,
                                });
                                emit_log(
                                    &io,
                                    format!(
                                        "pairing code {} — Initiator should Join ({}s)",
                                        s.code,
                                        PAIRING_TTL_MS / 1000
                                    ),
                                );
                            }
                            Err(e) => io.emit(SyncUiEvent::Error(format!("pairing code: {e}"))),
                        }
                    }
                    SyncUiCommand::Revoke(id) => revoke_device(&io, r, &id),
                    SyncUiCommand::Cancel => {
                        cancel.store(true, Ordering::SeqCst);
                        cancel.store(false, Ordering::SeqCst);
                        pending = None;
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        emit_log(&io, "cancelled");
                    }
                    SyncUiCommand::EnsureBle => {
                        // TCP host has no BlueZ radio.
                    }
                    SyncUiCommand::Join(_) | SyncUiCommand::Sync => {
                        io.emit(SyncUiEvent::Error(
                            "this process is the Host — run Initiator with --initiator --connect"
                                .into(),
                        ));
                    }
                }
            }
            accept = listener.accept() => {
                let Ok((stream, peer)) = accept else {
                    continue;
                };
                let Some(r) = root.clone() else {
                    emit_log(&io, format!("ignored connect from {peer} (no vault)"));
                    continue;
                };
                let _ = stream.set_nodelay(true);
                let mut transport = TcpTransport::new(stream);
                emit_log(&io, format!("TCP peer {peer}"));

                if let Some(mut p) = pending.take() {
                    if now_ms() > p.expires_at {
                        emit_log(&io, "pairing expired before accept");
                        io.emit(SyncUiEvent::Error("pairing code expired".into()));
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        continue;
                    }
                    let vault = match Vault::open(&r) {
                        Ok(v) => v,
                        Err(e) => {
                            io.emit(SyncUiEvent::Error(format!("open vault: {e}")));
                            continue;
                        }
                    };
                    io.emit(SyncUiEvent::Status(SyncStatus::Connecting));
                    let ttl = p.expires_at.saturating_sub(now_ms()).max(1);
                    let outcome = tokio::select! {
                        result = host_accept(
                            &mut transport,
                            &vault,
                            &p.code,
                            p.expires_at,
                            &mut p.attempts_left,
                        ) => Some(result),
                        _ = tokio::time::sleep(Duration::from_millis(ttl)) => None,
                        _ = wait_cancel_or_expiry(&cancel, p.expires_at) => None,
                    };
                    match outcome {
                        Some(Ok(device)) => {
                            emit_log(
                                &io,
                                format!("paired {} ({})", device.name, device.device_id),
                            );
                            emit_trusted(&io, &r);
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        }
                        Some(Err(e)) => {
                            io.emit(SyncUiEvent::Pairing {
                                code: p.code.clone(),
                                expires_at_ms: p.expires_at,
                                attempts_left: p.attempts_left,
                            });
                            pending = Some(p);
                            io.emit(SyncUiEvent::Error(format!("pairing failed: {e}")));
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        }
                        None => {
                            emit_log(&io, "pairing cancelled or expired");
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        }
                    }
                    cancel.store(false, Ordering::SeqCst);
                } else {
                    io.emit(SyncUiEvent::Status(SyncStatus::Connecting));
                    cancel.store(false, Ordering::SeqCst);
                    let mut vault = match Vault::open(&r) {
                        Ok(v) => v,
                        Err(e) => {
                            io.emit(SyncUiEvent::Error(format!("open vault: {e}")));
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                            continue;
                        }
                    };
                    let unsaved = HashSet::new();
                    let opts = SyncOptions {
                        role: Role::Responder,
                        unsaved: &unsaved,
                        cancel: Some(cancel.as_ref()),
                    };
                    match run_sync(&mut transport, &mut vault, opts).await {
                        Ok(()) => {
                            emit_log(&io, "sync completed");
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.to_lowercase().contains("cancel") {
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
}

// --- TCP Initiator -----------------------------------------------------------

async fn tcp_initiator_loop(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>, connect: SocketAddr) {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut root: Option<PathBuf> = None;

    emit_log(&io, format!("TCP initiator → {connect}"));
    io.emit(SyncUiEvent::Status(SyncStatus::Idle));

    loop {
        tokio::select! {
            ctrl = poll_ctrl(&ctrl_rx) => {
                match ctrl {
                    Control::Shutdown => {
                        emit_log(&io, "sync runtime stopped");
                        return;
                    }
                    Control::SetVault(path) => {
                        cancel.store(true, Ordering::SeqCst);
                        cancel.store(false, Ordering::SeqCst);
                        root = path;
                        if let Some(r) = &root {
                            emit_log(&io, format!("vault {}", r.display()));
                            emit_trusted(&io, r);
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        } else {
                            io.emit(SyncUiEvent::Trusted(Vec::new()));
                            io.emit(SyncUiEvent::Status(SyncStatus::Idle));
                        }
                    }
                }
            }
            cmd = poll_cmd(&io.cmd_rx) => {
                let Some(cmd) = cmd else { return };
                let Some(r) = root.clone() else {
                    io.emit(SyncUiEvent::Error("open a vault first".into()));
                    continue;
                };
                match cmd {
                    SyncUiCommand::Join(code) => {
                        cancel.store(false, Ordering::SeqCst);
                        io.emit(SyncUiEvent::Status(SyncStatus::Pairing));
                        emit_log(&io, format!("joining with code {code}…"));
                        let mut transport = match TcpTransport::connect(connect).await {
                            Ok(t) => t,
                            Err(e) => {
                                io.emit(SyncUiEvent::Error(format!("connect {connect}: {e}")));
                                io.emit(SyncUiEvent::Status(SyncStatus::Error));
                                continue;
                            }
                        };
                        let vault = match Vault::open(&r) {
                            Ok(v) => v,
                            Err(e) => {
                                io.emit(SyncUiEvent::Error(format!("open vault: {e}")));
                                continue;
                            }
                        };
                        match join(&mut transport, &vault, &code).await {
                            Ok(peer) => {
                                emit_log(
                                    &io,
                                    format!("paired with {} ({})", peer.name, peer.device_id),
                                );
                                emit_trusted(&io, &r);
                                io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                            }
                            Err(e) => {
                                io.emit(SyncUiEvent::Error(format!("join failed: {e}")));
                                emit_log(&io, format!("join failed: {e}"));
                                io.emit(SyncUiEvent::Status(SyncStatus::Error));
                            }
                        }
                    }
                    SyncUiCommand::Sync => {
                        cancel.store(false, Ordering::SeqCst);
                        io.emit(SyncUiEvent::Status(SyncStatus::Connecting));
                        emit_log(&io, "sync (Initiator)…");
                        let mut transport = match TcpTransport::connect(connect).await {
                            Ok(t) => t,
                            Err(e) => {
                                io.emit(SyncUiEvent::Error(format!("connect {connect}: {e}")));
                                io.emit(SyncUiEvent::Status(SyncStatus::Error));
                                continue;
                            }
                        };
                        let mut vault = match Vault::open(&r) {
                            Ok(v) => v,
                            Err(e) => {
                                io.emit(SyncUiEvent::Error(format!("open vault: {e}")));
                                continue;
                            }
                        };
                        let unsaved = HashSet::new();
                        let opts = SyncOptions {
                            role: Role::Initiator,
                            unsaved: &unsaved,
                            cancel: Some(cancel.as_ref()),
                        };
                        match run_sync(&mut transport, &mut vault, opts).await {
                            Ok(()) => {
                                emit_log(&io, "sync completed");
                                io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                io.emit(SyncUiEvent::Error(msg.clone()));
                                emit_log(&io, format!("sync error: {msg}"));
                                io.emit(SyncUiEvent::Status(SyncStatus::Error));
                            }
                        }
                        cancel.store(false, Ordering::SeqCst);
                    }
                    SyncUiCommand::Revoke(id) => revoke_device(&io, &r, &id),
                    SyncUiCommand::Cancel => {
                        cancel.store(true, Ordering::SeqCst);
                        cancel.store(false, Ordering::SeqCst);
                        emit_log(&io, "cancelled");
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                    }
                    SyncUiCommand::ShowCode => {
                        io.emit(SyncUiEvent::Error(
                            "Initiator cannot Show code — use the Host window".into(),
                        ));
                    }
                    SyncUiCommand::EnsureBle => {
                        // Initiator has no BlueZ peripheral.
                    }
                }
            }
        }
    }
}

// --- BLE Host (existing path) ------------------------------------------------

#[cfg(feature = "bluez")]
async fn start_ble(io: &SyncRuntimeIo) -> Result<BluezPeripheral, String> {
    emit_log(io, "starting BlueZ peripheral…");
    eprintln!("bluelink: start_ble begin");
    let t0 = std::time::Instant::now();
    let result = BluezPeripheral::start(BluezPeripheralConfig {
        local_name: "BlueLink".into(),
        pairing_open: false,
    })
    .await
    .map_err(|e| match e {
        bluelink_sync::TransportError::Timeout => {
            "BLE adapter start timed out (15s) — is bluetoothd stuck?".into()
        }
        other => format!("BLE adapter unavailable: {other}"),
    });
    eprintln!("bluelink: start_ble end {}ms", t0.elapsed().as_millis());
    result
}

#[cfg(feature = "bluez")]
struct BleSession {
    root: PathBuf,
    peripheral: Option<BluezPeripheral>,
    ble_failed: bool,
}

/// Bind vault + trust list only — no BlueZ/D-Bus. Radio starts via [`ensure_ble_radio`].
#[cfg(feature = "bluez")]
async fn bind_vault_ble(
    io: &SyncRuntimeIo,
    path: Option<PathBuf>,
    cancel: &AtomicBool,
) -> Option<BleSession> {
    cancel.store(true, Ordering::SeqCst);
    cancel.store(false, Ordering::SeqCst);

    let Some(root) = path else {
        io.emit(SyncUiEvent::Status(SyncStatus::Idle));
        io.emit(SyncUiEvent::Trusted(Vec::new()));
        return None;
    };

    emit_log(io, format!("vault {} (BLE idle until Sync)", root.display()));
    emit_trusted(io, &root);
    io.emit(SyncUiEvent::Status(SyncStatus::Idle));
    Some(BleSession {
        root,
        peripheral: None,
        ble_failed: false,
    })
}

/// Start GATT peripheral if this session has a vault and no live radio yet.
#[cfg(feature = "bluez")]
async fn ensure_ble_radio(io: &SyncRuntimeIo, sess: &mut BleSession) -> bool {
    if sess.peripheral.is_some() {
        return true;
    }
    match start_ble(io).await {
        Ok(p) => {
            sess.peripheral = Some(p);
            sess.ble_failed = false;
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
            emit_log(io, "BLE advertising (waiting for peer)");
            true
        }
        Err(msg) => {
            sess.ble_failed = true;
            io.emit(SyncUiEvent::Error(msg.clone()));
            io.emit(SyncUiEvent::Status(SyncStatus::Error));
            emit_log(io, format!("{msg} — tip: --sync-tcp 127.0.0.1:7878"));
            false
        }
    }
}

/// Re-bind vault and start radio (link restart / recovery after transport death).
#[cfg(feature = "bluez")]
async fn apply_vault_ble(
    io: &SyncRuntimeIo,
    path: Option<PathBuf>,
    cancel: &AtomicBool,
) -> Option<BleSession> {
    let mut session = bind_vault_ble(io, path, cancel).await?;
    let _ = ensure_ble_radio(io, &mut session).await;
    Some(session)
}

#[cfg(feature = "bluez")]
async fn run_pairing_ble(
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

#[cfg(feature = "bluez")]
async fn run_responder_ble(
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

#[cfg(feature = "bluez")]
fn ble_transport_dead(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("closed") || m.contains("timeout")
}

#[cfg(feature = "bluez")]
async fn ble_host_loop(io: SyncRuntimeIo, ctrl_rx: Receiver<Control>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut session: Option<BleSession> = None;
    // Prevent Closed/error tight-loops from flooding the UI wake flag (KDE ANR).
    let mut restart_backoff = Duration::from_millis(250);

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
                            // Vault only — no BlueZ until EnsureBle / ShowCode.
                            session = bind_vault_ble(&io, path, &cancel).await;
                            restart_backoff = Duration::from_millis(250);
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

        // Vault bound, radio not up yet (or last start failed).
        if sess.peripheral.is_none() {
            tokio::select! {
                ctrl = poll_ctrl(&ctrl_rx) => {
                    match ctrl {
                        Control::Shutdown => return,
                        Control::SetVault(path) => {
                            session = bind_vault_ble(&io, path, &cancel).await;
                            restart_backoff = Duration::from_millis(250);
                        }
                    }
                }
                cmd = poll_cmd(&io.cmd_rx) => {
                    let Some(cmd) = cmd else { return };
                    match cmd {
                        SyncUiCommand::EnsureBle => {
                            let _ = ensure_ble_radio(&io, sess).await;
                            restart_backoff = Duration::from_millis(250);
                        }
                        SyncUiCommand::ShowCode => {
                            if ensure_ble_radio(&io, sess).await {
                                restart_backoff = Duration::from_millis(250);
                                let root = sess.root.clone();
                                let peri = sess.peripheral.as_mut().expect("just started");
                                cancel.store(false, Ordering::SeqCst);
                                run_pairing_ble(&io, peri, &root, &cancel).await;
                            }
                        }
                        SyncUiCommand::Revoke(id) => revoke_device(&io, &sess.root, &id),
                        SyncUiCommand::Cancel => {
                            cancel.store(true, Ordering::SeqCst);
                        }
                        SyncUiCommand::Join(_) | SyncUiCommand::Sync => {
                            io.emit(SyncUiEvent::Error(
                                "BLE host cannot Join/Sync — use --initiator --connect".into(),
                            ));
                        }
                    }
                }
            }
            continue;
        }

        // GATT worker gone → recv returns Closed instantly. Restart with backoff
        // instead of spinning emit(Error) on the UI channel.
        if sess.peripheral.as_ref().is_some_and(|p| !p.link_alive()) {
            let root = sess.root.clone();
            emit_log(&io, "BLE link worker exited — restarting peripheral");
            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
            tokio::time::sleep(restart_backoff).await;
            restart_backoff = (restart_backoff * 2).min(Duration::from_secs(5));
            session = apply_vault_ble(&io, Some(root), &cancel).await;
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
                        // Drop radio with vault change; Sync screen will EnsureBle again.
                        session = bind_vault_ble(&io, path, &cancel).await;
                        restart_backoff = Duration::from_millis(250);
                    }
                }
            }
            cmd = poll_cmd(&io.cmd_rx) => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    SyncUiCommand::EnsureBle => {
                        // Already advertising.
                    }
                    SyncUiCommand::ShowCode => {
                        cancel.store(false, Ordering::SeqCst);
                        run_pairing_ble(&io, peri, &root, &cancel).await;
                    }
                    SyncUiCommand::Revoke(id) => {
                        revoke_device(&io, &root, &id);
                    }
                    SyncUiCommand::Cancel => {
                        cancel.store(true, Ordering::SeqCst);
                    }
                    SyncUiCommand::Join(_) | SyncUiCommand::Sync => {
                        io.emit(SyncUiEvent::Error(
                            "BLE host cannot Join/Sync — use --initiator --connect".into(),
                        ));
                    }
                }
            }
            result = run_responder_ble(peri, &root, &cancel) => {
                match result {
                    Ok(()) => {
                        emit_log(&io, "sync completed");
                        io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        restart_backoff = Duration::from_millis(250);
                        // Brief yield so a reconnecting peer cannot spin the select.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(msg) => {
                        let cancelled = msg.to_lowercase().contains("cancel");
                        if cancelled {
                            emit_log(&io, "sync cancelled");
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                        } else if ble_transport_dead(&msg) {
                            emit_log(&io, format!("BLE transport: {msg} — restarting"));
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                            tokio::time::sleep(restart_backoff).await;
                            restart_backoff = (restart_backoff * 2).min(Duration::from_secs(5));
                            session = apply_vault_ble(&io, Some(root), &cancel).await;
                        } else {
                            io.emit(SyncUiEvent::Error(msg.clone()));
                            emit_log(&io, format!("sync error: {msg}"));
                            io.emit(SyncUiEvent::Status(SyncStatus::Waiting));
                            // Back off so crypto/protocol noise cannot flood UI events.
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
                cancel.store(false, Ordering::SeqCst);
            }
        }
    }
}
