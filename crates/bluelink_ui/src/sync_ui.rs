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

/// UI → runtime.
///
/// Host (BLE or `--sync-tcp`): ShowCode / Revoke / Cancel.
/// Initiator (`--initiator --connect`): Join / Sync / Revoke / Cancel.
#[derive(Debug, Clone)]
pub enum SyncUiCommand {
    /// Show a new 6-digit pairing code (TTL [`PAIRING_TTL_MS`], [`PAIRING_ATTEMPTS`] tries).
    ShowCode,
    /// Initiator: dial peer and complete SPAKE2 join with this 6-digit code.
    Join(String),
    /// Initiator: dial peer and run sync as Initiator (after trust exists).
    Sync,
    Revoke(String),
    /// Cancel in-flight pairing or sync.
    Cancel,
}

/// UI half of the bridge — store on [`crate::App`], use from Devices/Sync panels.
#[derive(Clone)]
pub struct SyncUiHandle {
    cmd_tx: Sender<SyncUiCommand>,
    events: Arc<Mutex<VecDeque<SyncUiEvent>>>,
    wake: Arc<AtomicBool>,
    /// True when this process is the TCP/phone-side Initiator (shows Join + Sync).
    initiator: bool,
}

impl SyncUiHandle {
    pub fn is_initiator(&self) -> bool {
        self.initiator
    }

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

/// Cap so a tight sync-runtime error loop cannot grow unbounded and starve the UI.
const EVENT_QUEUE_CAP: usize = 64;

/// Runtime half — owned by `apps/desktop` SyncRuntime only.
pub struct SyncRuntimeIo {
    pub cmd_rx: Receiver<SyncUiCommand>,
    pub events: Arc<Mutex<VecDeque<SyncUiEvent>>>,
    pub wake: Arc<AtomicBool>,
}

impl SyncRuntimeIo {
    pub fn emit(&self, event: SyncUiEvent) {
        let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= EVENT_QUEUE_CAP {
            q.pop_front();
        }
        q.push_back(event);
        self.wake.store(true, Ordering::SeqCst);
    }
}

/// Split UI handle + runtime I/O. Call once from desktop `main`.
pub fn channel() -> (SyncUiHandle, SyncRuntimeIo) {
    channel_with_role(false)
}

/// Like [`channel`], with Initiator UI enabled when `initiator` is true.
pub fn channel_with_role(initiator: bool) -> (SyncUiHandle, SyncRuntimeIo) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let wake = Arc::new(AtomicBool::new(false));
    (
        SyncUiHandle {
            cmd_tx,
            events: events.clone(),
            wake: wake.clone(),
            initiator,
        },
        SyncRuntimeIo {
            cmd_rx,
            events,
            wake,
        },
    )
}
