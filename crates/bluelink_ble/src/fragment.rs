//! BLE ATT fragmentation for protocol frames (max 64 KiB).
//!
//! Packet layout: `[u8 flags][u16 seq LE][payload…]`
//! flags: bit0 = first, bit1 = last.

use thiserror::Error;

/// Same limit as `bluelink_sync::transport::MAX_FRAME_BYTES`.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// ATT header overhead (opcode + handle) — usable payload is MTU − 3.
pub const ATT_HDR: usize = 3;
/// Fragment header size.
pub const FRAG_HDR: usize = 3;

const FLAG_FIRST: u8 = 0b01;
const FLAG_LAST: u8 = 0b10;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FragmentError {
    #[error("message exceeds 64 KiB")]
    TooLarge,

    #[error("empty frame")]
    Empty,

    #[error("MTU too small for fragment header")]
    MtuTooSmall,

    #[error("fragment decode: {0}")]
    Decode(&'static str),
}

/// Max bytes of protocol payload per ATT packet for a given MTU.
pub fn max_payload(mtu: u16) -> Result<usize, FragmentError> {
    let usable = (mtu as usize).saturating_sub(ATT_HDR);
    if usable <= FRAG_HDR {
        return Err(FragmentError::MtuTooSmall);
    }
    Ok(usable - FRAG_HDR)
}

/// Split a protocol frame into ATT fragment packets.
pub fn encode(frame: &[u8], mtu: u16) -> Result<Vec<Vec<u8>>, FragmentError> {
    if frame.is_empty() {
        return Err(FragmentError::Empty);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(FragmentError::TooLarge);
    }
    let chunk = max_payload(mtu)?;
    let mut out = Vec::new();
    let mut seq: u16 = 0;
    let mut offset = 0;
    while offset < frame.len() {
        let end = (offset + chunk).min(frame.len());
        let first = offset == 0;
        let last = end == frame.len();
        let mut flags = 0u8;
        if first {
            flags |= FLAG_FIRST;
        }
        if last {
            flags |= FLAG_LAST;
        }
        let mut pkt = Vec::with_capacity(FRAG_HDR + (end - offset));
        pkt.push(flags);
        pkt.extend_from_slice(&seq.to_le_bytes());
        pkt.extend_from_slice(&frame[offset..end]);
        out.push(pkt);
        offset = end;
        seq = seq.wrapping_add(1);
    }
    Ok(out)
}

/// Incremental reassembly of fragments into one protocol frame.
#[derive(Debug, Default)]
pub struct FragmentDecoder {
    buf: Vec<u8>,
    next_seq: u16,
    started: bool,
}

impl FragmentDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one ATT fragment. Returns `Some(frame)` when last fragment arrives.
    pub fn push(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, FragmentError> {
        if packet.len() < FRAG_HDR {
            return Err(FragmentError::Decode("packet too short"));
        }
        let flags = packet[0];
        let seq = u16::from_le_bytes([packet[1], packet[2]]);
        let payload = &packet[FRAG_HDR..];

        let is_first = flags & FLAG_FIRST != 0;
        let is_last = flags & FLAG_LAST != 0;

        if is_first {
            self.buf.clear();
            self.next_seq = 0;
            self.started = true;
        } else if !self.started {
            return Err(FragmentError::Decode("missing first fragment"));
        }

        if seq != self.next_seq {
            self.reset();
            return Err(FragmentError::Decode("sequence gap"));
        }
        self.next_seq = self.next_seq.wrapping_add(1);

        if self.buf.len() + payload.len() > MAX_FRAME_BYTES {
            self.reset();
            return Err(FragmentError::TooLarge);
        }
        self.buf.extend_from_slice(payload);

        if is_last {
            if self.buf.is_empty() {
                self.reset();
                return Err(FragmentError::Empty);
            }
            let frame = std::mem::take(&mut self.buf);
            self.reset();
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.next_seq = 0;
        self.started = false;
    }
}

/// Encode then decode roundtrip helper for tests.
pub fn roundtrip(frame: &[u8], mtu: u16) -> Result<Vec<u8>, FragmentError> {
    let packets = encode(frame, mtu)?;
    let mut dec = FragmentDecoder::new();
    let mut out = None;
    for pkt in &packets {
        if let Some(frame) = dec.push(pkt)? {
            out = Some(frame);
        }
    }
    out.ok_or(FragmentError::Decode("incomplete"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_one_byte_default_mtu() {
        let frame = b"x";
        assert_eq!(roundtrip(frame, 23).unwrap(), frame);
    }

    #[test]
    fn roundtrip_mtu_23_multi_fragment() {
        // usable = 20, payload/chunk = 17
        let frame: Vec<u8> = (0..50).collect();
        assert_eq!(roundtrip(&frame, 23).unwrap(), frame);
    }

    #[test]
    fn roundtrip_64kib() {
        let frame = vec![0xABu8; MAX_FRAME_BYTES];
        assert_eq!(roundtrip(&frame, 247).unwrap(), frame);
    }

    #[test]
    fn rejects_oversized() {
        let frame = vec![0u8; MAX_FRAME_BYTES + 1];
        assert_eq!(encode(&frame, 247), Err(FragmentError::TooLarge));
    }

    #[test]
    fn sequence_gap_errors() {
        // Force ≥3 fragments: 17 bytes/chunk at MTU 23 → 40 bytes ⇒ 3 packets
        let frame: Vec<u8> = (0..40).collect();
        let packets = encode(&frame, 23).unwrap();
        assert!(packets.len() >= 3);
        let mut dec = FragmentDecoder::new();
        dec.push(&packets[0]).unwrap();
        // skip packets[1], push packets[2] with wrong seq
        assert!(dec.push(&packets[2]).is_err());
    }

    #[test]
    fn missing_first_errors() {
        let packets = encode(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18], 23)
            .unwrap();
        if packets.len() < 2 {
            return;
        }
        let mut dec = FragmentDecoder::new();
        assert!(dec.push(&packets[1]).is_err());
    }

    #[test]
    fn mtu_too_small() {
        assert_eq!(max_payload(3), Err(FragmentError::MtuTooSmall));
    }
}
