use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const APP_VERSION: &str = "0.1.0";
pub const PUT_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_NOTES: usize = 5000;
pub const MANIFEST_CHUNK: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteEntry {
    pub rel_path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Hello {
        protocol: u16,
        device_id: String,
        app_version: String,
    },
    HelloAck {
        protocol: u16,
        device_id: String,
        app_version: String,
    },
    Manifest {
        part: u16,
        total: u16,
        notes: Vec<NoteEntry>,
    },
    Get {
        rel_path: String,
        sha256: String,
    },
    Put {
        rel_path: String,
        sha256: String,
        offset: u64,
        last: bool,
        data: Vec<u8>,
    },
    Delete {
        rel_path: String,
        expected_sha256: String,
    },
    Commit,
    /// Responder acknowledges Commit; both sides may then persist sync_state.
    CommitAck,
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    Untrusted,
    ProtocolMismatch,
    InvalidPath,
    TooLarge,
    IntegrityError,
    TooManyNotes,
    Cancelled,
    Other,
}

pub fn encode(msg: &Message) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(msg)
}

pub fn decode(bytes: &[u8]) -> Result<Message, postcard::Error> {
    postcard::from_bytes(bytes)
}
