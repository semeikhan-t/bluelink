//! Find / replace helpers (byte ranges into the note buffer).

/// Non-overlapping matches as `(start, end)` byte ranges. Empty query → no matches.
pub fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(query) {
        let start = from + rel;
        let end = start + query.len();
        out.push((start, end));
        from = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_occurrences() {
        assert_eq!(find_matches("aa x aa", "aa"), vec![(0, 2), (5, 7)]);
    }

    #[test]
    fn empty_query_is_noop() {
        assert!(find_matches("abc", "").is_empty());
    }
}
