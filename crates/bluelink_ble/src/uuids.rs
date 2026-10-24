//! Fixed BlueLink GATT UUIDs (also documented in README).

/// Primary BlueLink GATT service.
pub const SERVICE: &str = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
/// Write Without Response: central → peripheral (phone → PC).
pub const RX: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";
/// Notify: peripheral → central (PC → phone).
pub const TX: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";
/// Read: protocol_version + pairing_open.
pub const INFO: &str = "6e400004-b5a3-f393-e0a9-e50e24dcca9e";
