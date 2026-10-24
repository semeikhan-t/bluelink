use std::collections::{HashMap, HashSet};

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
            return None;
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
        near.into_iter().chain(far).next().cloned()
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

    #[test]
    fn resolve_exact_and_alias() {
        let i = idx(&["Folder/Note.md", "note.md"]);
        assert_eq!(
            i.resolve("Folder/Note", None).as_deref(),
            Some("Folder/Note.md")
        );
        assert_eq!(
            i.resolve("Folder/Note|алиас", None).as_deref(),
            Some("Folder/Note.md")
        );
        assert_eq!(i.resolve("note", None).as_deref(), Some("note.md"));
    }

    #[test]
    fn resolve_shortest_on_name_clash_without_source() {
        let i = idx(&["a/deep/todo.md", "todo.md"]);
        assert_eq!(i.resolve("todo", None).as_deref(), Some("todo.md"));
    }

    #[test]
    fn resolve_prefers_same_folder_when_source_set() {
        // No root `todo.md`: exact path miss → same-folder preference.
        let i = idx(&["a/todo.md", "b/todo.md"]);
        assert_eq!(
            i.resolve("todo", Some("a/other.md")).as_deref(),
            Some("a/todo.md")
        );
        assert_eq!(
            i.resolve("todo", Some("b/x.md")).as_deref(),
            Some("b/todo.md")
        );
    }

    #[test]
    fn resolve_exact_path_beats_same_folder() {
        // Bare `todo` matches root path exactly before folder preference.
        let i = idx(&["a/todo.md", "todo.md"]);
        assert_eq!(
            i.resolve("todo", Some("a/other.md")).as_deref(),
            Some("todo.md")
        );
    }

    #[test]
    fn resolve_relative_dot_paths() {
        let i = idx(&["proj/alpha.md", "proj/sub/beta.md", "other.md"]);
        // `./` is same directory as the source note.
        assert_eq!(
            i.resolve("./alpha", Some("proj/readme.md")).as_deref(),
            Some("proj/alpha.md")
        );
        assert_eq!(
            i.resolve("../alpha", Some("proj/sub/beta.md")).as_deref(),
            Some("proj/alpha.md")
        );
        assert_eq!(
            i.resolve("./beta", Some("proj/sub/note.md")).as_deref(),
            Some("proj/sub/beta.md")
        );
        // `./alpha` from sub/ looks for sub/alpha.md — miss.
        assert_eq!(i.resolve("./alpha", Some("proj/sub/beta.md")), None);
    }

    #[test]
    fn resolve_vault_root_absolute() {
        let i = idx(&["Folder/Note.md", "Note.md"]);
        assert_eq!(
            i.resolve("/Folder/Note", Some("Note.md")).as_deref(),
            Some("Folder/Note.md")
        );
        assert_eq!(i.resolve("/missing", Some("Note.md")), None);
    }

    #[test]
    fn resolve_empty_is_source() {
        let i = idx(&["a.md"]);
        assert_eq!(i.resolve("", Some("a.md")).as_deref(), Some("a.md"));
        assert_eq!(i.resolve("#heading", Some("a.md")).as_deref(), Some("a.md"));
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
        assert_eq!(
            create_path_for_link("Note", Some("root.md")),
            "Note.md"
        );
        assert_eq!(create_path_for_link("Note", None), "Note.md");
        assert_eq!(
            create_path_for_link("folder/Note", Some("proj/a.md")),
            "folder/Note.md"
        );
    }
}
