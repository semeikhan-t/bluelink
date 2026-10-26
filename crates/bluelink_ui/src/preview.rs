use bluelink_core::{parse_preview, split_wikilinks, PreviewBlock, PreviewDoc, Span, Vault};
use egui::{
    Color32, CornerRadius, CursorIcon, FontFamily, FontId, OpenUrl, RichText, Sense, Ui,
};
use egui_phosphor::regular;

use crate::media;
use crate::theme;

/// Role of a source line inside a fenced code block (``` / ~~~).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceRole {
    Outside,
    Open,
    Body,
    Close,
}

/// Update fence state for `line` and return how it should be painted.
pub fn advance_fence(line: &str, open: &mut Option<String>) -> FenceRole {
    let trimmed = line.trim_start();
    if let Some(marker) = open.as_deref() {
        if is_closing_fence(trimmed, marker) {
            *open = None;
            return FenceRole::Close;
        }
        return FenceRole::Body;
    }
    if let Some(marker) = opening_fence_marker(trimmed) {
        *open = Some(marker.to_string());
        return FenceRole::Open;
    }
    FenceRole::Outside
}

fn opening_fence_marker(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_closing_fence(trimmed: &str, marker: &str) -> bool {
    let ch = marker.chars().next().unwrap_or('`');
    let n = trimmed.chars().take_while(|c| *c == ch).count();
    n >= marker.chars().count() && trimmed[n..].trim().is_empty()
}

/// Font for the raw line editor — same size/face as the preview of that line,
/// so activating a row doesn't jump in height.
pub fn edit_font(line: &str) -> FontId {
    edit_font_for(line, FenceRole::Outside)
}

pub fn edit_font_for(line: &str, role: FenceRole) -> FontId {
    if !matches!(role, FenceRole::Outside) {
        return FontId::new(16.0, FontFamily::Monospace);
    }
    let doc = parse_preview(line);
    match doc.blocks.first() {
        Some(PreviewBlock::Heading { level, .. }) => {
            FontId::new(theme::heading_size(*level), theme::heading_family(*level))
        }
        Some(PreviewBlock::CodeBlock { .. }) => FontId::new(16.0, FontFamily::Monospace),
        _ => FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
    }
}

/// What a click on a preview line means.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LineClick {
    #[default]
    None,
    /// Open / create this wikilink target (raw body inside `[[...]]`).
    /// `new_tab` is set when Mod (Ctrl/Cmd) was held on click.
    Wiki { raw: String, new_tab: bool },
    /// Enter raw edit for this line.
    Edit,
    /// Drag-resize finished: rewrite `![[target|…]]` size in the source buffer.
    ResizeEmbed {
        target: String,
        width: u32,
        height: Option<u32>,
    },
}

/// Render a line using a preview parsed by the caller.
///
/// The live editor keeps this model between frames so scrolling does not repeatedly
/// allocate and parse the same Markdown line.
pub fn render_line_with_preview(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    line: &str,
    role: FenceRole,
    preview: Option<&PreviewDoc>,
) -> LineClick {
    if !matches!(role, FenceRole::Outside) {
        return render_fence_stripe(ui, line, role);
    }
    if line.is_empty() {
        let resp =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
        return if resp.1.clicked() {
            LineClick::Edit
        } else {
            LineClick::None
        };
    }
    let owned_preview;
    let doc = match preview {
        Some(doc) => doc,
        None => {
            owned_preview = parse_preview(line);
            &owned_preview
        }
    };
    let mut click = LineClick::None;
    for block in &doc.blocks {
        let c = render_block_compact(ui, vault, source_path, block);
        click = merge_click(click, c);
    }
    if doc.blocks.is_empty() {
        click = merge_click(
            click,
            render_text_with_wikis(
                ui,
                vault,
                source_path,
                line,
                theme::EDITOR_SIZE,
                false,
                0,
            ),
        );
    }
    click
}

