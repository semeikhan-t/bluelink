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
    Code(String),
    Link { text: String, dest: String },
    WikiLink { raw: String, alias: Option<String> },
}

/// Parse markdown into a preview document. Wikilinks `[[...]]` become `Span::WikiLink`.
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

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading_level = Some(level as u8);
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = heading_level.take() {
                    blocks.push(PreviewBlock::Heading {
                        level,
                        text: std::mem::take(&mut heading_text),
                    });
                }
            }
            Event::Start(Tag::Paragraph) => {
                if quote_spans.is_some() {
                    // paragraph inside quote — reuse quote buffer
                } else {
                    para_spans = Some(Vec::new());
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(spans) = para_spans.take() {
                    blocks.push(PreviewBlock::Paragraph { spans });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                quote_spans = Some(Vec::new());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
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
                list_item_spans = Some(Vec::new());
                list_item_number = next_ordered;
                if let Some(n) = next_ordered.as_mut() {
                    *n += 1;
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(spans) = list_item_spans.take() {
                    if let Some(number) = list_item_number.take() {
                        blocks.push(PreviewBlock::NumberedItem { number, spans });
                    } else {
                        blocks.push(PreviewBlock::BulletItem { spans });
                    }
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
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
            Event::Rule => blocks.push(PreviewBlock::ThematicBreak),
            Event::Start(Tag::Strong) => strong_buf = Some(String::new()),
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
            Event::Start(Tag::Emphasis) => em_buf = Some(String::new()),
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
            Event::Code(code) => {
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
                // Collect link text via subsequent Text events into strong/em style buffer reuse:
                // use strong_buf as temporary link text accumulator keyed by dest in a side channel.
                // Simpler: append as Text for now and rewrite — keep dest in em_buf slot via pair.
                let _ = dest_url;
                strong_buf = Some(String::new());
            }
            Event::End(TagEnd::Link) => {
                // Link without capturing dest properly — handled via Text below as plain.
                // Upgrade: store pending link dest.
                if let Some(text) = strong_buf.take() {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        heading_level.is_some(),
                        Span::Text(text),
                    );
                }
            }
            Event::Text(text) => {
                let s = text.into_string();
                if in_code {
                    code_buf.push_str(&s);
                } else if let Some(buf) = strong_buf.as_mut().or(em_buf.as_mut()) {
                    buf.push_str(&s);
                } else if heading_level.is_some() {
                    heading_text.push_str(&s);
                } else {
                    for piece in split_wikilinks(&s) {
                        push_span(
                            &mut para_spans,
                            &mut quote_spans,
                            &mut list_item_spans,
                            &mut heading_text,
                            false,
                            piece,
                        );
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if heading_level.is_some() {
                    heading_text.push(' ');
                } else {
                    push_span(
                        &mut para_spans,
                        &mut quote_spans,
                        &mut list_item_spans,
                        &mut heading_text,
                        false,
                        Span::Text("\n".into()),
                    );
                }
            }
            _ => {}
        }
    }

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
        if let Span::Text(t) | Span::Strong(t) | Span::Emphasis(t) | Span::Code(t) = &span {
            heading.push_str(t);
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

/// Split plain text into `Text` / `WikiLink` spans.
pub fn split_wikilinks(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    if last < i {
                        out.push(Span::Text(text[last..i].to_string()));
                    }
                    let raw = &text[start..j];
                    let alias = raw.split_once('|').map(|(_, a)| a.to_string());
                    out.push(Span::WikiLink {
                        raw: raw.to_string(),
                        alias,
                    });
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
}
