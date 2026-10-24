//! BLE transport: BlueZ peripheral (feature `bluez`) / btleplug central (later).

mod fragment;
mod info;
mod uuids;

pub use fragment::{
    encode, max_payload, roundtrip, FragmentDecoder, FragmentError, MAX_FRAME_BYTES, ATT_HDR,
    FRAG_HDR,
};
pub use info::{InfoValue, PROTOCOL_VERSION};
pub use uuids::{INFO, RX, SERVICE, TX};

#[cfg(feature = "bluez")]
mod bluez;

#[cfg(feature = "bluez")]
pub use bluez::{BluezPeripheral, BluezPeripheralConfig};