/// One stripe of a multi-line fenced code block (open / body / close).
fn render_fence_stripe(ui: &mut Ui, line: &str, role: FenceRole) -> LineClick {
    let r = theme::ROUND;
    let corners = match role {
        FenceRole::Open => CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        },
        FenceRole::Close => CornerRadius {
            nw: 0,
            ne: 0,
            sw: r,
            se: r,
        },
        FenceRole::Body => CornerRadius::ZERO,
        FenceRole::Outside => CornerRadius::ZERO,
    };
    let (top, bottom) = match role {
        FenceRole::Open => (8, 2),
        FenceRole::Close => (2, 8),
        FenceRole::Body => (2, 2),
        FenceRole::Outside => (4, 4),
    };
    let rich = match role {
        FenceRole::Open | FenceRole::Close => RichText::new(line)
            .size(14.0)
            .monospace()
            .color(theme::WEAK),
        FenceRole::Body | FenceRole::Outside => {
            let shown = if line.is_empty() { " " } else { line };
            // Explicit ink — clickable labels otherwise pick hovered.fg (dark) and vanish.
            RichText::new(shown).size(16.0).monospace().color(theme::TEXT)
        }
    };

    let mut clicked = false;
    // No per-stripe stroke — adjacent stripes stack into one continuous block.
    egui::Frame::NONE
        .fill(theme::CODE_BG)
        .corner_radius(corners)
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top,
            bottom,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let resp = ui.add(egui::Label::new(rich).sense(Sense::click()));
            clicked = resp.clicked();
        });
    if clicked {
        LineClick::Edit
    } else {
        LineClick::None
    }
}

fn merge_click(a: LineClick, b: LineClick) -> LineClick {
    match (&a, &b) {
        (_, LineClick::ResizeEmbed { .. }) => b,
        (LineClick::ResizeEmbed { .. }, _) => a,
        (_, LineClick::Wiki { .. }) => b,
        (LineClick::Wiki { .. }, _) => a,
        (_, LineClick::Edit) => b,
        (LineClick::Edit, _) => a,
        _ => a,
    }
}

fn render_block_compact(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    block: &PreviewBlock,
) -> LineClick {
    match block {
        PreviewBlock::Heading { level, text } => render_text_with_wikis(
            ui,
            vault,
            source_path,
            text,
            theme::heading_size(*level),
            true,
            *level,
        ),
        PreviewBlock::Paragraph { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                click = render_spans(ui, vault, source_path, spans, theme::EDITOR_SIZE, false, 0);
            });
            click
        }
        PreviewBlock::BulletItem { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                click = render_spans(ui, vault, source_path, spans, theme::EDITOR_SIZE, false, 0);
            });
            click
        }
        PreviewBlock::NumberedItem { number, spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{number}."));
                click = render_spans(ui, vault, source_path, spans, theme::EDITOR_SIZE, false, 0);
            });
            click
        }
        PreviewBlock::Quote { spans } => {
            let mut click = LineClick::None;
            egui::Frame::NONE
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(theme::round())
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        click =
                            render_spans(ui, vault, source_path, spans, theme::EDITOR_SIZE, false, 0);
                    });
                });
            click
        }
        PreviewBlock::CodeBlock { code, .. } => {
            // Lone ``` lines parse as empty code blocks — don't draw hollow pills;
            // multi-line fences are handled via FenceRole in the editor.
            if code.trim().is_empty() {
                return render_fence_stripe(ui, "```", FenceRole::Open);
            }
            let mut clicked = false;
            egui::Frame::NONE
                .fill(theme::CODE_BG)
                .stroke(egui::Stroke::new(1.0_f32, theme::STROKE))
                .corner_radius(theme::round())
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(code.trim_end_matches('\n'))
                                .size(16.0)
                                .monospace()
                                .color(theme::TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    clicked = resp.clicked();
                });
            if clicked {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        PreviewBlock::ThematicBreak => {
            ui.separator();
            LineClick::None
        }
    }
}

fn render_text_with_wikis(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
) -> LineClick {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut click = LineClick::None;
        for span in split_wikilinks(text) {
            click = merge_click(
                click,
                render_one_span(ui, vault, source_path, &span, size, heading, level),
            );
        }
        click
    })
    .inner
}

fn render_spans(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    spans: &[Span],
    size: f32,
    heading: bool,
    level: u8,
) -> LineClick {
    let mut click = LineClick::None;
    for span in spans {
        click = merge_click(
            click,
            render_one_span(ui, vault, source_path, span, size, heading, level),
        );
    }
    click
}

