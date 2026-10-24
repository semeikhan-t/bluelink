use base64::Engine;
use snow::{Builder, TransportState};

use bluelink_core::DeviceMeta;

use crate::error::SyncError;
use crate::transport::{Transport, MAX_FRAME_BYTES};
use crate::trust::TrustStore;

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

pub struct NoiseSession {
    state: TransportState,
    pub remote_public_key_b64: String,
}

impl NoiseSession {
    pub async fn handshake_initiator(
        transport: &mut dyn Transport,
        local: &DeviceMeta,
        trust: &TrustStore,
    ) -> Result<Self, SyncError> {
        let private = local.static_secret()?.to_bytes();
        let builder = Builder::new(PATTERN.parse().map_err(|e| SyncError::Crypto(format!("{e}")))?);
        let mut noise = builder
            .local_private_key(&private)
            .build_initiator()
            .map_err(|e| SyncError::Crypto(e.to_string()))?;

        let mut buf = vec![0u8; 65535];

        // -> e
        let len = noise
            .write_message(&[], &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        transport.send(&buf[..len]).await?;

        // <- e, ee, s, es
        let msg = transport.recv().await?;
        noise
            .read_message(&msg, &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;

        // -> s, se
        let len = noise
            .write_message(&[], &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        transport.send(&buf[..len]).await?;

        let remote = remote_static_b64(&noise)?;
        if trust.find_by_public_key(&remote).is_none() {
            return Err(SyncError::Untrusted);
        }

        let state = noise
            .into_transport_mode()
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        Ok(Self {
            state,
            remote_public_key_b64: remote,
        })
    }

    pub async fn handshake_responder(
        transport: &mut dyn Transport,
        local: &DeviceMeta,
        trust: &TrustStore,
    ) -> Result<Self, SyncError> {
        let private = local.static_secret()?.to_bytes();
        let builder = Builder::new(PATTERN.parse().map_err(|e| SyncError::Crypto(format!("{e}")))?);
        let mut noise = builder
            .local_private_key(&private)
            .build_responder()
            .map_err(|e| SyncError::Crypto(e.to_string()))?;

        let mut buf = vec![0u8; 65535];

        // <- e
        let msg = transport.recv().await?;
        noise
            .read_message(&msg, &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;

        // -> e, ee, s, es
        let len = noise
            .write_message(&[], &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        transport.send(&buf[..len]).await?;

        // <- s, se
        let msg = transport.recv().await?;
        noise
            .read_message(&msg, &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;

        let remote = remote_static_b64(&noise)?;
        if trust.find_by_public_key(&remote).is_none() {
            return Err(SyncError::Untrusted);
        }

        let state = noise
            .into_transport_mode()
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        Ok(Self {
            state,
            remote_public_key_b64: remote,
        })
    }

    pub async fn send(
        &mut self,
        transport: &mut dyn Transport,
        plaintext: &[u8],
    ) -> Result<(), SyncError> {
        if plaintext.len() > MAX_FRAME_BYTES {
            return Err(SyncError::TooLarge);
        }
        let mut buf = vec![0u8; plaintext.len() + 64];
        let len = self
            .state
            .write_message(plaintext, &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        if len > MAX_FRAME_BYTES {
            return Err(SyncError::Transport(crate::error::TransportError::TooLarge));
        }
        transport.send(&buf[..len]).await?;
        Ok(())
    }

    pub async fn recv(
        &mut self,
        transport: &mut dyn Transport,
    ) -> Result<Vec<u8>, SyncError> {
        let msg = transport.recv().await?;
        let mut buf = vec![0u8; msg.len()];
        let len = self
            .state
            .read_message(&msg, &mut buf)
            .map_err(|e| SyncError::Crypto(e.to_string()))?;
        buf.truncate(len);
        Ok(buf)
    }
}

fn remote_static_b64(noise: &snow::HandshakeState) -> Result<String, SyncError> {
    let key = noise
        .get_remote_static()
        .ok_or_else(|| SyncError::Crypto("missing remote static".into()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(key))
}
