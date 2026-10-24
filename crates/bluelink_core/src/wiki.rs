use std::collections::{HashMap, HashSet};
use std::path::Path;

/// In-memory index of notes and their outgoing wiki links.
#[derive(Debug, Default, Clone)]
pub struct WikiIndex {
    /// rel_path -> set of link targets as written in [[...]] (before resolve)
    outgoing: HashMap<String, Vec<String>>,
    /// All known note rel_paths (with .md)
    notes: HashSet<String>,
}

impl WikiIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.outgoing.clear();
        self.notes.clear();
    }

    pub fn set_note(&mut self, rel_path: &str, content: &str) {
        self.notes.insert(rel_path.to_string());
        let links = extract_wikilinks(content);
        self.outgoing.insert(rel_path.to_string(), links);
    }

    pub fn remove_note(&mut self, rel_path: &str) {
        self.notes.remove(rel_path);
        self.outgoing.remove(rel_path);
    }

    pub fn rename_note(&mut self, from: &str, to: &str) {
        if let Some(links) = self.outgoing.remove(from) {
            self.outgoing.insert(to.to_string(), links);
        }
        self.notes.remove(from);
        self.notes.insert(to.to_string());
    }

    /// Resolve a wiki link target to a relative path with `.md`, if any.
    ///
    /// Rules:
    /// 1. Exact path match without `.md` (`folder/Note` → `folder/Note.md`).
    /// 2. Else case-insensitive file-name match; if several, shortest path wins.
    /// Part after `|` is alias (display only); after `#` is heading (ignored in MVP).
    pub fn resolve(&self, link: &str) -> Option<String> {
        let target = normalize_link_target(link);

        let with_md = if target.ends_with(".md") {
            target.clone()
        } else {
            format!("{target}.md")
        };
        if self.notes.contains(&with_md) {
            return Some(with_md);
        }

        let needle = Path::new(&target)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&target);
        let needle_lower = needle.to_ascii_lowercase();
        // Strip .md from needle for name compare if present
        let needle_stem = needle_lower.strip_suffix(".md").unwrap_or(&needle_lower);

        let mut best: Option<String> = None;
        for note in &self.notes {
            let stem = Path::new(note)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(note);
            if stem.eq_ignore_ascii_case(needle_stem) {
                match &best {
                    None => best = Some(note.clone()),
                    Some(cur) if note.len() < cur.len() => best = Some(note.clone()),
                    Some(cur) if note.len() == cur.len() && note < cur => {
                        best = Some(note.clone())
                    }
                    _ => {}
                }
            }
        }
        best
    }

    pub fn backlinks(&self, rel_path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (src, links) in &self.outgoing {
            for link in links {
                if self.resolve(link).as_deref() == Some(rel_path) {
                    out.push(src.clone());
                    break;
                }
            }
        }
        out.sort();
        out
    }

    /// All note paths (with `.md`), sorted — for `[[` autocomplete.
    pub fn note_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.notes.iter().cloned().collect();
        out.sort();
        out
    }
}

/// Strip alias (`|`) and heading (`#`) from a raw link body.
pub fn normalize_link_target(link: &str) -> String {
    let without_alias = link.split('|').next().unwrap_or(link);
    let without_heading = without_alias.split('#').next().unwrap_or(without_alias);
    without_heading.trim().to_string()
}

/// Extract `[[...]]` bodies from markdown text (naive scan, good enough for MVP).
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    if let Ok(body) = std::str::from_utf8(&bytes[start..j]) {
                        let trimmed = body.trim();
                        if !trimmed.is_empty() {
                            links.push(trimmed.to_string());
                        }
                    }
                    i = j + 2;
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
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_exact_and_alias() {
        let mut idx = WikiIndex::new();
        idx.set_note("Folder/Note.md", "hello");
        idx.set_note("note.md", "root");

        assert_eq!(
            idx.resolve("Folder/Note").as_deref(),
            Some("Folder/Note.md")
        );
        assert_eq!(
            idx.resolve("Folder/Note|алиас").as_deref(),
            Some("Folder/Note.md")
        );
        assert_eq!(idx.resolve("note").as_deref(), Some("note.md"));
    }

    #[test]
    fn resolve_shortest_on_name_clash() {
        let mut idx = WikiIndex::new();
        idx.set_note("a/deep/todo.md", "");
        idx.set_note("todo.md", "");
        assert_eq!(idx.resolve("todo").as_deref(), Some("todo.md"));
    }

    #[test]
    fn backlinks_work() {
        let mut idx = WikiIndex::new();
        idx.set_note("a.md", "see [[b]]");
        idx.set_note("b.md", "hi");
        assert_eq!(idx.backlinks("b.md"), vec!["a.md".to_string()]);
    }
}