fn render_one_span(
    ui: &mut Ui,
    vault: &Vault,
    source_path: Option<&str>,
    span: &Span,
    size: f32,
    heading: bool,
    level: u8,
) -> LineClick {
    match span {
        Span::Text(t) => {
            let rich = if heading {
                theme::heading_rich(t, level)
            } else {
                // Pin ink: Sense::click labels use hovered.fg_stroke — keep explicit TEXT.
                RichText::new(t).size(size).color(theme::TEXT)
            };
            let resp = ui.add(egui::Label::new(rich).sense(Sense::click()));
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Strong(t) => {
            let resp = ui.add(
                egui::Label::new(RichText::new(t).size(size).strong().color(theme::TEXT))
                    .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Emphasis(t) => {
            let resp = ui.add(
                egui::Label::new(RichText::new(t).size(size).italics().color(theme::TEXT))
                    .sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Code(t) => {
            // Same CODE_BG as fenced blocks — RichText::code() is too subtle here.
            let mut clicked = false;
            egui::Frame::NONE
                .fill(theme::CODE_BG)
                .corner_radius(theme::round_sm())
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(t).size(size).monospace().color(theme::TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    clicked = resp.clicked();
                });
            if clicked {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Link { text, dest } => {
            // External / markdown links keep default hyperlink behaviour.
            ui.hyperlink_to(RichText::new(text).size(size), dest);
            LineClick::None
        }
        Span::WikiLink { raw, alias } => {
            let target = bluelink_core::normalize_link_target(raw);
            let label = alias
                .as_deref()
                .unwrap_or_else(|| raw.split('|').next().unwrap_or(raw));
            let resolved = vault.index().resolve(raw, source_path);
            let base = if resolved.is_some() {
                theme::ACCENT
            } else {
                theme::WARN
            };

            let font = if heading {
                FontId::new(theme::heading_size(level), theme::heading_family(level))
            } else {
                FontId::new(size, FontFamily::Proportional)
            };
            // Measure with a placeholder color; final color depends on hover.
            let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font.clone(), base));
            let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
            let hovered = response.hovered();
            let color = if hovered { lighten(base) } else { base };

            if hovered {
                ui.painter().rect_filled(
                    rect.expand2(egui::vec2(4.0, 2.0)),
                    theme::round_sm(),
                    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 40),
                );
            }
            let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font, color));
            ui.painter().galley(rect.min, galley, color);
            let underline_y = rect.bottom() - 1.0;
            ui.painter().hline(
                rect.left()..=rect.right(),
                underline_y,
                egui::Stroke::new(if hovered { 1.5_f32 } else { 1.0_f32 }, color),
            );

            response
                .clone()
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(match &resolved {
                    Some(p) => format!("Open {p}"),
                    None => format!("Create {target}"),
                });
            if response.clicked() {
                let new_tab = ui.input(|i| i.modifiers.command);
                LineClick::Wiki {
                    raw: raw.clone(),
                    new_tab,
                }
            } else {
                LineClick::None
            }
        }
        Span::Embed {
            target,
            alias,
            width,
            height,
        } => {
            let label = alias.as_deref();
            render_media_embed(ui, vault, target, label, *width, *height)
        }
        Span::Image {
            dest,
            alt,
            width,
            height,
        } => {
            if dest.starts_with("http://") || dest.starts_with("https://") {
                let draw_w = embed_draw_width(ui, *width);
                let mut img = egui::Image::new(dest.as_str()).max_width(draw_w);
                if let Some(h) = *height {
                    img = img
                        .fit_to_exact_size(egui::vec2(draw_w, h as f32))
                        .maintain_aspect_ratio(false);
                } else {
                    img = img.maintain_aspect_ratio(true);
                }
                ui.add(img);
                if !alt.is_empty() {
                    ui.label(RichText::new(alt).size(13.0).color(theme::WEAK));
                }
                LineClick::None
            } else {
                let label = if alt.is_empty() {
                    None
                } else {
                    Some(alt.as_str())
                };
                render_media_embed(ui, vault, dest, label, *width, *height)
            }
        }
    }
}

fn embed_draw_width(ui: &Ui, width: Option<u32>) -> f32 {
    let avail = ui.available_width().max(40.0);
    match width {
        Some(w) => (w as f32).clamp(40.0, avail),
        None => avail.min(640.0),
    }
}

/// Embed / local image: capped async texture (not egui file:// decode), other media as open-externally.
fn render_media_embed(
    ui: &mut Ui,
    vault: &Vault,
    target: &str,
    alias: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
) -> LineClick {
    let display = alias.unwrap_or(target);
    // Cache resolve across frames — avoid even `is_file` / dir walks while scrolling.
    let resolve_id = egui::Id::new(("bl_media_resolve", target));
    let abs = ui.ctx().data(|d| d.get_temp::<Option<std::path::PathBuf>>(resolve_id));
    let abs = match abs {
        Some(cached) => cached,
        None => {
            let found = media::resolve_media(vault, target);
            ui.ctx()
                .data_mut(|d| d.insert_temp(resolve_id, found.clone()));
            found
        }
    };
    let Some(abs) = abs else {
        let resp = ui
            .add(
                egui::Label::new(
                    RichText::new(format!("{}  {display}", regular::IMAGE_BROKEN))
                        .size(theme::EDITOR_SIZE)
                        .color(theme::WEAK),
                )
                .sense(Sense::click()),
            )
            .on_hover_text(format!("Missing: {target}"));
        return if resp.clicked() {
            LineClick::Edit
        } else {
            LineClick::None
        };
    };

    let ext = media::extension_of(&abs).unwrap_or_default();
    let url = media::file_url(&abs);
    let avail = ui.available_width().max(40.0);
    let drag_id = ui.id().with("embed_w").with(target);
    let drag_w = ui.ctx().data(|d| d.get_temp::<f32>(drag_id));
    let base_w = embed_draw_width(ui, width);
    let draw_w = drag_w.unwrap_or(base_w).clamp(40.0, avail);

    if media::is_previewable_image(&ext) {
        let (media_rect, img_clicked) = match embed_texture(ui, &abs) {
            Some(tex) => {
                let native = tex.size_vec2();
                let draw_size = match height {
                    Some(h) => egui::vec2(draw_w, h as f32),
                    None => {
                        let scale = draw_w / native.x.max(1.0);
                        native * scale
                    }
                };
                let resp = ui
                    .add(
                        egui::Image::new((tex.id(), draw_size))
                            .maintain_aspect_ratio(height.is_none())
                            .sense(Sense::click()),
                    )
                    .on_hover_text(abs.display().to_string());
                (resp.rect, resp.clicked())
            }
            None => {
                // Decode in flight or failed — keep layout stable while loading.
                let ph_h = height.map(|h| h as f32).unwrap_or(draw_w * 0.55).clamp(40.0, 320.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(draw_w, ph_h), Sense::hover());
                ui.painter()
                    .rect_filled(rect, theme::round_sm(), theme::CODE_BG);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    regular::IMAGE,
                    FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
                    theme::WEAK,
                );
                (rect, false)
            }
        };

        let resized = paint_resize_handle(ui, media_rect, drag_id, draw_w, avail, target, height);
        if let Some(click) = resized {
            return click;
        }
        if img_clicked && drag_w.is_none() {
            LineClick::Edit
        } else {
            LineClick::None
        }
    } else {
        let kind = media_placeholder_kind(&ext);
        let box_w = draw_w.min(avail);
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(box_w, 40.0),
            Sense::click(),
        );
        ui.painter().rect_filled(rect, theme::round_sm(), theme::CODE_BG);
        ui.painter().text(
            rect.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{}  {display}", kind.icon),
            FontId::new(theme::EDITOR_SIZE, FontFamily::Proportional),
            theme::ACCENT,
        );
        let resized = paint_resize_handle(ui, rect, drag_id, draw_w, avail, target, height);
        if let Some(click) = resized {
            return click;
        }
        resp.clone()
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("Open {}", abs.display()));
        if resp.clicked() && drag_w.is_none() {
            ui.ctx().open_url(OpenUrl::new_tab(url));
        }
        LineClick::None
    }
}

