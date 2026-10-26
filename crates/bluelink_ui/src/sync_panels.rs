//! Devices + Sync screens.
//!
//! Host (BLE / `--sync-tcp`): Show code; peer initiates sync.
//! Initiator (`--initiator --connect`): Join with code + Sync button (TCP demo / Android later).

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use egui_phosphor::regular;

use crate::sync_ui::{SyncStatus, SyncUiCommand, SyncUiEvent, SyncUiHandle, TrustedDevice};
use crate::theme;

const LOG_CAP: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Notes,
    Devices,
    Sync,
}

#[derive(Debug, Clone)]
struct PairingUi {
    code: String,
    expires_at_ms: u64,
    attempts_left: u8,
}

#[derive(Debug, Clone)]
struct LogEntry {
    ts_ms: u64,
    text: String,
}

#[derive(Debug, Default)]
pub struct SyncPanelState {
    pub status: SyncStatus,
    trusted: Vec<TrustedDevice>,
    pairing: Option<PairingUi>,
    log: VecDeque<LogEntry>,
    last_error: Option<String>,
    /// Initiator: draft 6-digit code for Join.
    pub join_code: String,
}

impl SyncPanelState {
    pub fn apply_event(&mut self, event: SyncUiEvent) {
        match event {
            SyncUiEvent::Status(s) => {
                if !matches!(s, SyncStatus::Pairing | SyncStatus::Connecting) {
                    self.pairing = None;
                }
                self.status = s;
            }
            SyncUiEvent::Pairing {
                code,
                expires_at_ms,
                attempts_left,
            } => {
                self.status = SyncStatus::Pairing;
                self.pairing = Some(PairingUi {
                    code,
                    expires_at_ms,
                    attempts_left,
                });
            }
            SyncUiEvent::Trusted(list) => {
                self.trusted = list;
            }
            SyncUiEvent::Log { ts_ms, text } => {
                if self.log.len() >= LOG_CAP {
                    self.log.pop_front();
                }
                self.log.push_back(LogEntry { ts_ms, text });
            }
            SyncUiEvent::Error(msg) => {
                self.last_error = Some(msg.clone());
                self.status = SyncStatus::Error;
                if self.log.len() >= LOG_CAP {
                    self.log.pop_front();
                }
                self.log.push_back(LogEntry {
                    ts_ms: now_ms(),
                    text: msg,
                });
            }
        }
    }

    /// Drop expired pairing UI so the countdown doesn't stick.
    pub fn tick_pairing(&mut self) {
        let Some(p) = &self.pairing else {
            return;
        };
        if now_ms() >= p.expires_at_ms {
            self.pairing = None;
            if matches!(self.status, SyncStatus::Pairing) {
                self.status = SyncStatus::Waiting;
            }
        }
    }
}

pub fn status_label(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Idle => "Idle".into(),
        SyncStatus::Waiting => "Waiting for phone".into(),
        SyncStatus::Pairing => "Pairing".into(),
        SyncStatus::Connecting => "Connecting".into(),
        SyncStatus::Transfer { current, total } => {
            if *total == 0 {
                "Transferring…".into()
            } else {
                format!("Transfer {current} of {total}")
            }
        }
        SyncStatus::Error => "Error".into(),
    }
}

pub fn show_screen_nav(ui: &mut egui::Ui, screen: &mut Screen) {
    ui.horizontal(|ui| {
        nav_btn(ui, screen, Screen::Notes, regular::NOTEBOOK, "Notes");
        nav_btn(ui, screen, Screen::Devices, regular::DEVICES, "Devices");
        nav_btn(ui, screen, Screen::Sync, regular::ARROWS_CLOCKWISE, "Sync");
    });
}

fn nav_btn(ui: &mut egui::Ui, screen: &mut Screen, target: Screen, icon: &str, label: &str) {
    let selected = *screen == target;
    let text = egui::RichText::new(format!("{icon}  {label}")).size(14.0);
    let mut btn = egui::Button::new(text).min_size(egui::vec2(0.0, 32.0));
    if selected {
        btn = btn.fill(theme::FAINT);
    } else {
        btn = btn.frame(false);
    }
    if ui.add(btn).clicked() {
        *screen = target;
    }
}

