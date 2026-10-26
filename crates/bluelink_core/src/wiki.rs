use std::collections::{HashMap, HashSet};

use crate::frontmatter::aliases_from_frontmatter;

/// Heading text or `^id` after `#` in a wiki link body (`Note#H|a` → `H`).
pub fn link_fragment(raw: &str) -> Option<String> {
    let without_alias = raw.split('|').next().unwrap_or(raw);
    let frag = without_alias.split_once('#')?.1.trim();
    if frag.is_empty() {
        None
    } else {
        Some(frag.to_string())
    }
}

/// 0-based line of an ATX heading whose title equals `heading` (ASCII case-insensitive).
pub fn heading_line(text: &str, heading: &str) -> Option<usize> {
    for (i, line) in text.lines().enumerate() {
        let Some(title) = atx_heading_title(line) else {
            continue;
        };
        if title.eq_ignore_ascii_case(heading) {
            return Some(i);
        }
    }
    None
}

/// 0-based line of a block id (`^id` alone or trailing). `id` may include the `^`.
pub fn block_line(text: &str, id: &str) -> Option<usize> {
    let needle = if id.starts_with('^') {
        id.to_string()
    } else {
        format!("^{id}")
    };
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_end();
        if t == needle || t.ends_with(&format!(" {needle}")) {
            return Some(i);
        }
    }
    None
}

/// Rewrite `#old` → `#new` on wiki links whose target is `note_stem` (or empty `[[#old]]`).
pub fn rewrite_heading_links(content: &str, note_stem: &str, old: &str, new: &str) -> String {
    if old == new {
        return content.to_string();
    }
    let bytes = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    let mut last = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    out.push_str(&content[last..i]);
                    let body = &content[start..j];
                    let trimmed = body.trim();
                    let replacement = rewrite_heading_in_body(trimmed, note_stem, old, new);
                    out.push_str("[[");
                    out.push_str(replacement.as_deref().unwrap_or(body));
                    out.push_str("]]");
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
    out.push_str(&content[last..]);
    out
}

