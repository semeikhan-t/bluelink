# BlueLink

Native Markdown vault for Linux desktop and Android, written in Rust. Notes are plain `.md` files with `[[wikilinks]]`. Devices sync directly over Bluetooth LE after pairing with a 6-digit code — no cloud.

## Status

Stages 1–4 done: vault core, desktop UI, sync on Loopback, BlueZ GATT peripheral spike. Spec: [`docs/ТЗ_BlueLink.md`](docs/ТЗ_BlueLink.md).

```bash
cargo test --workspace
cargo run -p desktop --features bluez -- --vault ~/BlueLinkDemo
```

Without `--vault`, use **Open vault** in the start screen.

### TCP Host ↔ Initiator demo (no Android / no BLE)

Two vaults, two terminals — full pairing + sync over localhost:

```bash
# Host (shows code, waits for sync)
cargo run -p desktop -- --vault ~/BlueLinkHost --sync-tcp 127.0.0.1:7878

# Initiator (Join with code, then Synchronize)
cargo run -p desktop -- --vault ~/BlueLinkPhone --initiator --connect 127.0.0.1:7878
```

1. Host → **Devices** → **Show code**  
2. Initiator → **Devices** → enter code → **Join**  
3. Edit a note on either side → Initiator → **Sync** → **Synchronize**

## BLE peripheral spike (Linux / BlueZ)

Requires BlueZ ≥ 5.60, `bluetoothd` running, adapter powered (`bluetoothctl power on`), user in group `bluetooth` (or equivalent D-Bus policy).

```bash
cargo run -p bluelink_ble --example peripheral --features bluez
```

Then on a phone: nRF Connect → scan for **BlueLink** → connect → Read `info`, Write Without Response on `rx`, Subscribe on `tx` (echo).

| Role | UUID |
|------|------|
| Service | `6e400001-b5a3-f393-e0a9-e50e24dcca9e` |
| rx (write) | `6e400002-b5a3-f393-e0a9-e50e24dcca9e` |
| tx (notify) | `6e400003-b5a3-f393-e0a9-e50e24dcca9e` |
| info (read) | `6e400004-b5a3-f393-e0a9-e50e24dcca9e` |

`info` payload: `protocol_version` (u16 LE) + `pairing_open` (u8). Fragmentation unit-tested without hardware (`cargo test -p bluelink_ble`).

## Stack

- UI: `eframe` + `egui` (shared between desktop and Android)
- Sync: `spake2` + `snow` (Noise XX) + `postcard` over `Transport` (Loopback in tests)
- BLE: `bluer` (BlueZ peripheral, feature `bluez`); Android central (`btleplug`) next
