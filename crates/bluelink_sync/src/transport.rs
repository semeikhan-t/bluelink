use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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

/// Length-prefixed frames over TCP — desktop Host↔Initiator demo without BLE/Android.
///
/// Wire: `u32` LE length + payload (max [`MAX_FRAME_BYTES`]).
pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    pub async fn connect(addr: impl tokio::net::ToSocketAddrs) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let _ = stream.set_nodelay(true);
        Ok(Self { stream })
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn send(&mut self, frame: &[u8]) -> Result<(), TransportError> {
        if frame.len() > MAX_FRAME_BYTES {
            return Err(TransportError::TooLarge);
        }
        let len = (frame.len() as u32).to_le_bytes();
        self.stream
            .write_all(&len)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        self.stream
            .write_all(frame)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        self.stream
            .flush()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                TransportError::Closed
            } else {
                TransportError::Io(e.to_string())
            }
        })?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(TransportError::TooLarge);
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                TransportError::Closed
            } else {
                TransportError::Io(e.to_string())
            }
        })?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn tcp_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut t = TcpTransport::new(stream);
            let got = t.recv().await.unwrap();
            assert_eq!(got, b"hello");
            t.send(b"world").await.unwrap();
        });
        let mut client = TcpTransport::connect(addr).await.unwrap();
        client.send(b"hello").await.unwrap();
        assert_eq!(client.recv().await.unwrap(), b"world");
        server.await.unwrap();
    }
}
