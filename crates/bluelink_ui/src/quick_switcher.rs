//! Fast, local search for the quick switcher.
//!
//! Ranking deliberately favours a note's title and path over body text: the
//! switcher is first a navigation tool, and only then a lightweight full-text
//! search.

use bluelink_core::NoteMeta;

const MAX_RESULTS: usize = 24;
const MAX_SNIPPET_CHARS: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub path: String,
    pub title: String,
    pub snippet: Option<String>,
    score: u16,
}

/// Rank notes using their title, relative path, and text content.
///
/// Every whitespace-separated query term must occur somewhere in the note.
/// Empty queries prefer `recent` paths that still exist (Obsidian-style);
/// if `recent` is empty, notes are listed alphabetically.
pub fn search<F>(
    notes: &[NoteMeta],
    query: &str,
    recent: &[String],
    mut read: F,
) -> Vec<SearchResult>
where
    F: FnMut(&str) -> Option<String>,
{
    let terms = query_terms(query);
    if terms.is_empty() && !recent.is_empty() {
        let mut matches = Vec::new();
        for path in recent {
            let Some(note) = notes.iter().find(|n| n.rel_path == *path) else {
                continue;
            };
            matches.push(result_from_note(note, None, 0));
            if matches.len() >= MAX_RESULTS {
                break;
            }
        }
        if !matches.is_empty() {
            return matches;
        }
    }

    let mut matches = Vec::new();

    for note in notes {
        let title = display_title(note);
        let haystack_title = title.to_lowercase();
        let haystack_path = note.rel_path.to_lowercase();
        let content = if terms.is_empty() {
            None
        } else {
            read(&note.rel_path)
        };
        let haystack_content = content.as_ref().map(|text| text.to_lowercase());

        if !terms.iter().all(|term| {
            haystack_title.contains(term)
                || haystack_path.contains(term)
                || haystack_content
                    .as_ref()
                    .is_some_and(|text| text.contains(term))
        }) {
            continue;
        }

        let score = score(
            &haystack_title,
            &haystack_path,
            haystack_content.as_deref(),
            &terms,
        );
        let snippet = content
            .as_deref()
            .and_then(|text| snippet_for(text, &terms));
        matches.push(SearchResult {
            path: note.rel_path.clone(),
            title,
            snippet,
            score,
        });
    }

    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
    matches.truncate(MAX_RESULTS);
    matches
}

fn display_title(note: &NoteMeta) -> String {
    let title = note.title.trim();
    if title.is_empty() {
        fallback_title(&note.rel_path)
    } else {
        title.to_string()
    }
}

fn result_from_note(note: &NoteMeta, snippet: Option<String>, score: u16) -> SearchResult {
    SearchResult {
        path: note.rel_path.clone(),
        title: display_title(note),
        snippet,
        score,
    }
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|term| !term.is_empty())
        .collect()
}

fn score(title: &str, path: &str, content: Option<&str>, terms: &[String]) -> u16 {
    if terms.is_empty() {
        return 0;
    }

    terms.iter().fold(0, |score, term| {
        let rank = if title == term {
            500
        } else if title.starts_with(term) {
            380
        } else if title.contains(term) {
            300
        } else if path.starts_with(term) {
            220
        } else if path.contains(term) {
            160
        } else if content.is_some_and(|text| text.contains(term)) {
            80
        } else {
            0
        };
        score + rank
    })
}

fn snippet_for(content: &str, terms: &[String]) -> Option<String> {
    let line = content.lines().find(|line| {
        let normalized = line.to_lowercase();
        terms.iter().any(|term| normalized.contains(term))
    })?;
    let line = line.trim().trim_start_matches('#').trim();
    if line.is_empty() {
        return None;
    }
    Some(ellipsize(line, MAX_SNIPPET_CHARS))
}

fn fallback_title(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .into()
}

pub(crate) fn ellipsize(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let short: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, title: &str) -> NoteMeta {
        NoteMeta {
            rel_path: path.into(),
            title: title.into(),
            sha256: String::new(),
            size: 0,
            modified: 0,
        }
    }

    #[test]
    fn title_match_outranks_body_match() {
        let notes = vec![note("inbox.md", "Inbox"), note("ideas.md", "Ideas")];
        let results = search(&notes, "inbox", &[], |path| {
            (path == "ideas.md").then(|| "An inbox item lives here".into())
        });

        assert_eq!(
            results.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["inbox.md", "ideas.md"]
        );
    }

    #[test]
    fn all_terms_must_match_and_body_result_has_snippet() {
        let notes = vec![
            note("project.md", "Project plan"),
            note("other.md", "Other"),
        ];
        let results = search(&notes, "project bluetooth", &[], |path| {
            (path == "project.md").then(|| "Pair over Bluetooth LE nearby".into())
        });

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "project.md");
        assert_eq!(
            results[0].snippet.as_deref(),
            Some("Pair over Bluetooth LE nearby")
        );
    }

    #[test]
    fn empty_query_lists_notes_by_title() {
        let notes = vec![note("z.md", "Zulu"), note("a.md", "Alpha")];
        let results = search(&notes, "", &[], |_| None);

        assert_eq!(
            results.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Alpha", "Zulu"]
        );
    }

    #[test]
    fn empty_query_prefers_recent_order() {
        let notes = vec![
            note("a.md", "Alpha"),
            note("b.md", "Bravo"),
            note("c.md", "Charlie"),
        ];
        let recent = vec!["c.md".into(), "a.md".into(), "gone.md".into()];
        let results = search(&notes, "", &recent, |_| None);

        assert_eq!(
            results.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["c.md", "a.md"]
        );
    }
}
