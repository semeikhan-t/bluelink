use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::TransportError;

pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[async_trait]
pub trait Transport: Send {
    async fn send(&mut self, frame: &[u8]) -> Result<(), TransportError>;
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError>;
}

/// Two linked in-memory endpoints for protocol tests without Bluetooth.
pub struct Loopback {
    tx: mpsc::Sender<Vec<u8>>,
    rx: mpsc::Receiver<Vec<u8>>,
}

impl Loopback {
    pub fn pair() -> (Self, Self) {
        let (a_tx, b_rx) = mpsc::channel(32);
        let (b_tx, a_rx) = mpsc::channel(32);
        (
            Self {
                tx: a_tx,
                rx: a_rx,
            },
            Self {
                tx: b_tx,
                rx: b_rx,
            },
        )
    }
}

#[async_trait]
impl Transport for Loopback {
    async fn send(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        if frame.len() > MAX_FRAME_BYTES {
            return Err(TransportError::TooLarge);
        }
        self.tx
            .send(frame.to_vec())
            .await
            .map_err(|_| TransportError::Closed)
    }

    async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        self.rx.recv().await.ok_or(TransportError::Closed)
    }
}
