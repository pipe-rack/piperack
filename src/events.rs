//! Event definitions for the application event loop.
//!
//! This module defines the `Event` enum which encapsulates all possible events
//! that drive the application's state transitions, including process updates,
//! user input, and system signals.

use crossterm::event::{KeyEvent, MouseEvent};

use crate::output::StreamKind;

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
    ProcessOutput { id: usize, line: String, stream: StreamKind },
    /// A process exited with an optional exit code (None usually implies signal termination).
    ProcessExited { id: usize, code: Option<i32> },
    /// A process failed to start or encountered an error.
    ProcessFailed { id: usize, error: String },
    /// A request to restart a process.
    Restart { id: usize },
    /// Raw bytes received from the application's standard input.
    Stdin(Vec<u8>),
    /// A keyboard event received from the user.
    Key(KeyEvent),
    /// A mouse event received from the user.
    Mouse(MouseEvent),
    /// The terminal window was resized.
    Resize { width: u16, height: u16 },
}
