//! Session folds for headings and indented blocks.
//!
//! A fold hides every line after its header until the region ends. The header
//! stays visible. Callout `+`/`-` is stored in the note (see `preview`), not here.
//!
//! ponytail: closed folds are line indexes for this note. Edits that pass
//! `shift_lines` keep them; a reload that changes lines above a fold drops it
//! on the next `retain_valid`. Indent regions stop at a blank line or a fence.
//! A find jump unfolds ancestors through `scroll_to_line`. Replace shifts these
//! indexes; note undo and redo drop them.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    /// Exclusive. Hidden lines are `start + 1 .. end`.
    pub end: usize,
}

/// Foldable heading sections and indent blocks. `in_fence` is true for ``` lines.
pub fn regions(lines: &[&str], in_fence: &[bool]) -> Vec<Region> {
    let n = lines.len();
    let mut out = Vec::new();
    let mut headings = Vec::new();
    for i in 0..n {
        if fenced(in_fence, i) {
            continue;
        }
        if let Some(level) = atx_level(lines[i]) {
            headings.push((i, level));
        }
    }
    for (k, &(start, level)) in headings.iter().enumerate() {
        let end = headings[k + 1..]
            .iter()
            .find(|&&(_, next)| next <= level)
            .map(|&(line, _)| line)
            .unwrap_or(n);
        if end > start + 1 {
            out.push(Region { start, end });
        }
    }

    for i in 0..n {
        if fenced(in_fence, i) || atx_level(lines[i]).is_some() {
            continue;
        }
        if crate::preview::callout_type(lines[i]).is_some() {
            continue;
        }
        let base = indent_cols(lines[i]);
        let mut end = i + 1;
        let mut nested = false;
        while end < n {
            if fenced(in_fence, end) || lines[end].trim().is_empty() {
                break;
            }
            if indent_cols(lines[end]) <= base {
                break;
            }
            nested = true;
            end += 1;
        }
        if nested {
            out.push(Region { start: i, end });
        }
    }
    out
}

pub fn hidden(n: usize, regions: &[Region], closed: &[usize]) -> Vec<bool> {
    let mut out = vec![false; n];
    for region in regions {
        if !closed.contains(&region.start) {
            continue;
        }
        let last = region.end.min(n);
        for line in out.iter_mut().take(last).skip(region.start + 1) {
            *line = true;
        }
    }
    out
}

/// Drop indexes that are no longer the start of a region.
pub fn retain_valid(closed: &mut Vec<usize>, regions: &[Region]) {
    closed.retain(|start| regions.iter().any(|region| region.start == *start));
    closed.sort_unstable();
    closed.dedup();
}

/// Lines at and after `at` move by `delta`. A negative delta deletes that span.
pub fn shift_lines(closed: &mut Vec<usize>, at: usize, delta: i32) {
    if delta == 0 {
        return;
    }
    let mut next = Vec::with_capacity(closed.len());
    for start in closed.drain(..) {
        if start < at {
            next.push(start);
            continue;
        }
        if delta > 0 {
            next.push(start + delta as usize);
            continue;
        }
        let removed = (-delta) as usize;
        if start >= at + removed {
            next.push(start - removed);
        }
    }
    *closed = next;
}

/// How many visual lines `new` has compared to `old`.
/// A slice without a trailing newline counts one line per `\n`, plus the tail.
/// A trailing newline is the terminator, not an extra line. Empty is zero lines.
pub fn line_delta(old: &str, new: &str) -> i32 {
    spanned_lines(new) - spanned_lines(old)
}

/// Open every closed region that hides `line` (the header itself stays put).
pub fn unfold_containing(closed: &mut Vec<usize>, regions: &[Region], line: usize) {
    closed.retain(|start| {
        !regions
            .iter()
            .any(|region| region.start == *start && region.start < line && line < region.end)
    });
}

/// Collapse the smallest open region that covers `line`. Returns whether one closed.
pub fn fold_more(regions: &[Region], closed: &mut Vec<usize>, line: usize) -> bool {
    let Some(start) = pick(regions, closed, line, false) else {
        return false;
    };
    closed.push(start);
    true
}

/// Open the smallest closed region that covers `line`.
pub fn fold_less(regions: &[Region], closed: &mut Vec<usize>, line: usize) -> bool {
    let Some(start) = pick(regions, closed, line, true) else {
        return false;
    };
    closed.retain(|line| *line != start);
    true
}

/// Next visible line in `down` direction, skipping closed bodies.
pub fn step_visible(hidden: &[bool], line: usize, down: bool) -> Option<usize> {
    let n = hidden.len() as isize;
    if n == 0 {
        return None;
    }
    let dir: isize = if down { 1 } else { -1 };
    let mut i = line as isize;
    loop {
        i += dir;
        if i < 0 || i >= n {
            return None;
        }
        let idx = i as usize;
        if !hidden[idx] {
            return Some(idx);
        }
    }
}