/// Off-thread capped decode → [`TextureHandle`]. Avoids egui_extras sync UI-thread decode.
fn embed_texture(ui: &mut Ui, abs: &std::path::Path) -> Option<egui::TextureHandle> {
    use std::sync::{Arc, Mutex};

    let key = abs.to_string_lossy().to_string();
    let id = egui::Id::new(("bl_embed_tex", key.as_str()));

    #[derive(Clone)]
    enum Slot {
        Loading(Arc<Mutex<Option<Result<egui::ColorImage, String>>>>),
        Ready(egui::TextureHandle),
        Failed,
    }

    let slot = ui.ctx().data(|d| d.get_temp::<Slot>(id));
    match slot {
        Some(Slot::Ready(tex)) => return Some(tex),
        Some(Slot::Failed) => return None,
        Some(Slot::Loading(pending)) => {
            let done = pending.lock().ok().and_then(|mut g| g.take());
            match done {
                Some(Ok(color)) => {
                    let tex = ui.ctx().load_texture(
                        format!("bluelink-embed:{key}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(id, Slot::Ready(tex.clone())));
                    return Some(tex);
                }
                Some(Err(_)) => {
                    ui.ctx().data_mut(|d| d.insert_temp(id, Slot::Failed));
                    return None;
                }
                None => return None,
            }
        }
        None => {}
    }

    let pending = Arc::new(Mutex::new(None));
    ui.ctx()
        .data_mut(|d| d.insert_temp(id, Slot::Loading(pending.clone())));
    let path = abs.to_path_buf();
    let ctx = ui.ctx().clone();
    let _ = std::thread::Builder::new()
        .name("bluelink-embed-decode".into())
        .spawn(move || {
            let result = media::load_capped_color_image(&path);
            if let Ok(mut g) = pending.lock() {
                *g = Some(result);
            }
            ctx.request_repaint();
        });
    None
}

/// Corner handle: drag updates temp width; release → [`LineClick::ResizeEmbed`].
fn paint_resize_handle(
    ui: &mut Ui,
    media_rect: egui::Rect,
    drag_id: egui::Id,
    draw_w: f32,
    avail: f32,
    target: &str,
    height: Option<u32>,
) -> Option<LineClick> {
    let handle = 12.0;
    let handle_rect = egui::Rect::from_center_size(
        egui::pos2(media_rect.right() - 2.0, media_rect.bottom() - 2.0),
        egui::vec2(handle, handle),
    );
    let dragging = ui.ctx().data(|d| d.get_temp::<f32>(drag_id)).is_some();
    let hover = ui.input(|i| i.pointer.hover_pos());
    let show = dragging
        || hover.is_some_and(|p| media_rect.contains(p) || handle_rect.contains(p));
    if !show {
        return None;
    }

    ui.painter()
        .rect_filled(handle_rect, theme::round_sm(), theme::ACCENT);
    let sense = ui.interact(handle_rect, drag_id.with("h"), Sense::drag());
    sense.clone().on_hover_cursor(CursorIcon::ResizeEast);

    if sense.dragged() {
        let new_w = (draw_w + sense.drag_delta().x).clamp(40.0, avail);
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, new_w));
        ui.ctx().request_repaint();
    }
    if sense.drag_stopped() {
        let final_w = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<f32>(drag_id))
            .unwrap_or(draw_w)
            .round()
            .clamp(40.0, avail) as u32;
        return Some(LineClick::ResizeEmbed {
            target: target.to_string(),
            width: final_w.max(40),
            height,
        });
    }
    None
}

struct MediaKind {
    icon: &'static str,
}

fn media_placeholder_kind(ext: &str) -> MediaKind {
    let icon = match ext {
        "mp4" | "webm" | "ogv" | "mov" | "mkv" => regular::VIDEO,
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "opus" => regular::MUSIC_NOTES,
        "pdf" => regular::FILE_PDF,
        "svg" | "avif" | "bmp" => regular::IMAGE,
        _ => regular::FILE,
    };
    MediaKind { icon }
}

/// Brighten for hover on dark sheet (accent/warn stay legible).
fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(40),
        c.g().saturating_add(36),
        c.b().saturating_add(30),
    )
}

#[cfg(test)]
mod fence_tests {
    use super::*;

    #[test]
    fn tracks_fenced_block_roles() {
        let mut open = None;
        assert_eq!(advance_fence("```rust", &mut open), FenceRole::Open);
        assert!(open.is_some());
        assert_eq!(advance_fence("let x = 1;", &mut open), FenceRole::Body);
        assert_eq!(advance_fence("```", &mut open), FenceRole::Close);
        assert!(open.is_none());
        assert_eq!(advance_fence("after", &mut open), FenceRole::Outside);
    }
}
