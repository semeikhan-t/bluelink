//! `info` characteristic: protocol_version (u16 LE) + pairing_open (u8 0/1).

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoValue {
    pub protocol_version: u16,
    pub pairing_open: bool,
}

impl InfoValue {
    pub fn pack(self) -> [u8; 3] {
        let v = self.protocol_version.to_le_bytes();
        [v[0], v[1], u8::from(self.pairing_open)]
    }

    pub fn unpack(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 3 {
            return None;
        }
        Some(Self {
            protocol_version: u16::from_le_bytes([bytes[0], bytes[1]]),
            pairing_open: bytes[2] != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = InfoValue {
            protocol_version: PROTOCOL_VERSION,
            pairing_open: true,
        };
        assert_eq!(InfoValue::unpack(&v.pack()), Some(v));
    }
}