fn atx_heading_title(line: &str) -> Option<&str> {
    let t = line.trim();
    if !t.starts_with('#') {
        return None;
    }
    let after_hashes = t.trim_start_matches('#');
    let level = t.len() - after_hashes.len();
    if level < 1 || level > 6 {
        return None;
    }
    let title = after_hashes.trim();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

fn rewrite_heading_in_body(body: &str, note_stem: &str, old: &str, new: &str) -> Option<String> {
    let (before_alias, alias) = match body.split_once('|') {
        Some((path_heading, alias)) => (path_heading, Some(alias)),
        None => (body, None),
    };
    let hash = before_alias.find('#')?;
    let target = before_alias[..hash].trim();
    let heading = before_alias[hash + 1..].trim();
    if !heading.eq_ignore_ascii_case(old) {
        return None;
    }
    if !target_matches_stem(target, note_stem) {
        return None;
    }
    let mut out = String::new();
    out.push_str(target);
    out.push('#');
    out.push_str(new);
    if let Some(a) = alias {
        out.push('|');
        out.push_str(a);
    }
    Some(out)
}

fn target_matches_stem(target: &str, note_stem: &str) -> bool {
    // Empty target: `[[#heading]]` same-note form (rewritten in every scanned file; ponytail).
    if target.is_empty() {
        return true;
    }
    let t = normalize_path_separators(target);
    let t = strip_md_suffix(&t);
    t == note_stem || file_stem(&t) == note_stem || t.ends_with(&format!("/{note_stem}"))
}

/// Display form for a renamed note's wiki target (no `#` / `|`).
///
/// Obsidian `MetadataCache.fileToLinktext` (shortest): unique basename, else
/// vault-relative path; both without `.md`.
pub fn preferred_wikilink_text(target_rel_path: &str, notes_after: &HashSet<String>) -> String {
    let path = normalize_path_separators(target_rel_path);
    let without_md = strip_md_suffix(&path);
    let base = file_stem(&path);
    let lookup = format!("{base}.md");
    let unique = notes_after
        .iter()
        .filter(|n| file_name(n).eq_ignore_ascii_case(&lookup))
        .count()
        == 1;
    if unique {
        base.to_string()
    } else {
        without_md
    }
}

/// Rebuild a wikilink body with a new path display, keeping `#heading` and `|alias`.
pub fn rewrite_wikilink_body(old_body: &str, new_target_display: &str) -> String {
    let (before_alias, alias) = match old_body.split_once('|') {
        Some((path_heading, alias)) => (path_heading, Some(alias)),
        None => (old_body, None),
    };
    let heading = before_alias.find('#').map(|i| &before_alias[i..]);
    let mut out = new_target_display.to_string();
    if let Some(h) = heading {
        out.push_str(h);
    }
    if let Some(a) = alias {
        out.push('|');
        out.push_str(a);
    }
    out
}

/// Rewrite `[[wikilinks]]` (and embeds `![[...]]`) whose resolve target is in `renames`.
///
/// `renames`: old rel_path → new rel_path. `notes_after` is the note set after those renames
/// (used for unique-basename vs path display). Resolution uses the pre-rename `index`.
pub fn rewrite_wikilinks_in_content(
    content: &str,
    source_path: &str,
    index: &WikiIndex,
    renames: &HashMap<String, String>,
    notes_after: &HashSet<String>,
) -> String {
    if renames.is_empty() {
        return content.to_string();
    }

    let bytes = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    let mut last = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    out.push_str(&content[last..i]);
                    let body = &content[start..j];
                    let trimmed = body.trim();
                    let replacement = if !trimmed.is_empty() {
                        index
                            .resolve(trimmed, Some(source_path))
                            .and_then(|resolved| renames.get(&resolved).cloned())
                            .map(|new_path| {
                                let display = preferred_wikilink_text(&new_path, notes_after);
                                rewrite_wikilink_body(trimmed, &display)
                            })
                    } else {
                        None
                    };
                    out.push_str("[[");
                    match replacement {
                        Some(new_body) => out.push_str(&new_body),
                        None => out.push_str(body),
                    }
                    out.push_str("]]");
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
    out.push_str(&content[last..]);
    out
}

/// Note paths after applying `old → new` renames (for display-form uniqueness).
pub fn notes_after_renames(notes: &[String], renames: &HashMap<String, String>) -> HashSet<String> {
    notes
        .iter()
        .map(|p| renames.get(p).cloned().unwrap_or_else(|| p.clone()))
        .collect()
}

/// 1-hop neighborhood around a note for the local graph view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalGraph {
    pub center: String,
    /// Center first, then neighbors in discovery order (outgoing, then backlinks).
    pub nodes: Vec<String>,
    /// Directed edges `(from, to)` among `nodes` that resolve via wiki links.
    pub edges: Vec<(String, String)>,
}

/// In-memory index of notes and their outgoing wiki links.
#[derive(Debug, Default, Clone)]
pub struct WikiIndex {
    /// rel_path -> link targets as written in [[...]] (before resolve)
    outgoing: HashMap<String, Vec<String>>,
    /// All known note rel_paths (with .md)
    notes: HashSet<String>,
    /// Lowercased frontmatter alias → first note path that claimed it (`ponytail:`).
    aliases: HashMap<String, String>,
}

