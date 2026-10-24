//! BlueLink sync protocol: SPAKE2 pairing, Noise XX, 3-way sync over [`Transport`].

mod conflict;
mod error;
mod noise_session;
mod pairing;
mod plan;
mod proto;
mod session;
mod sync_state;
mod transport;
mod trust;

pub use error::{PairingError, SyncError, TransportError};
pub use pairing::{generate_code, host_accept, host_session, join, HostSessionInfo, PAIRING_ATTEMPTS, PAIRING_TTL_MS};
pub use plan::{build_plan, Action};
pub use session::{run_sync, Role, SyncOptions};
pub use sync_state::SyncState;
pub use transport::{Loopback, Transport, MAX_FRAME_BYTES};
pub use trust::{TrustStore, TrustedDevice};

pub use bluelink_core;