pub fn show_devices(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut SyncPanelState,
    handle: Option<&SyncUiHandle>,
    format_hhmm: impl Fn(u64) -> String,
) {
    state.tick_pairing();

    let initiator = handle.map(|h| h.is_initiator()).unwrap_or(false);

    ui.label(
        egui::RichText::new("Devices")
            .size(22.0)
            .family(theme::display_family())
            .color(theme::TEXT),
    );
    ui.label(
        egui::RichText::new(if initiator {
            "Enter the Host pairing code, then Join. Sync from the Sync screen."
        } else {
            "Show a 6-digit code; the other device Joins (phone or --initiator)."
        })
        .size(13.0)
        .color(theme::WEAK),
    );
    ui.add_space(12.0);

    let runtime_ok = handle.is_some();
    if initiator {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Code")
                    .size(13.0)
                    .color(theme::WEAK),
            );
            let edit = ui.add(
                egui::TextEdit::singleline(&mut state.join_code)
                    .desired_width(100.0)
                    .char_limit(6)
                    .hint_text("000000"),
            );
            if edit.changed() {
                state.join_code.retain(|c| c.is_ascii_digit());
            }
            let join = ui.add_enabled(
                runtime_ok && state.join_code.len() == 6,
                egui::Button::new(format!("{}  Join", regular::KEY)).min_size(egui::vec2(0.0, 36.0)),
            );
            if join.clicked() {
                if let Some(h) = handle {
                    h.send(SyncUiCommand::Join(state.join_code.clone()));
                }
            }
            if !runtime_ok {
                ui.label(
                    egui::RichText::new("Sync runtime unavailable")
                        .size(13.0)
                        .color(theme::WARN),
                );
            }
        });
    } else {
        ui.horizontal(|ui| {
            let show = ui.add_enabled(
                runtime_ok,
                egui::Button::new(format!("{}  Show code", regular::KEY))
                    .min_size(egui::vec2(0.0, 36.0)),
            );
            if show.clicked() {
                if let Some(h) = handle {
                    h.send(SyncUiCommand::ShowCode);
                }
            }
            if state.pairing.is_some() {
                let cancel = ui.add_enabled(
                    runtime_ok,
                    egui::Button::new(format!("{}  Cancel", regular::X_CIRCLE))
                        .min_size(egui::vec2(0.0, 36.0)),
                );
                if cancel.clicked() {
                    if let Some(h) = handle {
                        h.send(SyncUiCommand::Cancel);
                    }
                    state.pairing = None;
                }
            }
            if !runtime_ok {
                ui.label(
                    egui::RichText::new("Sync runtime unavailable")
                        .size(13.0)
                        .color(theme::WARN),
                );
            }
        });
    }

    if let Some(p) = &state.pairing {
        let now = now_ms();
        let left_ms = p.expires_at_ms.saturating_sub(now);
        let secs = left_ms / 1000;
        let mm = secs / 60;
        let ss = secs % 60;
        ui.add_space(16.0);
        egui::Frame::NONE
            .fill(theme::FAINT)
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("{}  Pairing code", regular::BLUETOOTH))
                        .size(13.0)
                        .color(theme::WEAK),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(&p.code)
                        .size(36.0)
                        .family(theme::display_family())
                        .color(theme::TEXT)
                        .monospace(),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  {mm:02}:{ss:02} left · {} attempt{} left",
                        regular::CLOCK_COUNTDOWN,
                        p.attempts_left,
                        if p.attempts_left == 1 { "" } else { "s" }
                    ))
                    .size(14.0)
                    .color(theme::WEAK),
                );
            });
        // Live countdown without spinning the CPU.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    ui.add_space(20.0);
    ui.label(
        egui::RichText::new("TRUSTED")
            .size(12.0)
            .family(theme::display_family())
            .color(theme::WEAK),
    );
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    if state.trusted.is_empty() {
        ui.label(
            egui::RichText::new("No trusted devices yet")
                .size(14.0)
                .color(theme::WEAK),
        );
    } else {
        let mut revoke: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for dev in &state.trusted {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}  {}", regular::DEVICE_MOBILE, dev.name))
                                .size(15.0)
                                .color(theme::TEXT),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "paired {}",
                                format_hhmm(dev.paired_at)
                            ))
                            .size(12.0)
                            .color(theme::WEAK),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let btn = ui.add_enabled(
                                runtime_ok,
                                egui::Button::new(format!("{}  Revoke", regular::LINK_BREAK))
                                    .min_size(egui::vec2(0.0, 32.0)),
                            );
                            if btn.clicked() {
                                revoke = Some(dev.device_id.clone());
                            }
                        });
                    });
                    ui.label(
                        egui::RichText::new(&dev.device_id)
                            .size(11.0)
                            .color(theme::WEAK)
                            .monospace(),
                    );
                    ui.add_space(8.0);
                }
            });
        if let Some(id) = revoke {
            if let Some(h) = handle {
                h.send(SyncUiCommand::Revoke(id));
            }
        }
    }
}

