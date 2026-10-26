use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Simple preview model for the UI (not HTML).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewDoc {
    pub blocks: Vec<PreviewBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewBlock {
    Heading { level: u8, text: String },
    Paragraph { spans: Vec<Span> },
    BulletItem { spans: Vec<Span> },
    NumberedItem { number: u64, spans: Vec<Span> },
    Quote { spans: Vec<Span> },
    CodeBlock { lang: Option<String>, code: String },
    ThematicBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Strong(String),
    Emphasis(String),
    Strike(String),
    Highlight(String),
    /// Obsidian `%%…%%` comment body (delimiters not stored; UI paints them when shown).
    Comment(String),
    /// Inline TeX from `$...$` (not `$$` blocks).
    InlineMath { formula: String },
    Code(String),
    Link { text: String, dest: String },
    WikiLink { raw: String, alias: Option<String> },
    /// Obsidian-style embed `![[target]]` / `![[target|alias]]` / `![[target|400]]`.
    Embed {
        target: String,
        alias: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// Markdown image `![alt](dest)` / `![alt|400](dest)`.
    Image {
        dest: String,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

/// Obsidian size suffix: `400` or `400x300` (x case-insensitive). Not a display alias.
pub fn parse_embed_size(s: &str) -> Option<(u32, Option<u32>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        let w: u32 = s.parse().ok()?;
        return (w > 0).then_some((w, None));
    }
    let x = s.find(|c| c == 'x' || c == 'X')?;
    let (w, h) = (&s[..x], &s[x + 1..]);
    if w.is_empty()
        || h.is_empty()
        || !w.bytes().all(|b| b.is_ascii_digit())
        || !h.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: u32 = w.parse().ok()?;
    let h: u32 = h.parse().ok()?;
    (w > 0 && h > 0).then_some((w, Some(h)))
}

/// Split a pipe suffix into display alias vs Obsidian size.
fn alias_or_size(pipe: &str) -> (Option<String>, Option<u32>, Option<u32>) {
    match parse_embed_size(pipe) {
        Some((w, h)) => (None, Some(w), h),
        None => (Some(pipe.to_string()), None, None),
    }
}

/// Alt text may end with `|400` / `|400x200`; pure size alt → empty display string.
fn split_image_alt(alt: &str) -> (String, Option<u32>, Option<u32>) {
    if let Some((before, after)) = alt.rsplit_once('|') {
        if let Some((w, h)) = parse_embed_size(after) {
            return (before.to_string(), Some(w), h);
        }
        return (alt.to_string(), None, None);
    }
    if let Some((w, h)) = parse_embed_size(alt) {
        return (String::new(), Some(w), h);
    }
    (alt.to_string(), None, None)
}

/// Parse markdown into a preview document.
/// Wikilinks `[[...]]` → [`Span::WikiLink`]; embeds `![[...]]` → [`Span::Embed`];
/// markdown images `![](…)` → [`Span::Image`].
///
/// Note: pulldown-cmark emits `[[Note]]` as separate Text events (`[`, `[`, `Note`, `]`, `]`),
/// so plain text is buffered and run through [`split_wikilinks`] before being pushed as spans.
pub fn parse_preview(input: &str) -> PreviewDoc {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(input, options);
    let mut blocks = Vec::new();

    let mut heading_level: Option<u8> = None;
    let mut heading_text = String::new();

    let mut para_spans: Option<Vec<Span>> = None;
    let mut quote_spans: Option<Vec<Span>> = None;
    let mut list_item_spans: Option<Vec<Span>> = None;
    let mut list_item_number: Option<u64> = None;
    let mut next_ordered: Option<u64> = None;

    let mut code_lang: Option<String> = None;
    let mut code_buf = String::new();
    let mut in_code = false;

    let mut strong_buf: Option<String> = None;
    let mut em_buf: Option<String> = None;
    let mut strike_buf: Option<String> = None;
    let mut pending_link_dest: Option<String> = None;
    let mut pending_image_dest: Option<String> = None;
    let mut image_alt_buf: Option<String> = None;

    // Coalesce fragmented Text events so `[[wikilinks]]` can be detected.
    let mut pending_text = String::new();

    let flush_text = |pending: &mut String,
                      para: &mut Option<Vec<Span>>,
                      quote: &mut Option<Vec<Span>>,
                      item: &mut Option<Vec<Span>>,
                      heading: &mut String,
                      in_heading: bool| {
        if pending.is_empty() {
            return;
        }
        let text = std::mem::take(pending);
        if in_heading {
            heading.push_str(&text);
            return;
        }
        for piece in split_wikilinks(&text) {
            match piece {
                Span::Text(t) => {
                    for mark in split_inline_marks(&t) {
                        push_span(para, quote, item, heading, false, mark);
                    }
                }
                other => push_span(para, quote, item, heading, false, other),
            }
        }
    };

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                heading_level = Some(level as u8);
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    true,
                );
                if let Some(level) = heading_level.take() {
                    blocks.push(PreviewBlock::Heading {
                        level,
                        text: std::mem::take(&mut heading_text),
                    });
                }
            }
            Event::Start(Tag::Paragraph) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                if quote_spans.is_some() {
                    // paragraph inside quote — reuse quote buffer
                } else {
                    para_spans = Some(Vec::new());
                }
            }
            Event::End(TagEnd::Paragraph) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                if let Some(spans) = para_spans.take() {
                    blocks.push(PreviewBlock::Paragraph { spans });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                quote_spans = Some(Vec::new());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                if let Some(spans) = quote_spans.take() {
                    blocks.push(PreviewBlock::Quote { spans });
                }
            }
            Event::Start(Tag::List(start)) => {
                next_ordered = start;
            }
            Event::End(TagEnd::List(_)) => {
                next_ordered = None;
            }
            Event::Start(Tag::Item) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                list_item_spans = Some(Vec::new());
                list_item_number = next_ordered;
                if let Some(n) = next_ordered.as_mut() {
                    *n += 1;
                }
            }
            Event::End(TagEnd::Item) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                if let Some(spans) = list_item_spans.take() {
                    if let Some(number) = list_item_number.take() {
                        blocks.push(PreviewBlock::NumberedItem { number, spans });
                    } else {
                        blocks.push(PreviewBlock::BulletItem { spans });
                    }
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                in_code = true;
                code_buf.clear();
                code_lang = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(lang) => {
                        if lang.is_empty() {
                            None
                        } else {
                            Some(lang.into_string())
                        }
                    }
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                blocks.push(PreviewBlock::CodeBlock {
                    lang: code_lang.take(),
                    code: std::mem::take(&mut code_buf),
                });
            }
            Event::Rule => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    false,
                );
                blocks.push(PreviewBlock::ThematicBreak);
            }
            Event::Start(Tag::Strong) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                strong_buf = Some(String::new());
            }
            Event::End(TagEnd::Strong) => {
                if let Some(text) = strong_buf.take() {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        heading_level.is_some(),
                        Span::Strong(text),
                    );
                }
            }
            Event::Start(Tag::Emphasis) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                em_buf = Some(String::new());
            }
            Event::End(TagEnd::Emphasis) => {
                if let Some(text) = em_buf.take() {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        heading_level.is_some(),
                        Span::Emphasis(text),
                    );
                }
            }
            Event::Start(Tag::Strikethrough) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                strike_buf = Some(String::new());
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some(text) = strike_buf.take() {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        heading_level.is_some(),
                        Span::Strike(text),
                    );
                }
            }
            Event::Code(code) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                push_span(
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                    Span::Code(code.into_string()),
                );
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                pending_link_dest = Some(dest_url.into_string());
                strong_buf = Some(String::new());
            }
            Event::End(TagEnd::Link) => {
                let dest = pending_link_dest.take().unwrap_or_default();
                if let Some(text) = strong_buf.take() {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        heading_level.is_some(),
                        Span::Link { text, dest },
                    );
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                flush_text(
                    &mut pending_text,
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                );
                pending_image_dest = Some(dest_url.into_string());
                image_alt_buf = Some(String::new());
            }
            Event::End(TagEnd::Image) => {
                let dest = pending_image_dest.take().unwrap_or_default();
                let raw_alt = image_alt_buf.take().unwrap_or_default();
                let (alt, width, height) = split_image_alt(&raw_alt);
                push_span(
                    &mut para_spans,
                    &mut quote_spans,
                    &mut list_item_spans,
                    &mut heading_text,
                    heading_level.is_some(),
                    Span::Image {
                        dest,
                        alt,
                        width,
                        height,
                    },
                );
            }
            Event::Text(text) => {
                let s = text.into_string();
                if in_code {
                    code_buf.push_str(&s);
                } else if let Some(buf) = image_alt_buf.as_mut() {
                    buf.push_str(&s);
                } else if let Some(buf) = strike_buf
                    .as_mut()
                    .or(strong_buf.as_mut())
                    .or(em_buf.as_mut())
                {
                    buf.push_str(&s);
                } else {
                    pending_text.push_str(&s);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if heading_level.is_some()
                    || strong_buf.is_some()
                    || em_buf.is_some()
                    || strike_buf.is_some()
                {
                    if let Some(buf) = strike_buf
                        .as_mut()
                        .or(strong_buf.as_mut())
                        .or(em_buf.as_mut())
                    {
                        buf.push(' ');
                    } else {
                        pending_text.push(' ');
                    }
                } else {
                    pending_text.push('\n');
                }
            }
            _ => {}
        }
    }

    flush_text(
        &mut pending_text,
        &mut para_spans,
        &mut quote_spans,
        &mut list_item_spans,
        &mut heading_text,
        heading_level.is_some(),
    );

    PreviewDoc { blocks }
}

