//! Display-math blocks delimited by lines that are exactly `$$`.
//!
//! Separate from fenced code so a `$$` inside ``` is not a formula.

use std::sync::OnceLock;

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use latex_rust::{latex_to_png, Color, MathFont, PngBackground, PngOptions};

use crate::preview::FenceRole;
use crate::theme;

pub const SNIPPET: &str = "$$\n\n$$";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MathRole {
    Outside,
    Open,
    Body,
    Close,
}

#[derive(Clone, Copy, Debug)]
pub struct MathSpan {
    pub role: MathRole,
    pub start: usize,
    pub end: usize,
}

impl MathSpan {
    pub fn contains(self, line: usize) -> bool {
        self.role != MathRole::Outside && line >= self.start && line <= self.end
    }
}

pub fn spans(lines: &[String], fences: &[FenceRole], opaque: &[bool]) -> Vec<MathSpan> {
    let n = lines.len();
    let mut out = vec![
        MathSpan {
            role: MathRole::Outside,
            start: 0,
            end: 0,
        };
        n
    ];
    let mut i = 0;
    while i < n {
        if !opaque.get(i).copied().unwrap_or(false)
            && is_delim(&lines[i], fences.get(i).copied())
        {
            let start = i;
            let mut close = None;
            let mut j = i + 1;
            while j < n {
                if opaque.get(j).copied().unwrap_or(false) {
                    break;
                }
                if is_delim(&lines[j], fences.get(j).copied()) {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = close {
                out[start] = MathSpan {
                    role: MathRole::Open,
                    start,
                    end,
                };
                for k in (start + 1)..end {
                    out[k] = MathSpan {
                        role: MathRole::Body,
                        start,
                        end,
                    };
                }
                out[end] = MathSpan {
                    role: MathRole::Close,
                    start,
                    end,
                };
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_delim(line: &str, fence: Option<FenceRole>) -> bool {
    if !matches!(fence, None | Some(FenceRole::Outside)) {
        return false;
    }
    line.trim() == "$$"
}

/// First line of the formula, or the opening `$$` when the body is empty.
pub fn body_line(span: MathSpan) -> usize {
    if span.end > span.start + 1 {
        span.start + 1
    } else {
        span.start
    }
}

pub fn formula(lines: &[String], span: MathSpan) -> String {
    if span.end <= span.start + 1 {
        return String::new();
    }
    lines[span.start + 1..span.end].join("\n")
}

/// Draw the typeset formula. Empty source draws nothing unless `catch_empty`,
/// which leaves a click target so a blank block can be opened again.
/// Returns (height added, clicked).
pub fn show_preview(ui: &mut egui::Ui, formula: &str, catch_empty: bool) -> (f32, bool) {
    let formula = formula.trim();
    let top = ui.cursor().top();
    let clicked = if formula.is_empty() {
        if !catch_empty {
            return (0.0, false);
        }
        let (_, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 28.0),
            egui::Sense::click(),
        );
        resp.clicked()
    } else {
        match cached_texture(ui.ctx(), formula, true) {
            Some(tex) => ui
                .add(
                    egui::Image::from_texture(&tex)
                        .fit_to_original_size(1.0)
                        .max_width(ui.available_width()),
                )
                .clicked(),
            None => ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(formula)
                            .monospace()
                            .color(theme::TEXT),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked(),
        }
    };
    let height = (ui.cursor().top() - top).max(0.0);
    (height, clicked)
}

/// ponytail: longer formulas stay as source; raise or async-raster when inline TeX is common.
const INLINE_MAX_CHARS: usize = 64;

/// Inline `$...$` via the same PNG cache. Returns whether the glyph was clicked.
pub fn show_inline(ui: &mut egui::Ui, formula: &str, size: f32) -> bool {
    let formula = formula.trim();
    if formula.is_empty() {
        return false;
    }
    let too_long = formula.chars().count() > INLINE_MAX_CHARS;
    if too_long {
        return ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("${formula}$"))
                        .size(size)
                        .monospace()
                        .color(theme::TEXT),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
    }
    match cached_texture(ui.ctx(), formula, false) {
        Some(tex) => ui
            .add(
                egui::Image::from_texture(&tex)
                    .fit_to_original_size(1.0)
                    .max_height(size * 1.4),
            )
            .clicked(),
        None => ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("${formula}$"))
                        .size(size)
                        .monospace()
                        .color(theme::TEXT),
                )
                .sense(egui::Sense::click()),
            )
            .clicked(),
    }
}

fn cached_texture(ctx: &Context, formula: &str, display: bool) -> Option<TextureHandle> {
    let key = egui::Id::new(("math_png", display, formula));
    if let Some(slot) = ctx.data(|d| d.get_temp::<MathSlot>(key)) {
        return slot.tex;
    }
    let slot = match raster(formula, display) {
        Ok(png) => match decode_texture(ctx, formula, &png) {
            Some(tex) => MathSlot { tex: Some(tex) },
            None => MathSlot { tex: None },
        },
        Err(_) => MathSlot { tex: None },
    };
    let tex = slot.tex.clone();
    ctx.data_mut(|d| d.insert_temp(key, slot));
    tex
}

#[derive(Clone)]
struct MathSlot {
    tex: Option<TextureHandle>,
}

fn decode_texture(ctx: &Context, formula: &str, png: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(format!("math:{formula}"), color, TextureOptions::LINEAR))
}

