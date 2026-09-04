//! A lightweight scrollable, source-invariant event log.
//!
//! The TUI reducer appends `LogEntry`s (already formatted) and the log keeps
//! a viewport offset so the newest entries are always in view by default.

/// A single log line with a severity flag so the renderer can color failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Pre-rendered single-line message.
    pub message: String,
    /// Whether this entry represents a failure (informational otherwise).
    pub is_error: bool,
}

/// Append-only event log with a clamped scroll offset.
#[derive(Debug, Clone, Default)]
pub struct EventLog {
    entries: Vec<LogEntry>,
    offset: usize,
}

impl EventLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a new entry, keeping the current scroll context.
    pub fn push(&mut self, message: impl Into<String>, is_error: bool) {
        self.entries.push(LogEntry {
            message: message.into(),
            is_error,
        });
        self.clamp();
    }

    /// Number of entries in the log.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The raw entries (oldest first).
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// The current scroll offset, where `0` means the newest entry is at the
    /// bottom of the viewport.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Scroll up (toward older entries) by `n` lines.
    pub fn scroll_up(&mut self, n: usize) {
        self.offset = self.offset.saturating_add(n).min(self.entries.len());
        self.clamp();
    }

    /// Scroll down (toward newer entries) by `n` lines.
    pub fn scroll_down(&mut self, n: usize) {
        self.offset = self.offset.saturating_sub(n);
    }

    /// Jump to the newest entries.
    pub fn reset(&mut self) {
        self.offset = 0;
    }

    /// Jump to the oldest entry.
    pub fn to_oldest(&mut self) {
        self.offset = self.entries.len();
    }

    /// The oldest 1-based line number visible at the top of the viewport.
    pub fn top_line(&self) -> usize {
        self.entries.len().saturating_sub(self.offset)
    }

    /// How far back from the newest entry the viewport is (0 == newest in view).
    pub fn delta_from_newest(&self) -> usize {
        self.offset
    }

    fn clamp(&mut self) {
        self.offset = self.offset.min(self.entries.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_log_is_empty_and_offset_zero() {
        let log = EventLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.offset(), 0);
    }

    #[test]
    fn push_keeps_newest_in_view_by_default() {
        let mut log = EventLog::new();
        for i in 0..10 {
            log.push(format!("line {i}"), false);
        }
        assert_eq!(log.len(), 10);
        assert_eq!(log.offset(), 0);
        assert_eq!(log.top_line(), 10);
    }

    #[test]
    fn scroll_offsets_and_reveals_older_lines() {
        let mut log = EventLog::new();
        for i in 0..10 {
            log.push(format!("line {i}"), false);
        }
        log.scroll_up(3);
        assert_eq!(log.offset(), 3);
        assert_eq!(log.top_line(), 7);

        log.scroll_down(1);
        assert_eq!(log.offset(), 2);
        assert_eq!(log.top_line(), 8);
    }

    #[test]
    fn scroll_up_clamps_at_len() {
        let mut log = EventLog::new();
        for i in 0..5 {
            log.push(format!("line {i}"), false);
        }
        log.scroll_up(100);
        assert_eq!(log.offset(), 5);
        assert_eq!(log.top_line(), 0);
    }

    #[test]
    fn scroll_down_floors_at_zero() {
        let mut log = EventLog::new();
        for i in 0..5 {
            log.push(format!("line {i}"), false);
        }
        log.scroll_up(2);
        log.scroll_down(100);
        assert_eq!(log.offset(), 0);
    }

    #[test]
    fn home_and_end_navigation() {
        let mut log = EventLog::new();
        for i in 0..8 {
            log.push(format!("line {i}"), false);
        }
        log.to_oldest();
        assert_eq!(log.offset(), 8);
        log.reset();
        assert_eq!(log.offset(), 0);
    }

    #[test]
    fn error_entries_are_flagged() {
        let mut log = EventLog::new();
        log.push("all good", false);
        log.push("boom", true);
        assert!(!log.entries()[0].is_error);
        assert!(log.entries()[1].is_error);
    }
}
