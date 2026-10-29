//! Mermaid fenced diagrams (` ```mermaid ` … ` ``` `).
//!
//! Renderer choice (stage 9.1): external `mmdc` (mermaid-cli) → PNG, hashed under
//! `{temp}/bluelink-mermaid/`. Uses `-t dark -b transparent` so edges stay visible on
//! BlueLink’s dark chrome. If `mmdc` is missing or fails → stub widget with
//! source + status (no `mermaid.min.js`, no WASM, no SVG stack).
//!
//! `ponytail:` sync `mmdc` on the UI thread; first paint can hitch. Upgrade: background
//! job + placeholder while rasterizing.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, RichText, TextureHandle, TextureOptions, Ui};
use egui_phosphor::regular;
use sha2::{Digest, Sha256};

use crate::preview::FenceRole;
use crate::theme;

const MMDC_TIMEOUT: Duration = Duration::from_secs(10);
/// Baked into the cache key with the diagram source. Bump when mmdc flags/theme change
/// so stale PNGs (e.g. default-theme dark arrows on our dark UI) are not reused.
const RENDER_FINGERPRINT: &str = "mmdc:dark:transparent:v1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MermaidRole {
    Outside,
    Open,
    Body,
    Close,
}

#[derive(Clone, Copy, Debug)]
pub struct MermaidSpan {
    pub role: MermaidRole,
    pub start: usize,
    pub end: usize,
}

impl MermaidSpan {
    pub fn contains(self, line: usize) -> bool {
        self.role != MermaidRole::Outside && line >= self.start && line <= self.end
    }
}

/// Closed `mermaid` fences only; unclosed / other langs stay [`MermaidRole::Outside`].
pub fn spans(
    lines: &[String],
    fence_roles: &[FenceRole],
    fence_langs: &[Option<String>],
) -> Vec<MermaidSpan> {
    let n = lines.len();
    let mut out = vec![
        MermaidSpan {
            role: MermaidRole::Outside,
            start: 0,
            end: 0,
        };
        n
    ];
    let mut i = 0;
    while i < n {
        let is_open = matches!(fence_roles.get(i), Some(FenceRole::Open))
            && fence_langs
                .get(i)
                .and_then(|l| l.as_deref())
                .is_some_and(|l| l.eq_ignore_ascii_case("mermaid"));
        if !is_open {
            i += 1;
            continue;
        }
        let start = i;
        let mut close = None;
        let mut j = i + 1;
        while j < n {
            if matches!(fence_roles.get(j), Some(FenceRole::Close)) {
                close = Some(j);
                break;
            }
            j += 1;
        }
        let Some(end) = close else {
            i += 1;
            continue;
        };
        out[start] = MermaidSpan {
            role: MermaidRole::Open,
            start,
            end,
        };
        for k in (start + 1)..end {
            out[k] = MermaidSpan {
                role: MermaidRole::Body,
                start,
                end,
            };
        }
        out[end] = MermaidSpan {
            role: MermaidRole::Close,
            start,
            end,
        };
        i = end + 1;
    }
    out
}

/// First body line, or the opening fence when the body is empty.
pub fn body_line(span: MermaidSpan) -> usize {
    if span.end > span.start + 1 {
        span.start + 1
    } else {
        span.start
    }
}

pub fn source(lines: &[String], span: MermaidSpan) -> String {
    if span.end <= span.start + 1 {
        return String::new();
    }
    lines[span.start + 1..span.end].join("\n")
}

pub fn source_hash(source: &str) -> String {
    let mut h = Sha256::new();
    h.update(RENDER_FINGERPRINT.as_bytes());
    h.update([0]);
    h.update(source.as_bytes());
    hex::encode(h.finalize())
}

/// Draw diagram or stub. Returns (height added, clicked).
pub fn show_preview(ui: &mut Ui, source: &str) -> (f32, bool) {
    let source = source.trim_end_matches('\n');
    let top = ui.cursor().top();
    let clicked = if source.trim().is_empty() {
        let (_, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 28.0),
            egui::Sense::click(),
        );
        resp.clicked()
    } else {
        match cached_slot(ui.ctx(), source) {
            CacheSlot::Ready(tex) => ui
                .add(
                    egui::Image::from_texture(&tex)
                        .fit_to_original_size(1.0)
                        .max_width(ui.available_width()),
                )
                .clicked(),
            CacheSlot::Failed(err) => show_stub(ui, source, &err),
        }
    };
    let height = (ui.cursor().top() - top).max(0.0);
    (height, clicked)
}

fn show_stub(ui: &mut Ui, source: &str, status: &str) -> bool {
    let mut clicked = false;
    let frame = egui::Frame::NONE
        .fill(theme::FAINT)
        .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
        .corner_radius(theme::round_sm())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            let title = ui.add(
                egui::Label::new(
                    RichText::new(format!("{} Mermaid", regular::GRAPH))
                        .size(12.0)
                        .color(theme::WEAK),
                )
                .sense(egui::Sense::click()),
            );
            if title.clicked() {
                clicked = true;
            }
            ui.label(RichText::new(status).size(12.0).color(theme::WARN));
            egui::CollapsingHeader::new("Source")
                .default_open(false)
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(source)
                                .monospace()
                                .size(12.0)
                                .color(theme::TEXT),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if resp.clicked() {
                        clicked = true;
                    }
                });
        });
    clicked || frame.response.clicked()
}

#[derive(Clone)]
enum CacheSlot {
    Ready(TextureHandle),
    Failed(String),
}