fn raster(formula: &str, display: bool) -> Result<Vec<u8>, String> {
    let font = math_font()?;
    let mut opt = PngOptions::new();
    opt.display = display;
    // Near-white glyphs: the note canvas is almost black, default ink is black.
    opt.color = Color::rgb(theme::TEXT.r(), theme::TEXT.g(), theme::TEXT.b());
    opt.background = PngBackground::Transparent;
    latex_to_png(formula, font, &opt).map_err(|e| e.to_string())
}

fn math_font() -> Result<&'static MathFont, String> {
    static FONT: OnceLock<MathFont> = OnceLock::new();
    if FONT.get().is_none() {
        match MathFont::stix_two_math() {
            Ok(font) => {
                let _ = FONT.set(font);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    FONT.get().ok_or_else(|| "math font missing".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outside() -> Vec<FenceRole> {
        vec![FenceRole::Outside; 8]
    }

    #[test]
    fn spans_mark_open_body_close() {
        let lines = vec![
            "before".into(),
            "$$".into(),
            "x^2".into(),
            "$$".into(),
            "after".into(),
        ];
        let spans = spans(&lines, &outside(), &vec![false; lines.len()]);
        assert_eq!(spans[0].role, MathRole::Outside);
        assert_eq!(spans[1].role, MathRole::Open);
        assert_eq!(spans[2].role, MathRole::Body);
        assert_eq!(spans[3].role, MathRole::Close);
        assert_eq!(spans[4].role, MathRole::Outside);
        assert_eq!(spans[1].start, 1);
        assert_eq!(spans[1].end, 3);
        assert_eq!(formula(&lines, spans[1]), "x^2");
        assert_eq!(body_line(spans[1]), 2);
    }

    #[test]
    fn unclosed_delimiter_stays_text() {
        let lines = vec!["$$".into(), "x".into()];
        let spans = spans(&lines, &outside(), &vec![false; lines.len()]);
        assert!(spans.iter().all(|s| s.role == MathRole::Outside));
    }

    #[test]
    fn dollar_inside_fence_is_not_math() {
        let lines = vec!["$$".into(), "x".into(), "$$".into()];
        let fences = vec![FenceRole::Body, FenceRole::Body, FenceRole::Body];
        let spans = spans(&lines, &fences, &vec![false; lines.len()]);
        assert!(spans.iter().all(|s| s.role == MathRole::Outside));
    }

    #[test]
    fn opaque_line_breaks_a_math_pair() {
        let lines = vec!["$$".into(), String::new(), "$$".into(), "x".into(), "$$".into()];
        let opaque = vec![false, true, false, false, false];
        let spans = spans(&lines, &outside(), &opaque);
        assert_eq!(spans[0].role, MathRole::Outside);
        assert_eq!(spans[1].role, MathRole::Outside);
        assert_eq!(spans[2].role, MathRole::Open);
        assert_eq!(spans[3].role, MathRole::Body);
        assert_eq!(spans[4].role, MathRole::Close);
    }

    #[test]
    fn latex_png_signature() {
        let png = raster("x", true).expect("raster");
        assert!(png.starts_with(b"\x89PNG"));
    }
}
