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

/// Replace the `index`-th match (0-based). Returns false if out of range or query empty.
pub fn replace_one(text: &mut String, query: &str, replacement: &str, index: usize) -> bool {
    let Some(&(start, end)) = find_matches(text, query).get(index) else {
        return false;
    };
    text.replace_range(start..end, replacement);
    true
}

/// Replace every match. Returns how many were replaced.
pub fn replace_all(text: &mut String, query: &str, replacement: &str) -> usize {
    let matches = find_matches(text, query);
    let n = matches.len();
    for &(start, end) in matches.iter().rev() {
        text.replace_range(start..end, replacement);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_occurrences() {
        assert_eq!(find_matches("aa x aa", "aa"), vec![(0, 2), (5, 7)]);
    }

    #[test]
    fn replace_one_and_all() {
        let mut s = String::from("aa x aa");
        assert!(replace_one(&mut s, "aa", "bb", 1));
        assert_eq!(s, "aa x bb");
        assert_eq!(replace_all(&mut s, "aa", "cc"), 1);
        assert_eq!(s, "cc x bb");
        assert_eq!(replace_all(&mut s, "zz", "q"), 0);
    }

    #[test]
    fn empty_query_is_noop() {
        assert!(find_matches("abc", "").is_empty());
        let mut s = String::from("abc");
        assert!(!replace_one(&mut s, "", "x", 0));
        assert_eq!(replace_all(&mut s, "", "x"), 0);
        assert_eq!(s, "abc");
    }
}
