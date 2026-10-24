use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,

    #[error("transport I/O: {0}")]
    Io(String),

    #[error("message exceeds 64 KiB")]
    TooLarge,

    #[error("timeout")]
    Timeout,
}

#[derive(Debug, Error)]
pub enum PairingError {
    #[error("wrong pairing code")]
    WrongCode,

    #[error("pairing session expired")]
    Expired,

    #[error("peer is not in pairing mode")]
    NotInPairingMode,

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error(transparent)]
    Vault(#[from] bluelink_core::VaultError),
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("peer is not trusted")]
    Untrusted,

    #[error("protocol version mismatch")]
    ProtocolMismatch,

    #[error("invalid path")]
    InvalidPath,

    #[error("note too large")]
    TooLarge,

    #[error("integrity check failed")]
    IntegrityError,

    #[error("timeout")]
    Timeout,

    #[error("sync cancelled")]
    Cancelled,

    #[error("too many notes (max 5000)")]
    TooManyNotes,

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error(transparent)]
    Vault(#[from] bluelink_core::VaultError),
}
