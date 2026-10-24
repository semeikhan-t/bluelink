# BlueLink

Native Markdown vault for Linux desktop and Android, written in Rust. Notes are plain `.md` files with `[[wikilinks]]`. Devices sync directly over Bluetooth LE after pairing with a 6-digit code — no cloud.

## Status

Planning. The full specification (in Russian) lives in [`docs/ТЗ_BlueLink.md`](docs/ТЗ_BlueLink.md).

## Planned stack

- UI: `eframe` + `egui` (shared between desktop and Android)
- BLE: `bluer` (BlueZ peripheral on Linux), `btleplug` (central on Android)
- Pairing: SPAKE2 over a 6-digit code; secure channel: Noise XX
- Serialization: `serde` + `postcard`
