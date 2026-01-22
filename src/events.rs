//! Event definitions for the application event loop.
//!
//! This module defines the `Event` enum which encapsulates all possible events
//! that drive the application's state transitions, including process updates,
//! user input, and system signals.

use crossterm::event::{KeyEvent, MouseEvent};

use crate::output::StreamKind;

/// Signals used for graceful process shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSignal {
    SigInt,
    SigTerm,
}

impl ProcessSignal {
    pub fn label(self) -> &'static str {
        match self {
            ProcessSignal::SigInt => "SIGINT",
            ProcessSignal::SigTerm => "SIGTERM",
        }
    }
}

/// Represents an event in the application's main event loop.
#[derive(Debug, Clone)]
pub enum Event {
    /// A process is about to start.
    ProcessStarting { id: usize },
    /// A process has started successfully.
    ProcessStarted { id: usize, pid: u32 },
    /// A process has passed its readiness check.
    ProcessReady { id: usize },
    /// A process is waiting on its dependencies to become ready.
    ProcessWaiting { id: usize, deps: Vec<String> },
    /// A line of output (stdout or stderr) was received from a process.
    ProcessOutput {
        id: usize,
        line: String,
        stream: StreamKind,
    },
    /// A process exited with an optional exit code (None usually implies signal termination).
    ProcessExited { id: usize, code: Option<i32> },
    /// A process failed to start or encountered an error.
    ProcessFailed { id: usize, error: String },
    /// A signal was sent to a process.
    ProcessSignal { id: usize, signal: ProcessSignal },
    /// A request to restart a process.
    Restart { id: usize },
    /// The application received a shutdown signal (e.g. SIGINT/SIGTERM).
    Shutdown { signal: ProcessSignal },
    /// Raw bytes received from the application's standard input.
    Stdin(Vec<u8>),
    /// A keyboard event received from the user.
    Key(KeyEvent),
    /// A mouse event received from the user.
    Mouse(MouseEvent),
    /// The terminal window was resized.
    Resize { width: u16, height: u16 },
}

#[cfg(test)]
mod tests {
    use super::ProcessSignal;

    #[test]
    fn process_signal_labels() {
        assert_eq!(ProcessSignal::SigInt.label(), "SIGINT");
        assert_eq!(ProcessSignal::SigTerm.label(), "SIGTERM");
    }
}