fn cached_slot(ctx: &Context, source: &str) -> CacheSlot {
    let hash = source_hash(source);
    let key = egui::Id::new(("mermaid_png", hash.clone()));
    if let Some(slot) = ctx.data(|d| d.get_temp::<CacheSlot>(key)) {
        return slot;
    }
    let slot = match raster_to_png(source, &hash) {
        Ok(png) => match decode_texture(ctx, &hash, &png) {
            Some(tex) => CacheSlot::Ready(tex),
            None => CacheSlot::Failed("failed to decode mmdc PNG".into()),
        },
        Err(err) => CacheSlot::Failed(err),
    };
    ctx.data_mut(|d| d.insert_temp(key, slot.clone()));
    slot
}

fn decode_texture(ctx: &Context, hash: &str, png: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(
        format!("mermaid:{hash}"),
        color,
        TextureOptions::LINEAR,
    ))
}

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("bluelink-mermaid")
}

fn png_path(hash: &str) -> PathBuf {
    cache_dir().join(format!("{hash}.png"))
}

/// `true` when `mmdc` resolves on PATH (cached for the process).
pub fn mmdc_available() -> bool {
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| which_mmdc().is_some())
}

fn which_mmdc() -> Option<PathBuf> {
    Command::new("mmdc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .filter(|s| s.success())
        .map(|_| PathBuf::from("mmdc"))
}

fn raster_to_png(source: &str, hash: &str) -> Result<Vec<u8>, String> {
    let out = png_path(hash);
    if out.is_file() {
        return fs::read(&out).map_err(|e| format!("read cache: {e}"));
    }
    if !mmdc_available() {
        return Err("mmdc not found — install @mermaid-js/mermaid-cli or edit source".into());
    }
    let dir = cache_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("cache dir: {e}"))?;
    let input = dir.join(format!("{hash}.mmd"));
    {
        let mut f = fs::File::create(&input).map_err(|e| format!("write input: {e}"))?;
        f.write_all(source.as_bytes())
            .map_err(|e| format!("write input: {e}"))?;
    }
    run_mmdc(&input, &out)?;
    let _ = fs::remove_file(&input);
    fs::read(&out).map_err(|e| format!("read mmdc output: {e}"))
}

fn run_mmdc(input: &Path, output: &Path) -> Result<(), String> {
    // Default mermaid theme draws dark strokes; with `-b transparent` on BlueLink's
    // dark editor those arrows vanish. `dark` keeps nodes readable and edges light.
    let mut child = Command::new("mmdc")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-t")
        .arg("dark")
        .arg("-b")
        .arg("transparent")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "mmdc not found — install @mermaid-js/mermaid-cli or edit source".into()
            } else {
                format!("spawn mmdc: {e}")
            }
        })?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|mut s| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                        Some(buf)
                    })
                    .unwrap_or_default();
                if status.success() && output.is_file() {
                    return Ok(());
                }
                let msg = stderr.trim();
                if msg.is_empty() {
                    return Err(format!("mmdc failed ({status})"));
                }
                return Err(truncate_status(msg));
            }
            Ok(None) if started.elapsed() > MMDC_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("mmdc timed out".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(format!("wait mmdc: {e}")),
        }
    }
}

fn truncate_status(s: &str) -> String {
    const MAX: usize = 240;
    let one_line = s.lines().next().unwrap_or(s).trim();
    if one_line.chars().count() <= MAX {
        one_line.to_string()
    } else {
        let mut out: String = one_line.chars().take(MAX).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror the fence_roles / fence_langs pass in `editor::show_live`.
    fn roles_langs(lines: &[&str]) -> (Vec<String>, Vec<FenceRole>, Vec<Option<String>>) {
        let owned: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        let mut roles = Vec::with_capacity(owned.len());
        let mut langs = Vec::with_capacity(owned.len());
        let mut open = None;
        let mut cur_lang = None;
        for line in &owned {
            let role = crate::preview::advance_fence(line, &mut open);
            match role {
                FenceRole::Open => {
                    cur_lang = crate::preview::fence_lang(line);
                    langs.push(cur_lang.clone());
                }
                FenceRole::Body | FenceRole::Close => langs.push(cur_lang.clone()),
                FenceRole::Outside => {
                    cur_lang = None;
                    langs.push(None);
                }
            }
            roles.push(role);
        }
        (owned, roles, langs)
    }

    #[test]
    fn spans_closed_mermaid_only() {
        let (lines, roles, langs) = roles_langs(&[
            "```mermaid",
            "flowchart TD",
            "  A-->B",
            "```",
            "```rust",
            "fn main() {}",
            "```",
        ]);
        let s = spans(&lines, &roles, &langs);
        assert_eq!(s[0].role, MermaidRole::Open);
        assert_eq!(s[1].role, MermaidRole::Body);
        assert_eq!(s[2].role, MermaidRole::Body);
        assert_eq!(s[3].role, MermaidRole::Close);
        assert_eq!(s[0].end, 3);
        assert_eq!(s[4].role, MermaidRole::Outside);
        assert_eq!(s[5].role, MermaidRole::Outside);
        assert_eq!(
            source(&lines, s[0]),
            "flowchart TD\n  A-->B"
        );
    }

    #[test]
    fn unclosed_mermaid_is_outside() {
        let (lines, roles, langs) = roles_langs(&["```mermaid", "flowchart TD"]);
        let s = spans(&lines, &roles, &langs);
        assert!(s.iter().all(|sp| sp.role == MermaidRole::Outside));
    }

    #[test]
    fn source_hash_stable() {
        assert_eq!(source_hash("A-->B"), source_hash("A-->B"));
        assert_ne!(source_hash("A-->B"), source_hash("A-->C"));
        // Fingerprint is part of the key (theme/bg bump must invalidate disk cache).
        assert!(
            source_hash("A-->B").len() == 64,
            "sha256 hex"
        );
    }

    #[test]
    fn mmdc_available_does_not_panic() {
        let _ = mmdc_available();
    }
}
