/// Byte span of a closed YAML frontmatter fence, if present.
///
/// `yaml` is the interior between opening and closing `---` (may be empty).
/// `body` is everything after the closing fence line.
/// `line_count` includes both fence lines.
fn frontmatter_span(content: &str) -> Option<FrontmatterSpan<'_>> {
    let mut iter = content.split_inclusive('\n');
    let first = iter.next()?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return None;
    }

    let yaml_start = first.len();
    let mut yaml_end = yaml_start;
    let mut body_start = first.len();
    let mut line_count = 1_usize;
    let mut closed = false;
    for line in iter {
        body_start += line.len();
        line_count += 1;
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        yaml_end = body_start;
    }
    if !closed {
        return None;
    }
    Some(FrontmatterSpan {
        yaml: &content[yaml_start..yaml_end],
        body: &content[body_start..],
        line_count,
    })
}

struct FrontmatterSpan<'a> {
    yaml: &'a str,
    body: &'a str,
    line_count: usize,
}

/// Split note text into YAML frontmatter fields and the body that follows.
///
/// Expects an opening `---` on the first line and a closing `---` on its own line.
/// No fence → empty fields and the original `content` as body.
pub fn parse_frontmatter(content: &str) -> (Vec<(String, String)>, &str) {
    match frontmatter_span(content) {
        Some(span) => (parse_yaml_fields(span.yaml), span.body),
        None => (Vec::new(), content),
    }
}

/// Number of lines occupied by a closed frontmatter fence (`---` … `---`), if any.
pub fn frontmatter_line_count(content: &str) -> Option<usize> {
    frontmatter_span(content).map(|s| s.line_count)
}

/// Raw YAML interior between the opening and closing `---` fences, if closed.
pub fn frontmatter_raw_yaml(content: &str) -> Option<&str> {
    frontmatter_span(content).map(|s| s.yaml)
}

/// Rebuild a note from raw YAML interior + body (no leading/trailing fence in `yaml`).
///
/// Empty / whitespace-only `yaml` → `body` unchanged (no fence), matching [`write_frontmatter`].
pub fn splice_frontmatter_yaml(yaml: &str, body: &str) -> String {
    if yaml.trim().is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    out.push_str(yaml.trim_start_matches(['\r', '\n']));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Serialize fields as a frontmatter block and append `body`.
///
/// Empty `fields` → `body` unchanged (no fence).
pub fn write_frontmatter(fields: &[(String, String)], body: &str) -> String {
    if fields.is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    for (key, value) in fields {
        if value.contains('\n') {
            out.push_str(key);
            out.push_str(":\n");
            for line in value.lines() {
                out.push_str("  - ");
                out.push_str(line);
                out.push('\n');
            }
        } else {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push('\n');
        }
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Alias strings from `alias` / `aliases` frontmatter keys.
pub fn aliases_from_frontmatter(content: &str) -> Vec<String> {
    let (fields, _) = parse_frontmatter(content);
    let mut out = Vec::new();
    for (key, value) in fields {
        if !key.eq_ignore_ascii_case("alias") && !key.eq_ignore_ascii_case("aliases") {
            continue;
        }
        out.extend(split_alias_value(&value));
    }
    out
}

fn parse_yaml_fields(yaml: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut lines = yaml.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, after)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let after = after.trim();
        if after.is_empty() {
            let mut items = Vec::new();
            while let Some(next) = lines.peek() {
                let n = next.trim();
                if let Some(item) = n.strip_prefix("- ") {
                    items.push(item.trim().to_string());
                    lines.next();
                } else if n == "-" {
                    items.push(String::new());
                    lines.next();
                } else {
                    break;
                }
            }
            fields.push((key.to_string(), items.join("\n")));
        } else {
            fields.push((key.to_string(), strip_yaml_quotes(after).to_string()));
        }
    }
    fields
}

fn split_alias_value(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if value.contains('\n') {
        return value
            .lines()
            .map(|l| strip_yaml_quotes(l.trim()))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(|s| strip_yaml_quotes(s.trim()))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn strip_yaml_quotes(s: &str) -> &str {
    if s.len() >= 2 {
        let b = s.as_bytes();
        if (b[0] == b'"' && b[s.len() - 1] == b'"') || (b[0] == b'\'' && b[s.len() - 1] == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_frontmatter() {
        let src = "---\ntitle: Hello\naliases: one, two\n---\nbody text\n";
        let (fields, body) = parse_frontmatter(src);
        assert_eq!(
            fields,
            vec![
                ("title".into(), "Hello".into()),
                ("aliases".into(), "one, two".into()),
            ]
        );
        assert_eq!(body, "body text\n");
        let out = write_frontmatter(&fields, body);
        let (fields2, body2) = parse_frontmatter(&out);
        assert_eq!(fields2, fields);
        assert_eq!(body2, body);
    }

    #[test]
    fn no_frontmatter_passthrough() {
        let src = "# Hi\n";
        let (fields, body) = parse_frontmatter(src);
        assert!(fields.is_empty());
        assert_eq!(body, src);
        assert_eq!(write_frontmatter(&[], src), src);
    }

    #[test]
    fn aliases_block_list() {
        let src = "---\naliases:\n  - alpha\n  - beta\n---\n";
        assert_eq!(
            aliases_from_frontmatter(src),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn unclosed_fence_is_not_frontmatter() {
        let src = "---\ntitle: x\nno close\n";
        let (fields, body) = parse_frontmatter(src);
        assert!(fields.is_empty());
        assert_eq!(body, src);
        assert_eq!(frontmatter_line_count(src), None);
        assert_eq!(frontmatter_raw_yaml(src), None);
    }

    #[test]
    fn line_count_and_raw_yaml() {
        let src = "---\ntitle: Hello\naliases:\n  - a\n---\nbody\n";
        assert_eq!(frontmatter_line_count(src), Some(5));
        assert_eq!(
            frontmatter_raw_yaml(src),
            Some("title: Hello\naliases:\n  - a\n")
        );
        assert_eq!(frontmatter_line_count("# no fm\n"), None);
    }

    #[test]
    fn splice_raw_yaml_roundtrip() {
        let src = "---\ntitle: Hello\n---\nbody text\n";
        let yaml = frontmatter_raw_yaml(src).unwrap();
        let body = parse_frontmatter(src).1;
        let out = splice_frontmatter_yaml(yaml, body);
        assert_eq!(out, src);
        assert_eq!(splice_frontmatter_yaml("  \n", "only body\n"), "only body\n");
    }
}
