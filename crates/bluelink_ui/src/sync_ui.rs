//! Sync UI contract (Agent C / Agent D).
//!
//! Desktop owns the BLE + `run_sync` runtime and pushes [`SyncUiEvent`]s here.
//! UI sends [`SyncUiCommand`]s; it never touches BlueZ or tokio sync sessions.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender};

/// Re-exports for Devices panel (countdown / attempts / list rows).
pub use bluelink_sync::{PAIRING_ATTEMPTS, PAIRING_TTL_MS, TrustedDevice};

/// High-level sync status string source for the Sync panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SyncStatus {
    /// Runtime not started / vault not open.
    #[default]
    Idle,
    /// Advertising; waiting for phone to connect (desktop is Responder).
    Waiting,
    /// Pairing code shown; waiting for join.
    Pairing,
    /// Noise handshake / session start.
    Connecting,
    /// File transfer progress (optional; may stay 0/0 if runtime has no counters yet).
    Transfer { current: u32, total: u32 },
    /// Last operation failed; see [`SyncUiEvent::Error`] / log.
    Error,
}

/// Runtime → UI. Drain via [`SyncUiHandle::drain_events`] each frame.
#[derive(Debug, Clone)]
pub enum SyncUiEvent {
    Status(SyncStatus),
    Pairing {
        code: String,
        /// Unix milliseconds (same clock as `PAIRING_TTL_MS`).
        expires_at_ms: u64,
        attempts_left: u8,
    },
    /// Full trusted-device list after load / pair / revoke.
    Trusted(Vec<TrustedDevice>),
    Log {
        /// Unix milliseconds — format with existing `hhmm_*` helpers (24h).
        ts_ms: u64,
        text: String,
    },
    Error(String),
}

/// UI → runtime. Desktop does **not** expose a Sync button (phone initiates).
#[derive(Debug, Clone)]
pub enum SyncUiCommand {
    /// Show a new 6-digit pairing code (TTL [`PAIRING_TTL_MS`], [`PAIRING_ATTEMPTS`] tries).
    ShowCode,
    Revoke(String),
    /// Cancel in-flight pairing or responder sync.
    Cancel,
}

/// UI half of the bridge — store on [`crate::App`], use from Devices/Sync panels.
#[derive(Clone)]
pub struct SyncUiHandle {
    cmd_tx: Sender<SyncUiCommand>,
    events: Arc<Mutex<VecDeque<SyncUiEvent>>>,
    wake: Arc<AtomicBool>,
}

impl SyncUiHandle {
    pub fn send(&self, cmd: SyncUiCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Take all pending events (Agent D: update status / log / trusted list).
    /// Also clears the wake flag.
    pub fn drain_events(&self) -> Vec<SyncUiEvent> {
        self.wake.store(false, Ordering::SeqCst);
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// True if runtime pushed events since last drain / take_wake.
    /// DesktopApp uses this for `request_repaint`; Agent D should prefer `drain_events`.
    pub fn take_wake(&self) -> bool {
        self.wake.swap(false, Ordering::SeqCst)
    }

    pub fn wake_flag(&self) -> Arc<AtomicBool> {
        self.wake.clone()
    }
}

/// Runtime half — owned by `apps/desktop` SyncRuntime only.
pub struct SyncRuntimeIo {
    pub cmd_rx: Receiver<SyncUiCommand>,
    pub events: Arc<Mutex<VecDeque<SyncUiEvent>>>,
    pub wake: Arc<AtomicBool>,
}

impl SyncRuntimeIo {
    pub fn emit(&self, event: SyncUiEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(event);
        self.wake.store(true, Ordering::SeqCst);
    }
}

/// Split UI handle + runtime I/O. Call once from desktop `main`.
pub fn channel() -> (SyncUiHandle, SyncRuntimeIo) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let wake = Arc::new(AtomicBool::new(false));
    (
        SyncUiHandle {
            cmd_tx,
            events: events.clone(),
            wake: wake.clone(),
        },
        SyncRuntimeIo {
            cmd_rx,
            events,
            wake,
        },
    )
}
