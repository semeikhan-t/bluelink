//! Per-note typing history: burst grouping for line edits, explicit checkpoints.

const MAX_UNDO: usize = 50;

#[derive(Default)]
pub struct NoteHistory {
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    /// Snapshot at burst start; `None` when not in a burst.
    burst_start: Option<String>,
}

impl NoteHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot when entering a line. Repeat calls in the same burst are no-ops.
    pub fn begin_burst(&mut self, buffer: &str) {
        if self.burst_start.is_some() {
            return;
        }
        self.burst_start = Some(buffer.to_string());
    }

    /// If the buffer changed since `begin_burst`, push that snapshot (cap 50) and close the burst.
    pub fn end_burst(&mut self, buffer: &str) {
        let Some(start) = self.burst_start.take() else {
            return;
        };
        if start != buffer {
            self.push_undo(start);
            self.redo_stack.clear();
        }
    }

    /// Separate undo step before a structural edit. Flushes an open burst first if needed.
    pub fn checkpoint(&mut self, before: &str) {
        if let Some(start) = self.burst_start.take() {
            if start != before {
                self.push_undo(start);
            }
        }
        self.push_undo(before.to_string());
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, buffer: &mut String) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.burst_start = None;
        self.redo_stack.push(std::mem::replace(buffer, prev));
        true
    }

    pub fn redo(&mut self, buffer: &mut String) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.burst_start = None;
        self.undo_stack.push(std::mem::replace(buffer, next));
        true
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.burst_start = None;
    }

    fn push_undo(&mut self, snapshot: String) {
        self.undo_stack.push(snapshot);
        while self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_steps_undo_redo() {
        let mut h = NoteHistory::new();
        h.checkpoint("a");
        h.checkpoint("ab");
        let mut buf = String::from("abc");

        assert!(h.undo(&mut buf));
        assert_eq!(buf, "ab");
        assert!(h.undo(&mut buf));
        assert_eq!(buf, "a");
        assert!(!h.undo(&mut buf));

        assert!(h.redo(&mut buf));
        assert_eq!(buf, "ab");
        assert!(h.redo(&mut buf));
        assert_eq!(buf, "abc");
        assert!(!h.redo(&mut buf));
    }

    #[test]
    fn burst_groups_typing() {
        let mut h = NoteHistory::new();
        let mut buf = String::from("hi");
        h.begin_burst("hi");
        h.begin_burst("hix"); // same burst — ignored
        buf.push_str(" there");
        h.end_burst(&buf);
        assert!(h.undo(&mut buf));
        assert_eq!(buf, "hi");
    }

    #[test]
    fn clear_empties_stacks() {
        let mut h = NoteHistory::new();
        h.checkpoint("x");
        let mut buf = String::from("y");
        h.clear();
        assert!(!h.undo(&mut buf));
        assert_eq!(buf, "y");
    }
}
