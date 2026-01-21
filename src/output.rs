//! Output handling and buffering for process logs.
//!
//! This module provides structures to store and manage log lines for individual processes
//! (`LogBuffer`) and a global timeline (`TimelineBuffer`). It also handles text sanitization
//! for display.

use std::collections::VecDeque;

use strip_ansi_escapes::strip;

/// Indicates the source stream of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// Standard Output.
    Stdout,
    /// Standard Error.
    Stderr,
}

/// A single line of log output from a process.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// The content of the log line.
    pub text: String,
    /// The stream it originated from (stdout/stderr).
    pub stream: StreamKind,
}

/// An entry in the global timeline view.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    /// The content of the log line.
    pub text: String,
    /// The ID of the process that generated this line.
    pub process_id: usize,
}

/// A fixed-capacity ring buffer for storing `LogLine`s.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    max_lines: usize,
    lines: VecDeque<LogLine>,
}

impl LogBuffer {
    /// Creates a new `LogBuffer` with the specified maximum capacity.
    pub fn new(max_lines: usize) -> Self {
        Self {
            max_lines,
            lines: VecDeque::with_capacity(max_lines.min(1024)),
        }
    }

    /// Adds a line to the buffer.
    ///
    /// Returns `true` if an old line was dropped to make room.
    pub fn push(&mut self, line: LogLine) -> bool {
        let mut dropped = false;
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
            dropped = true;
        }
        dropped
    }

    /// Returns the number of lines currently in the buffer.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns an iterator over the lines in the buffer.
    pub fn iter(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }
}

/// A fixed-capacity ring buffer for storing `TimelineEntry`s.
#[derive(Debug, Clone)]
pub struct TimelineBuffer {
    max_lines: usize,
    entries: VecDeque<TimelineEntry>,
}

impl TimelineBuffer {
    /// Creates a new `TimelineBuffer` with the specified maximum capacity.
    pub fn new(max_lines: usize) -> Self {
        Self {
            max_lines,
            entries: VecDeque::with_capacity(max_lines.min(1024)),
        }
    }

    /// Adds an entry to the buffer.
    ///
    /// Returns `true` if an old entry was dropped to make room.
    pub fn push(&mut self, entry: TimelineEntry) -> bool {
        let mut dropped = false;
        self.entries.push_back(entry);
        while self.entries.len() > self.max_lines {
            self.entries.pop_front();
            dropped = true;
        }
        dropped
    }

    /// Returns the number of entries currently in the buffer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns an iterator over the entries in the buffer.
    pub fn iter(&self) -> impl Iterator<Item = &TimelineEntry> {
        self.entries.iter()
    }
}

/// Sanitizes text for display, optionally stripping ANSI escape codes.
///
/// If `strip_ansi` is true, ANSI codes are removed. Invalid UTF-8 sequences are replaced.
pub fn sanitize_text(text: &str, strip_ansi: bool) -> String {
    if !strip_ansi {
        return text.to_string();
    }
    let stripped = strip(text.as_bytes());
    String::from_utf8_lossy(&stripped).to_string()
}

pub fn format_json(text: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        if let Ok(pretty) = serde_json::to_string_pretty(&val) {
            return pretty;
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_drops_oldest() {
        let mut buffer = LogBuffer::new(2);
        buffer.push(LogLine {
            text: "a".into(),
            stream: StreamKind::Stdout,
        });
        buffer.push(LogLine {
            text: "b".into(),
            stream: StreamKind::Stdout,
        });
        let dropped = buffer.push(LogLine {
            text: "c".into(),
            stream: StreamKind::Stdout,
        });
        assert!(dropped);
        let lines = buffer.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
        assert_eq!(lines, vec!["b", "c"]);
    }

    #[test]
    fn timeline_buffer_drops_oldest() {
        let mut buffer = TimelineBuffer::new(1);
        buffer.push(TimelineEntry {
            text: "x".into(),
            process_id: 0,
        });
        let dropped = buffer.push(TimelineEntry {
            text: "y".into(),
            process_id: 1,
        });
        assert!(dropped);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.iter().next().unwrap().text, "y");
    }
}