impl WikiIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.outgoing.clear();
        self.notes.clear();
        self.aliases.clear();
    }

    pub fn set_note(&mut self, rel_path: &str, content: &str) {
        self.forget_aliases_for(rel_path);
        self.notes.insert(rel_path.to_string());
        let links = extract_wikilinks(content);
        self.outgoing.insert(rel_path.to_string(), links);
        for alias in aliases_from_frontmatter(content) {
            let key = alias.to_ascii_lowercase();
            // First note wins when two share an alias.
            self.aliases
                .entry(key)
                .or_insert_with(|| rel_path.to_string());
        }
    }

    pub fn remove_note(&mut self, rel_path: &str) {
        self.forget_aliases_for(rel_path);
        self.notes.remove(rel_path);
        self.outgoing.remove(rel_path);
    }

    pub fn rename_note(&mut self, from: &str, to: &str) {
        if let Some(links) = self.outgoing.remove(from) {
            self.outgoing.insert(to.to_string(), links);
        }
        self.notes.remove(from);
        self.notes.insert(to.to_string());
        for path in self.aliases.values_mut() {
            if path == from {
                *path = to.to_string();
            }
        }
    }

    fn forget_aliases_for(&mut self, rel_path: &str) {
        self.aliases.retain(|_, path| path != rel_path);
    }

    /// Resolve a wiki link like Obsidian `MetadataCache.getFirstLinkpathDest`.
    ///
    /// `source_path` is the note that contains the link (with `.md`). Used for
    /// `./` / `../` and same-folder preference when several notes share a name.
    ///
    /// Rules (parity with Obsidian 1.14):
    /// 1. Strip `|alias` and `#heading`.
    /// 2. Empty target → `source_path` itself.
    /// 3. Match by file name (case-insensitive, `.md` implied); among hits:
    ///    exact path, then `./`/`../` from source, then vault-root `/path`,
    ///    then prefer notes under the source directory, shortest path wins.
    pub fn resolve(&self, link: &str, source_path: Option<&str>) -> Option<String> {
        let target = normalize_path_separators(&normalize_link_target(link));
        if target.is_empty() {
            return source_path
                .filter(|s| self.notes.contains(*s))
                .map(|s| s.to_string());
        }

        // Basename lookup key (Obsidian uniqueFileLookup): name including .md.
        let mut path_lower = target.to_ascii_lowercase();
        let base = file_name(&path_lower);
        let lookup_key = if base.contains('.') {
            base.to_string()
        } else {
            path_lower = format!("{path_lower}.md");
            file_name(&path_lower).to_string()
        };

        let candidates: Vec<&String> = self
            .notes
            .iter()
            .filter(|n| file_name(n).eq_ignore_ascii_case(&lookup_key))
            .collect();
        if candidates.is_empty() {
            // After file-name lookup: frontmatter aliases recorded in `set_note`.
            return self.resolve_alias(&target);
        }

        // Unique basename-only link → that file.
        if lookup_key == path_lower && candidates.len() == 1 {
            return Some(candidates[0].clone());
        }

        let source_dir = source_path.map(parent_dir).unwrap_or_default();
        let source_dir_lower = source_dir.to_ascii_lowercase();

        // Relative ./ ../ — mutate path like Obsidian, then exact-match candidates.
        if path_lower.starts_with("./") || path_lower.starts_with("../") {
            path_lower = resolve_relative(&path_lower, &source_dir_lower);
            if let Some(hit) = find_ci(&candidates, &path_lower) {
                return Some(hit.to_string());
            }
        }

        if let Some(stripped) = path_lower.strip_prefix('/') {
            path_lower = stripped.to_string();
        }

        // Exact vault path among same-name candidates (root `todo.md` beats `a/todo.md`
        // when the link is bare `todo` — same order as Obsidian).
        if let Some(hit) = find_ci(&candidates, &path_lower) {
            return Some(hit.to_string());
        }

        // Absolute-from-vault `/foo` that didn't match → miss (don't fuzzy).
        if target.starts_with('/') {
            return None;
        }

        // Prefer notes under the source folder, then elsewhere; shortest path wins.
        // Boundary-aware prefix (Obsidian uses raw startsWith; we avoid `folder` matching `folder2`).
        let mut near: Vec<&String> = Vec::new();
        let mut far: Vec<&String> = Vec::new();
        for n in &candidates {
            let y = n.to_ascii_lowercase();
            // Path-boundary suffix (avoid `mytodo.md` matching `todo.md`).
            if y != path_lower && !y.ends_with(&format!("/{path_lower}")) {
                continue;
            }
            if under_dir(&y, &source_dir_lower) {
                near.push(n);
            } else {
                far.push(n);
            }
        }
        near.sort_by_key(|p| (p.len(), p.as_str()));
        far.sort_by_key(|p| (p.len(), p.as_str()));
        near.into_iter()
            .chain(far)
            .next()
            .cloned()
            .or_else(|| self.resolve_alias(&target))
    }

    fn resolve_alias(&self, target: &str) -> Option<String> {
        let key = target.to_ascii_lowercase();
        self.aliases.get(&key).cloned()
    }

    pub fn backlinks(&self, rel_path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (src, links) in &self.outgoing {
            for link in links {
                if self.resolve(link, Some(src)).as_deref() == Some(rel_path) {
                    out.push(src.clone());
                    break;
                }
            }
        }
        out.sort();
        out
    }

    /// Local graph: `rel_path` plus resolved outgoing targets and backlinks.
    ///
    /// Node count is capped at `max_nodes` (center always kept). Edges are the
    /// induced subgraph of resolving wiki links among those nodes.
    pub fn local_graph(&self, rel_path: &str, max_nodes: usize) -> LocalGraph {
        let max_nodes = max_nodes.max(1);
        let center = rel_path.to_string();

        let mut nodes = vec![center.clone()];
        let mut seen = HashSet::new();
        seen.insert(center.clone());

        if let Some(links) = self.outgoing.get(rel_path) {
            for link in links {
                let Some(target) = self.resolve(link, Some(rel_path)) else {
                    continue;
                };
                if seen.insert(target.clone()) {
                    nodes.push(target);
                    if nodes.len() >= max_nodes {
                        break;
                    }
                }
            }
        }

        if nodes.len() < max_nodes {
            for src in self.backlinks(rel_path) {
                if seen.insert(src.clone()) {
                    nodes.push(src);
                    if nodes.len() >= max_nodes {
                        break;
                    }
                }
            }
        }

        let mut edges = Vec::new();
        for src in &nodes {
            let Some(links) = self.outgoing.get(src) else {
                continue;
            };
            for link in links {
                let Some(dst) = self.resolve(link, Some(src)) else {
                    continue;
                };
                if seen.contains(&dst) {
                    edges.push((src.clone(), dst));
                }
            }
        }
        edges.sort();
        edges.dedup();

        LocalGraph {
            center,
            nodes,
            edges,
        }
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

/// Path to create for an unresolved wiki link (Obsidian `openLinkText` / `createNewFile`).
///
/// - `[[folder/Note]]` → `folder/Note.md` as written.
/// - `[[Note]]` (no `/`) → same directory as `source_path`, or vault root if none.
pub fn create_path_for_link(link: &str, source_path: Option<&str>) -> String {
    let target = normalize_path_separators(&normalize_link_target(link));
    let with_md = if target.ends_with(".md") {
        target
    } else {
        format!("{target}.md")
    };

    if with_md.contains('/') {
        return with_md.trim_start_matches('/').to_string();
    }

    match source_path.map(parent_dir).filter(|d| !d.is_empty()) {
        Some(dir) => format!("{dir}/{with_md}"),
        None => with_md,
    }
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

fn normalize_path_separators(s: &str) -> String {
    s.replace('\\', "/")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn strip_md_suffix(path: &str) -> String {
    match path.strip_suffix(".md") {
        Some(rest) => rest.to_string(),
        None => path.to_string(),
    }
}

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    name.strip_suffix(".md").unwrap_or(name)
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

fn under_dir(path_lower: &str, dir_lower: &str) -> bool {
    if dir_lower.is_empty() {
        return true;
    }
    path_lower == dir_lower || path_lower.starts_with(&format!("{dir_lower}/"))
}

fn find_ci<'a>(candidates: &[&'a String], want_lower: &str) -> Option<&'a String> {
    candidates
        .iter()
        .find(|n| n.eq_ignore_ascii_case(want_lower))
        .copied()
}

