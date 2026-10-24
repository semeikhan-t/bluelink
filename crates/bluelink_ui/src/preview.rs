use bluelink_core::{parse_preview, split_wikilinks, PreviewBlock, Span, Vault};
use egui::{Color32, CursorIcon, FontFamily, FontId, RichText, Sense, Ui};

use crate::theme;

/// Font for the raw line editor — same size/face as the preview of that line,
/// so activating a row doesn't jump in height.
pub fn edit_font(line: &str) -> FontId {
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
    Wiki(String),
    /// Enter raw edit for this line.
    Edit,
}

/// Compact preview of a single source line.
/// Wikilinks render as accent underlines; click opens them. Other text click → edit.
pub fn render_line(ui: &mut Ui, vault: &Vault, line: &str) -> LineClick {
    if line.is_empty() {
        let resp =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
        return if resp.1.clicked() {
            LineClick::Edit
        } else {
            LineClick::None
        };
    }
    let doc = parse_preview(line);
    let mut click = LineClick::None;
    for block in &doc.blocks {
        let c = render_block_compact(ui, vault, block);
        click = merge_click(click, c);
    }
    if doc.blocks.is_empty() {
        click = merge_click(click, render_text_with_wikis(ui, vault, line, 17.0, false, 0));
    }
    click
}

fn merge_click(a: LineClick, b: LineClick) -> LineClick {
    match (&a, &b) {
        (_, LineClick::Wiki(_)) => b,
        (LineClick::Wiki(_), _) => a,
        (_, LineClick::Edit) => b,
        (LineClick::Edit, _) => a,
        _ => a,
    }
}

fn render_block_compact(ui: &mut Ui, vault: &Vault, block: &PreviewBlock) -> LineClick {
    match block {
        PreviewBlock::Heading { level, text } => {
            render_text_with_wikis(ui, vault, text, theme::heading_size(*level), true, *level)
        }
        PreviewBlock::Paragraph { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                click = render_spans(ui, vault, spans, 17.0, false, 0);
            });
            click
        }
        PreviewBlock::BulletItem { spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                click = render_spans(ui, vault, spans, 17.0, false, 0);
            });
            click
        }
        PreviewBlock::NumberedItem { number, spans } => {
            let mut click = LineClick::None;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{number}."));
                click = render_spans(ui, vault, spans, 17.0, false, 0);
            });
            click
        }
        PreviewBlock::Quote { spans } => {
            let mut click = LineClick::None;
            egui::Frame::NONE
                .fill(ui.visuals().faint_bg_color)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        click = render_spans(ui, vault, spans, 17.0, false, 0);
                    });
                });
            click
        }
        PreviewBlock::CodeBlock { code, .. } => {
            let resp = ui.add(
                egui::Label::new(
                    RichText::new(code.trim_end_matches('\n'))
                        .size(16.0)
                        .monospace(),
                )
                .sense(Sense::click()),
            );
            if resp.clicked() {
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
    text: &str,
    size: f32,
    heading: bool,
    level: u8,
) -> LineClick {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut click = LineClick::None;
        for span in split_wikilinks(text) {
            click = merge_click(click, render_one_span(ui, vault, &span, size, heading, level));
        }
        click
    })
    .inner
}

fn render_spans(
    ui: &mut Ui,
    vault: &Vault,
    spans: &[Span],
    size: f32,
    heading: bool,
    level: u8,
) -> LineClick {
    let mut click = LineClick::None;
    for span in spans {
        click = merge_click(click, render_one_span(ui, vault, span, size, heading, level));
    }
    click
}

fn render_one_span(
    ui: &mut Ui,
    vault: &Vault,
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
                RichText::new(t).size(size)
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
                egui::Label::new(RichText::new(t).size(size).strong()).sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Emphasis(t) => {
            let resp = ui.add(
                egui::Label::new(RichText::new(t).size(size).italics()).sense(Sense::click()),
            );
            if resp.clicked() {
                LineClick::Edit
            } else {
                LineClick::None
            }
        }
        Span::Code(t) => {
            let resp = ui.add(
                egui::Label::new(RichText::new(t).size(size).monospace().code())
                    .sense(Sense::click()),
            );
            if resp.clicked() {
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
            let resolved = vault.index().resolve(raw);
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
                    rect.expand2(egui::vec2(3.0, 1.5)),
                    3.0,
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
                LineClick::Wiki(raw.clone())
            } else {
                LineClick::None
            }
        }
    }
}

fn lighten(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(48),
        c.g().saturating_add(48),
        c.b().saturating_add(36),
    )
}