pub fn first_visible(hidden: &[bool]) -> Option<usize> {
    hidden.iter().position(|line| !*line)
}

pub fn last_visible(hidden: &[bool]) -> Option<usize> {
    hidden.iter().rposition(|line| !*line)
}

/// Header to show when `line` sits inside a closed region. Outermost wins.
pub fn visible_anchor(regions: &[Region], closed: &[usize], line: usize) -> Option<usize> {
    regions
        .iter()
        .filter(|region| closed.contains(&region.start) && region.start < line && line < region.end)
        .min_by_key(|region| region.start)
        .map(|region| region.start)
}

fn pick(regions: &[Region], closed: &[usize], line: usize, want_closed: bool) -> Option<usize> {
    let mut best: Option<&Region> = None;
    for region in regions {
        let covers = region.start == line || (region.start < line && line < region.end);
        if !covers {
            continue;
        }
        let is_closed = closed.contains(&region.start);
        if is_closed != want_closed {
            continue;
        }
        let tighter = match best {
            None => true,
            Some(current) => region.end - region.start < current.end - current.start,
        };
        if tighter {
            best = Some(region);
        }
    }
    best.map(|region| region.start)
}

fn fenced(in_fence: &[bool], i: usize) -> bool {
    in_fence.get(i).copied().unwrap_or(false)
}

fn atx_level(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let after = trimmed.trim_start_matches('#');
    let level = (trimmed.len() - after.len()) as u8;
    if !(1..=6).contains(&level) {
        return None;
    }
    let title = after.trim();
    if title.is_empty() {
        None
    } else {
        Some(level)
    }
}

fn indent_cols(line: &str) -> usize {
    let mut cols = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => cols += 1,
            '\t' => cols += 4,
            _ => break,
        }
    }
    cols
}

fn spanned_lines(text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let breaks = text.matches('\n').count() as i32;
    if text.ends_with('\n') {
        breaks
    } else {
        breaks + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    #[test]
    fn heading_fold_stops_at_same_or_higher_level() {
        let src = lines("# A\npara\n## B\nx\n# C");
        let regions = regions(&src, &[]);
        assert!(regions.contains(&Region { start: 0, end: 4 }));
        assert!(regions.contains(&Region { start: 2, end: 4 }));
        assert!(!regions.iter().any(|region| region.start == 4));
    }

    #[test]
    fn indent_fold_hides_deeper_lines_until_blank() {
        let src = lines("- a\n  - b\n    - c\n  - d\n- e");
        let regions = regions(&src, &[]);
        assert!(regions.contains(&Region { start: 0, end: 4 }));
        assert!(regions.contains(&Region { start: 1, end: 3 }));

        let with_blank = lines("- a\n\n- b");
        assert!(super::regions(&with_blank, &[]).is_empty());
    }

    #[test]
    fn closed_parent_hides_descendants() {
        let src = lines("# A\n- a\n  - b\n# C");
        let regions = regions(&src, &[]);
        let hidden = hidden(src.len(), &regions, &[0]);
        assert!(!hidden[0]);
        assert!(hidden[1] && hidden[2]);
        assert!(!hidden[3]);
    }

    #[test]
    fn fold_more_closes_inner_region_first() {
        let src = lines("# A\n- a\n  - b\n# C");
        let regions = regions(&src, &[]);
        let mut closed = Vec::new();
        assert!(fold_more(&regions, &mut closed, 1));
        assert_eq!(closed, vec![1]);
        assert!(fold_more(&regions, &mut closed, 1));
        assert!(closed.contains(&0));
        assert!(fold_less(&regions, &mut closed, 1));
        assert!(!closed.contains(&1));
    }

    #[test]
    fn shift_lines_moves_folds_below_an_insert() {
        let mut closed = vec![0, 3];
        shift_lines(&mut closed, 2, 1);
        assert_eq!(closed, vec![0, 4]);
        shift_lines(&mut closed, 2, -1);
        assert_eq!(closed, vec![0, 3]);
    }

    #[test]
    fn fence_lines_are_not_headings() {
        let src = lines("```\n# not a heading\n```\n# real\nbody");
        let fence = [true, true, true, false, false];
        let regions = regions(&src, &fence);
        assert_eq!(regions, vec![Region { start: 3, end: 5 }]);
    }

    #[test]
    fn callout_header_is_not_a_session_region() {
        // H1/H2, nested list, and a callout — callout fold is text `+/-`, not session.
        let src = lines("# A\npara\n## B\n- a\n  - b\n> [!note] Title\n> body\n# C");
        let regions = regions(&src, &[]);
        assert!(regions.contains(&Region { start: 0, end: 7 }));
        assert!(regions.contains(&Region { start: 2, end: 7 }));
        assert!(regions.contains(&Region { start: 3, end: 5 }));
        assert!(!regions.iter().any(|region| region.start == 5));
        assert!(!regions.iter().any(|region| region.start == 6));
    }
}
