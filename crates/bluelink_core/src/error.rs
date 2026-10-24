use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("path is not a directory")]
    NotADirectory,

    #[error("permission denied")]
    PermissionDenied,

    #[error("corrupt .bluelink metadata")]
    CorruptMeta,

    #[error("note not found")]
    NotFound,

    #[error("invalid path")]
    InvalidPath,

    #[error("note exceeds 1 MiB")]
    TooLarge,

    #[error("note is not valid UTF-8")]
    NotUtf8,

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl VaultError {
    pub(crate) fn from_io(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::NotFound => Self::NotFound,
            _ => Self::Io(err),
        }
    }
}