pub fn show_sync(
    ui: &mut egui::Ui,
    state: &mut SyncPanelState,
    handle: Option<&SyncUiHandle>,
    format_hhmm: impl Fn(u64) -> String,
) {
    state.tick_pairing();

    let initiator = handle.map(|h| h.is_initiator()).unwrap_or(false);

    ui.label(
        egui::RichText::new("Sync")
            .size(22.0)
            .family(theme::display_family())
            .color(theme::TEXT),
    );
    ui.label(
        egui::RichText::new(if initiator {
            "This window initiates sync over TCP. Pair on Devices first."
        } else {
            "This window waits (Responder). Start sync from Initiator / phone."
        })
        .size(13.0)
        .color(theme::WEAK),
    );
    ui.add_space(12.0);

    if initiator {
        ui.horizontal(|ui| {
            let sync = ui.add_enabled(
                handle.is_some(),
                egui::Button::new(format!("{}  Synchronize", regular::ARROWS_CLOCKWISE))
                    .min_size(egui::vec2(0.0, 36.0)),
            );
            if sync.clicked() {
                if let Some(h) = handle {
                    h.send(SyncUiCommand::Sync);
                }
            }
            let cancel = ui.add_enabled(
                handle.is_some(),
                egui::Button::new(format!("{}  Cancel", regular::X_CIRCLE))
                    .min_size(egui::vec2(0.0, 36.0)),
            );
            if cancel.clicked() {
                if let Some(h) = handle {
                    h.send(SyncUiCommand::Cancel);
                }
            }
        });
        ui.add_space(12.0);
    }

    let (icon, color) = match state.status {
        SyncStatus::Idle => (regular::CIRCLE, theme::WEAK),
        SyncStatus::Waiting => (regular::BLUETOOTH, theme::TEXT),
        SyncStatus::Pairing => (regular::KEY, theme::WARN),
        SyncStatus::Connecting => (regular::PLUGS, theme::TEXT),
        SyncStatus::Transfer { .. } => (regular::ARROWS_CLOCKWISE, theme::TEXT),
        SyncStatus::Error => (regular::WARNING_CIRCLE, theme::ACCENT),
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{icon}  {}", status_label(&state.status)))
                .size(16.0)
                .color(color),
        );
    });
    if let Some(err) = &state.last_error {
        if matches!(state.status, SyncStatus::Error) {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(err)
                    .size(13.0)
                    .color(theme::ACCENT),
            );
        }
    }

    ui.add_space(16.0);
    ui.label(
        egui::RichText::new("LOG")
            .size(12.0)
            .family(theme::display_family())
            .color(theme::WEAK),
    );
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.log.is_empty() {
                ui.label(
                    egui::RichText::new("No sync events yet")
                        .size(13.0)
                        .color(theme::WEAK),
                );
            }
            for entry in &state.log {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format_hhmm(entry.ts_ms))
                            .size(12.0)
                            .color(theme::WEAK)
                            .monospace(),
                    );
                    ui.label(
                        egui::RichText::new(&entry.text)
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                });
            }
        });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_rings_at_50() {
        let mut state = SyncPanelState::default();
        for i in 0..60 {
            state.apply_event(SyncUiEvent::Log {
                ts_ms: i,
                text: format!("e{i}"),
            });
        }
        assert_eq!(state.log.len(), LOG_CAP);
        assert_eq!(state.log.front().unwrap().text, "e10");
        assert_eq!(state.log.back().unwrap().text, "e59");
    }

    #[test]
    fn pairing_cleared_when_waiting() {
        let mut state = SyncPanelState::default();
        state.apply_event(SyncUiEvent::Pairing {
            code: "123456".into(),
            expires_at_ms: now_ms() + 60_000,
            attempts_left: 5,
        });
        assert!(state.pairing.is_some());
        state.apply_event(SyncUiEvent::Status(SyncStatus::Waiting));
        assert!(state.pairing.is_none());
        assert_eq!(state.status, SyncStatus::Waiting);
    }
}