/// Resolve `./` / `../` against a source directory (both already lowercased, `/` seps).
fn resolve_relative(path_lower: &str, source_dir_lower: &str) -> String {
    let mut o = path_lower.to_string();
    let mut l = source_dir_lower.to_string();

    // Obsidian normalizes `./../` → `../`
    if o.starts_with("./../") {
        o = o[2..].to_string();
    }

    if let Some(rest) = o.strip_prefix("./") {
        return if l.is_empty() {
            rest.to_string()
        } else {
            format!("{l}/{rest}")
        };
    }

    while let Some(rest) = o.strip_prefix("../") {
        o = rest.to_string();
        l = parent_dir(&l);
    }
    if l.is_empty() {
        o
    } else {
        format!("{l}/{o}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(notes: &[&str]) -> WikiIndex {
        let mut i = WikiIndex::new();
        for n in notes {
            i.set_note(n, "");
        }
        i
    }

    /// Obsidian 1.14 `MetadataCache.getFirstLinkpathDest` corpus.
    /// Recovered from shipped `app.js` (Obsidian-1.14.5); static, not runtime.
    #[test]
    fn obsidian_114_resolve_corpus() {
        // (notes, link, source, expected)
        let cases: &[(&[&str], &str, Option<&str>, Option<&str>)] = &[
            // unique basename, any folder
            (&["deep/x/Note.md"], "Note", Some("other.md"), Some("deep/x/Note.md")),
            (&["deep/x/Note.md"], "note", None, Some("deep/x/Note.md")),
            (&["deep/x/Note.md"], "NOTE.md", None, Some("deep/x/Note.md")),
            // alias + heading stripped before resolve (BlueLink; Obsidian stores path sans alias)
            (&["Folder/Note.md"], "Folder/Note|алиас", None, Some("Folder/Note.md")),
            (&["Folder/Note.md"], "Folder/Note#Heading", None, Some("Folder/Note.md")),
            (&["Folder/Note.md"], "Folder/Note#H|a", None, Some("Folder/Note.md")),
            // empty / heading-only → source
            (&["a.md"], "", Some("a.md"), Some("a.md")),
            (&["a.md"], "#heading", Some("a.md"), Some("a.md")),
            (&["a.md"], "|alias", Some("a.md"), Some("a.md")),
            (&["a.md"], "", None, None),
            // exact vault-relative path
            (
                &["Folder/Note.md", "note.md"],
                "Folder/Note",
                None,
                Some("Folder/Note.md"),
            ),
            (
                &["Folder/Note.md", "note.md"],
                "folder/note",
                None,
                Some("Folder/Note.md"),
            ),
            // bare name: exact path `todo.md` wins before same-folder preference
            (
                &["a/todo.md", "todo.md"],
                "todo",
                Some("a/other.md"),
                Some("todo.md"),
            ),
            // no root hit → prefer source directory, then shortest elsewhere
            (
                &["a/todo.md", "b/todo.md"],
                "todo",
                Some("a/other.md"),
                Some("a/todo.md"),
            ),
            (
                &["a/todo.md", "b/todo.md"],
                "todo",
                Some("b/x.md"),
                Some("b/todo.md"),
            ),
            // no source → shortest path (RR = path.length)
            (
                &["a/deep/todo.md", "todo.md"],
                "todo",
                None,
                Some("todo.md"),
            ),
            (
                &["a/deep/todo.md", "b/todo.md"],
                "todo",
                None,
                Some("b/todo.md"),
            ),
            // nested path suffix among same basename
            (
                &["x/a/b/todo.md", "a/b/todo.md"],
                "a/b/todo",
                None,
                Some("a/b/todo.md"),
            ),
            (
                &["x/a/b/todo.md"],
                "a/b/todo",
                None,
                Some("x/a/b/todo.md"),
            ),
            // relative ./ ../
            (
                &["proj/alpha.md", "proj/sub/beta.md", "other.md"],
                "./alpha",
                Some("proj/readme.md"),
                Some("proj/alpha.md"),
            ),
            (
                &["proj/alpha.md", "proj/sub/beta.md"],
                "../alpha",
                Some("proj/sub/beta.md"),
                Some("proj/alpha.md"),
            ),
            (
                &["proj/alpha.md", "proj/sub/beta.md"],
                "./beta",
                Some("proj/sub/note.md"),
                Some("proj/sub/beta.md"),
            ),
            (
                &["proj/alpha.md", "proj/sub/beta.md"],
                "./alpha",
                Some("proj/sub/beta.md"),
                None,
            ),
            (
                &["proj/alpha.md", "proj/sub/beta.md"],
                "./../alpha",
                Some("proj/sub/beta.md"),
                Some("proj/alpha.md"),
            ),
            (
                &["top.md", "a/b/c.md"],
                "../../top",
                Some("a/b/c.md"),
                Some("top.md"),
            ),
            // vault-root absolute: miss must not fuzzy
            (
                &["Folder/Note.md", "Note.md"],
                "/Folder/Note",
                Some("Note.md"),
                Some("Folder/Note.md"),
            ),
            (&["Folder/Note.md"], "/missing", Some("Note.md"), None),
            (
                &["a/todo.md", "todo.md"],
                "/todo",
                Some("a/x.md"),
                Some("todo.md"),
            ),
            (
                &["a/todo.md"],
                "/todo",
                Some("a/x.md"),
                None, // absolute miss — no fuzzy to a/todo.md
            ),
            // backslash separators normalized
            (
                &["Folder/Note.md"],
                r"Folder\Note",
                None,
                Some("Folder/Note.md"),
            ),
            // missing
            (&["a.md"], "nope", None, None),
            // deeper same-folder beats far
            (
                &["proj/sub/todo.md", "other/todo.md"],
                "todo",
                Some("proj/sub/x.md"),
                Some("proj/sub/todo.md"),
            ),
            // source in parent: child still "near" via under_dir
            (
                &["proj/sub/todo.md", "other/todo.md"],
                "todo",
                Some("proj/readme.md"),
                Some("proj/sub/todo.md"),
            ),
        ];

        for (notes, link, source, expected) in cases {
            let i = idx(notes);
            let got = i.resolve(link, *source);
            assert_eq!(
                got.as_deref(),
                *expected,
                "link={link:?} source={source:?} notes={notes:?}"
            );
        }
    }

    /// Intentional divergences from Obsidian's raw `endsWith` / `startsWith`.
    /// Obsidian 1.14: `mytodo.md`.endsWith(`todo.md`) and `folder2`.startsWith(`folder`).
    /// BlueLink requires a path-segment boundary.
    #[test]
    fn resolve_rejects_obsidian_false_positive_suffix() {
        let i = idx(&["a/mytodo.md"]);
        assert_eq!(
            i.resolve("todo", Some("a/x.md")),
            None,
            "Obsidian would resolve to a/mytodo.md via endsWith"
        );
    }

    #[test]
    fn resolve_rejects_obsidian_false_positive_folder_prefix() {
        // Source dir `fo` must not treat `folder/todo.md` as "near".
        let i = idx(&["folder/todo.md", "z/todo.md"]);
        assert_eq!(
            i.resolve("todo", Some("fo/x.md")).as_deref(),
            Some("z/todo.md"),
            "near-empty → shortest far; Obsidian would prefer folder/ via startsWith(\"fo\")"
        );
    }

    #[test]
    fn normalize_link_target_strips_alias_and_heading() {
        assert_eq!(normalize_link_target("A|b"), "A");
        assert_eq!(normalize_link_target("A#h"), "A");
        assert_eq!(normalize_link_target("A#h|b"), "A");
        assert_eq!(normalize_link_target("  x  "), "x");
    }

    #[test]
    fn extract_wikilinks_basic() {
        assert_eq!(
            extract_wikilinks("see [[A]] and [[B|x]] [[C#h]]"),
            vec!["A", "B|x", "C#h"]
        );
        assert_eq!(extract_wikilinks("[[  spaced  ]]"), vec!["spaced"]);
        assert!(extract_wikilinks("[[]").is_empty());
        assert!(extract_wikilinks("no links").is_empty());
    }

    #[test]
    fn backlinks_use_source_for_relative() {
        let mut i = WikiIndex::new();
        i.set_note("proj/sub/a.md", "see [[../b]]");
        i.set_note("proj/b.md", "hi");
        i.set_note("b.md", "other");
        assert_eq!(i.backlinks("proj/b.md"), vec!["proj/sub/a.md".to_string()]);
        assert!(i.backlinks("b.md").is_empty());
    }

    #[test]
    fn backlinks_work() {
        let mut i = WikiIndex::new();
        i.set_note("a.md", "see [[b]]");
        i.set_note("b.md", "hi");
        assert_eq!(i.backlinks("b.md"), vec!["a.md".to_string()]);
    }

    #[test]
    fn local_graph_neighbors_of_note() {
        let mut i = WikiIndex::new();
        i.set_note("hub.md", "to [[out]] and [[missing]]");
        i.set_note("out.md", "leaf");
        i.set_note("in.md", "see [[hub]]");
        i.set_note("other.md", "unrelated");

        let g = i.local_graph("hub.md", 30);
        assert_eq!(g.center, "hub.md");
        assert_eq!(g.nodes, vec!["hub.md", "out.md", "in.md"]);
        assert!(g.edges.contains(&("hub.md".into(), "out.md".into())));
        assert!(g.edges.contains(&("in.md".into(), "hub.md".into())));
        assert!(!g.nodes.iter().any(|n| n == "other.md"));

        let capped = i.local_graph("hub.md", 2);
        assert_eq!(capped.nodes.len(), 2);
        assert_eq!(capped.nodes[0], "hub.md");
        assert_eq!(capped.nodes[1], "out.md");
    }

    #[test]
    fn create_path_next_to_source_for_bare_name() {
        assert_eq!(
            create_path_for_link("Note|alias", Some("proj/sub/a.md")),
            "proj/sub/Note.md"
        );
        assert_eq!(create_path_for_link("Note", Some("root.md")), "Note.md");
        assert_eq!(create_path_for_link("Note", None), "Note.md");
        assert_eq!(
            create_path_for_link("folder/Note", Some("proj/a.md")),
            "folder/Note.md"
        );
        assert_eq!(
            create_path_for_link("/Abs", Some("proj/a.md")),
            "Abs.md"
        );
    }

    #[test]
    fn preferred_text_unique_basename_vs_path() {
        let unique: HashSet<String> = ["deep/x/Note.md".into()].into_iter().collect();
        assert_eq!(preferred_wikilink_text("deep/x/Note.md", &unique), "Note");

        let clash: HashSet<String> = ["a/Note.md".into(), "b/Note.md".into()].into_iter().collect();
        assert_eq!(preferred_wikilink_text("a/Note.md", &clash), "a/Note");
        assert_eq!(preferred_wikilink_text("b/Note.md", &clash), "b/Note");
    }

    #[test]
    fn rewrite_body_keeps_heading_and_alias() {
        assert_eq!(rewrite_wikilink_body("Old#H|a", "New"), "New#H|a");
        assert_eq!(rewrite_wikilink_body("folder/Old|алиас", "New"), "New|алиас");
        assert_eq!(rewrite_wikilink_body("Old#H", "path/New"), "path/New#H");
        assert_eq!(rewrite_wikilink_body("Old", "New"), "New");
    }

    #[test]
    fn rewrite_content_relative_and_embed() {
        let mut i = WikiIndex::new();
        i.set_note("proj/b.md", "");
        i.set_note("proj/sub/src.md", "");

        let renames: HashMap<String, String> =
            [("proj/b.md".into(), "proj/renamed.md".into())].into_iter().collect();
        let notes_after = notes_after_renames(
            &["proj/b.md".into(), "proj/sub/src.md".into()],
            &renames,
        );
        let out = rewrite_wikilinks_in_content(
            "see [[../b]] and ![[../b#h|x]] and [[missing]]",
            "proj/sub/src.md",
            &i,
            &renames,
            &notes_after,
        );
        assert_eq!(out, "see [[renamed]] and ![[renamed#h|x]] and [[missing]]");
    }

    #[test]
    fn rewrite_content_basename_clash_uses_path() {
        let mut i = WikiIndex::new();
        i.set_note("a/Note.md", "");
        i.set_note("b/Note.md", "");
        i.set_note("index.md", "[[a/Note#H|alias]] [[b/Note]]");

        let renames: HashMap<String, String> =
            [("a/Note.md".into(), "a/Renamed.md".into())].into_iter().collect();
        let notes_after = notes_after_renames(
            &["a/Note.md".into(), "b/Note.md".into(), "index.md".into()],
            &renames,
        );
        // After rename: a/Renamed.md + b/Note.md — Renamed is unique, Note clash remains for b.
        assert_eq!(
            preferred_wikilink_text("a/Renamed.md", &notes_after),
            "Renamed"
        );

        let out = rewrite_wikilinks_in_content(
            "[[a/Note#H|alias]] [[b/Note]]",
            "index.md",
            &i,
            &renames,
            &notes_after,
        );
        assert_eq!(out, "[[Renamed#H|alias]] [[b/Note]]");
    }

    #[test]
    fn link_fragment_heading_and_block() {
        assert_eq!(link_fragment("Note#Heading|a").as_deref(), Some("Heading"));
        assert_eq!(link_fragment("Note#^bid").as_deref(), Some("^bid"));
        assert_eq!(link_fragment("#Only").as_deref(), Some("Only"));
        assert_eq!(link_fragment("Note"), None);
        assert_eq!(link_fragment("Note#"), None);
    }

    #[test]
    fn heading_and_block_line_find() {
        let text = "# Intro\n\npara\n\n## Target Title\n\nline ^block-a\n\n^solo\n";
        assert_eq!(heading_line(text, "target title"), Some(4));
        assert_eq!(heading_line(text, "missing"), None);
        assert_eq!(block_line(text, "block-a"), Some(6));
        assert_eq!(block_line(text, "^solo"), Some(8));
    }

    #[test]
    fn rewrite_heading_links_stem_and_bare() {
        let content = "see [[MyNote#Old|x]] and [[#Old]] and [[Other#Old]]";
        let out = rewrite_heading_links(content, "MyNote", "Old", "New");
        assert_eq!(out, "see [[MyNote#New|x]] and [[#New]] and [[Other#Old]]");
    }

    #[test]
    fn resolve_uses_frontmatter_alias() {
        let mut i = WikiIndex::new();
        i.set_note(
            "real.md",
            "---\naliases: nickname\n---\nbody\n",
        );
        i.set_note("other.md", "");
        assert_eq!(i.resolve("nickname", None).as_deref(), Some("real.md"));
        // File name still wins over alias.
        i.set_note(
            "nickname.md",
            "---\naliases: ignored-for-file\n---\n",
        );
        assert_eq!(i.resolve("nickname", None).as_deref(), Some("nickname.md"));
    }

    #[test]
    fn alias_first_wins() {
        let mut i = WikiIndex::new();
        i.set_note("a.md", "---\naliases: shared\n---\n");
        i.set_note("b.md", "---\naliases: shared\n---\n");
        assert_eq!(i.resolve("shared", None).as_deref(), Some("a.md"));
    }
}