fn push_span(
    para: &mut Option<Vec<Span>>,
    quote: &mut Option<Vec<Span>>,
    item: &mut Option<Vec<Span>>,
    heading: &mut String,
    in_heading: bool,
    span: Span,
) {
    if in_heading {
        match &span {
            Span::Text(t)
            | Span::Strong(t)
            | Span::Emphasis(t)
            | Span::Strike(t)
            | Span::Highlight(t)
            | Span::Comment(t)
            | Span::Code(t) => heading.push_str(t),
            Span::InlineMath { formula } => heading.push_str(formula),
            _ => {}
        }
        return;
    }
    if let Some(spans) = item.as_mut() {
        spans.push(span);
    } else if let Some(spans) = quote.as_mut() {
        spans.push(span);
    } else if let Some(spans) = para.as_mut() {
        spans.push(span);
    }
}

/// Split plain text into comment / highlight / inline math / text.
///
/// Closed `%%…%%` becomes [`Span::Comment`] (body only). Unclosed `%%` stays as text.
/// First closer wins (no nesting). Live editor parses per line, so multi-line pairs do not match.
fn split_inline_marks(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        // `%%…%%` comment
        if i + 1 < bytes.len() && bytes[i] == b'%' && bytes[i + 1] == b'%' {
            if let Some(end) = find_close(bytes, i + 2, b"%%") {
                if last < i {
                    out.push(Span::Text(text[last..i].to_string()));
                }
                out.push(Span::Comment(text[i + 2..end].to_string()));
                i = end + 2;
                last = i;
                continue;
            }
        }
        // `==…==` highlight
        if i + 1 < bytes.len() && bytes[i] == b'=' && bytes[i + 1] == b'=' {
            if let Some(end) = find_close(bytes, i + 2, b"==") {
                if end > i + 2 {
                    if last < i {
                        out.push(Span::Text(text[last..i].to_string()));
                    }
                    out.push(Span::Highlight(text[i + 2..end].to_string()));
                    i = end + 2;
                    last = i;
                    continue;
                }
            }
        }
        // `$…$` inline math (not `$$`)
        if bytes[i] == b'$' && (i + 1 >= bytes.len() || bytes[i + 1] != b'$') {
            if let Some(end) = find_dollar_close(bytes, i + 1) {
                if end > i + 1 {
                    if last < i {
                        out.push(Span::Text(text[last..i].to_string()));
                    }
                    out.push(Span::InlineMath {
                        formula: text[i + 1..end].to_string(),
                    });
                    i = end + 1;
                    last = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if last < text.len() {
        out.push(Span::Text(text[last..].to_string()));
    }
    if out.is_empty() {
        out.push(Span::Text(text.to_string()));
    }
    out
}

fn find_close(bytes: &[u8], from: usize, delim: &[u8]) -> Option<usize> {
    let d0 = delim[0];
    let d1 = delim[1];
    let mut j = from;
    while j + 1 < bytes.len() {
        if bytes[j] == d0 && bytes[j + 1] == d1 {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_dollar_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == b'$' {
            // Don't treat `$$` as a closer for inline math.
            if j + 1 < bytes.len() && bytes[j + 1] == b'$' {
                return None;
            }
            return Some(j);
        }
        // Stop at newline — inline only.
        if bytes[j] == b'\n' {
            return None;
        }
        j += 1;
    }
    None
}

/// Split plain text into `Text` / `WikiLink` / `Embed` spans.
/// `![[target|alias]]` becomes [`Span::Embed`]; `[[…]]` without `!` stays [`Span::WikiLink`].
pub fn split_wikilinks(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let is_embed = i > 0 && bytes[i - 1] == b'!';
            let text_end = if is_embed { i - 1 } else { i };
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    if last < text_end {
                        out.push(Span::Text(text[last..text_end].to_string()));
                    }
                    let raw = &text[start..j];
                    if is_embed {
                        let (target, alias, width, height) = match raw.split_once('|') {
                            Some((t, a)) => {
                                let (alias, width, height) = alias_or_size(a);
                                (t.to_string(), alias, width, height)
                            }
                            None => (raw.to_string(), None, None, None),
                        };
                        out.push(Span::Embed {
                            target,
                            alias,
                            width,
                            height,
                        });
                    } else {
                        let alias = raw.split_once('|').map(|(_, a)| a.to_string());
                        out.push(Span::WikiLink {
                            raw: raw.to_string(),
                            alias,
                        });
                    }
                    i = j + 2;
                    last = i;
                    break;
                }
                j += 1;
            }
            if j + 1 >= bytes.len() {
                break;
            }
        } else {
            i += 1;
        }
    }
    if last < text.len() {
        out.push(Span::Text(text[last..].to_string()));
    }
    if out.is_empty() {
        out.push(Span::Text(text.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_and_lists() {
        let doc = parse_preview("# Hi\n\n- a\n- b\n\n```rust\nlet x = 1;\n```\n");
        assert!(doc
            .blocks
            .iter()
            .any(|b| matches!(b, PreviewBlock::Heading { level: 1, text } if text == "Hi")));
        assert!(doc
            .blocks
            .iter()
            .any(|b| matches!(b, PreviewBlock::BulletItem { .. })));
        assert!(doc
            .blocks
            .iter()
            .any(|b| matches!(b, PreviewBlock::CodeBlock { .. })));
    }

    #[test]
    fn parses_wikilink_despite_fragmented_text_events() {
        let doc = parse_preview("see [[Hello]] now");
        let spans = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                PreviewBlock::Paragraph { spans } => Some(spans.clone()),
                _ => None,
            })
            .expect("paragraph");
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::WikiLink { raw, .. } if raw == "Hello")),
            "spans={spans:?}"
        );
    }

    #[test]
    fn parses_lone_wikilink() {
        let doc = parse_preview("[[Hello]]");
        let has = doc.blocks.iter().any(|b| match b {
            PreviewBlock::Paragraph { spans } => spans
                .iter()
                .any(|s| matches!(s, Span::WikiLink { raw, .. } if raw == "Hello")),
            _ => false,
        });
        assert!(has, "blocks={:?}", doc.blocks);
    }

    #[test]
    fn parses_embed_attachment() {
        let doc = parse_preview("see ![[attachments/x.png]] now");
        let spans = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                PreviewBlock::Paragraph { spans } => Some(spans.clone()),
                _ => None,
            })
            .expect("paragraph");
        assert!(
            spans.iter().any(|s| matches!(
                s,
                Span::Embed {
                    target,
                    alias: None,
                    width: None,
                    height: None,
                } if target == "attachments/x.png"
            )),
            "spans={spans:?}"
        );
        // Leading `!` must not remain as text glued to the embed.
        assert!(
            !spans.iter().any(|s| matches!(s, Span::Text(t) if t.ends_with('!'))),
            "spans={spans:?}"
        );
    }

    #[test]
    fn parses_embed_with_alias() {
        let spans = split_wikilinks("![[photo.png|cap]]");
        assert_eq!(
            spans,
            vec![Span::Embed {
                target: "photo.png".into(),
                alias: Some("cap".into()),
                width: None,
                height: None,
            }]
        );
    }

    #[test]
    fn parses_embed_width() {
        let spans = split_wikilinks("![[a.png|400]]");
        assert_eq!(
            spans,
            vec![Span::Embed {
                target: "a.png".into(),
                alias: None,
                width: Some(400),
                height: None,
            }]
        );
    }

    #[test]
    fn parses_embed_width_height() {
        let spans = split_wikilinks("![[a.png|400x200]]");
        assert_eq!(
            spans,
            vec![Span::Embed {
                target: "a.png".into(),
                alias: None,
                width: Some(400),
                height: Some(200),
            }]
        );
        let spans = split_wikilinks("![[a.png|400X200]]");
        assert!(matches!(
            &spans[..],
            [Span::Embed {
                width: Some(400),
                height: Some(200),
                ..
            }]
        ));
    }

    #[test]
    fn parses_embed_caption_not_size() {
        let spans = split_wikilinks("![[a.png|caption]]");
        assert_eq!(
            spans,
            vec![Span::Embed {
                target: "a.png".into(),
                alias: Some("caption".into()),
                width: None,
                height: None,
            }]
        );
    }

    #[test]
    fn parse_embed_size_rejects_zero() {
        assert!(parse_embed_size("0").is_none());
        assert!(parse_embed_size("10x0").is_none());
    }

    #[test]
    fn parses_markdown_image() {
        let doc = parse_preview("![alt text](https://example.com/a.png)");
        let spans = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                PreviewBlock::Paragraph { spans } => Some(spans.clone()),
                _ => None,
            })
            .expect("paragraph");
        assert!(
            spans.iter().any(|s| matches!(
                s,
                Span::Image { dest, alt, width: None, height: None }
                    if dest == "https://example.com/a.png" && alt == "alt text"
            )),
            "spans={spans:?}"
        );
    }

    #[test]
    fn parses_markdown_image_size_in_alt() {
        let doc = parse_preview("![shot|320](https://example.com/a.png)");
        let spans = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                PreviewBlock::Paragraph { spans } => Some(spans.clone()),
                _ => None,
            })
            .expect("paragraph");
        assert!(
            spans.iter().any(|s| matches!(
                s,
                Span::Image {
                    dest,
                    alt,
                    width: Some(320),
                    height: None,
                } if dest == "https://example.com/a.png" && alt == "shot"
            )),
            "spans={spans:?}"
        );
    }

    #[test]
    fn wiki_without_bang_stays_wikilink() {
        let spans = split_wikilinks("[[attachments/x.png]]");
        assert!(matches!(
            &spans[..],
            [Span::WikiLink { raw, alias: None }] if raw == "attachments/x.png"
        ));
    }

    fn para_spans(doc: &PreviewDoc) -> Vec<Span> {
        doc.blocks
            .iter()
            .find_map(|b| match b {
                PreviewBlock::Paragraph { spans } => Some(spans.clone()),
                _ => None,
            })
            .expect("paragraph")
    }

    #[test]
    fn parses_strikethrough() {
        let spans = para_spans(&parse_preview("a ~~gone~~ b"));
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Strike(t) if t == "gone")),
            "spans={spans:?}"
        );
    }

    #[test]
    fn parses_highlight() {
        let spans = para_spans(&parse_preview("see ==hot== now"));
        assert!(
            spans.iter().any(|s| matches!(s, Span::Highlight(t) if t == "hot")),
            "spans={spans:?}"
        );
    }

    #[test]
    fn comment_emitted_as_span() {
        let spans = para_spans(&parse_preview("keep %%secret%% visible"));
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Comment(t) if t == "secret")),
            "spans={spans:?}"
        );
        assert!(
            spans.iter().any(|s| matches!(s, Span::Text(t) if t.contains("keep"))),
            "spans={spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Text(t) if t.contains("visible"))),
            "spans={spans:?}"
        );
        assert!(
            !spans
                .iter()
                .any(|s| matches!(s, Span::Text(t) if t.contains("secret"))),
            "spans={spans:?}"
        );
    }

    #[test]
    fn comment_unclosed_stays_text() {
        let spans = para_spans(&parse_preview("keep %%open"));
        assert!(
            !spans.iter().any(|s| matches!(s, Span::Comment(_))),
            "spans={spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Text(t) if t.contains("%%open"))),
            "spans={spans:?}"
        );
    }

    #[test]
    fn comment_first_closer_wins() {
        let spans = para_spans(&parse_preview("%% a %% b %%"));
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Comment(t) if t == " a ")),
            "spans={spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Span::Text(t) if t.contains("b") && t.contains("%%"))),
            "spans={spans:?}"
        );
    }

    #[test]
    fn parses_inline_math() {
        let spans = para_spans(&parse_preview("e = $mc^2$ end"));
        assert!(
            spans.iter().any(|s| matches!(
                s,
                Span::InlineMath { formula } if formula == "mc^2"
            )),
            "spans={spans:?}"
        );
    }
}
