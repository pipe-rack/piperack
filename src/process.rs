//! Data structures for tracking process state.
//!
//! This module defines the specifications for a process (`ProcessSpec`), its current execution status (`ProcessStatus`),
//! and the full state object (`ProcessState`) that holds logs and runtime information.

use std::collections::HashMap;
use std::time::Instant;

use crate::config::ReadinessCheck;
use crate::output::LogBuffer;

/// Specification for a process to be run.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    /// Friendly name for the process.
    pub name: String,
    /// The command executable.
    pub cmd: String,
    /// Arguments for the command.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: Option<String>,
    /// Color to use for the process name in logs.
    pub color: Option<String>,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Whether to restart the process on failure.
    pub restart_on_fail: bool,
    /// Initial follow state for logs.
    pub follow: bool,
    /// Optional command to run before the main process.
    pub pre_cmd: Option<String>,
    /// Paths to watch for changes.
    pub watch_paths: Vec<String>,
    /// Patterns to ignore when watching.
    pub watch_ignore: Vec<String>,
    /// Whether to respect gitignore rules.
    pub watch_ignore_gitignore: bool,
    /// Debounce time for watch events.
    pub watch_debounce_ms: u64,
    /// List of process names this process depends on.
    pub depends_on: Vec<String>,
    /// Configuration for checking if the process is ready.
    pub ready_check: Option<ReadinessCheck>,
    /// Tags for grouping.
    pub tags: Vec<String>,
}

/// The current lifecycle status of a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Process is not running and has no exit status yet (initial state).
    Idle,
    /// Process is starting up.
    Starting,
    /// Process is actively running.
    Running,
    /// Process has exited.
    Exited { code: Option<i32> },
    /// Process failed to start or encountered a runtime error.
    Failed { error: String },
}

/// Runtime state of a single process.
#[derive(Debug, Clone)]
pub struct ProcessState {
    /// The configuration specification for this process.
    pub spec: ProcessSpec,
    /// Current execution status.
    pub status: ProcessStatus,
    /// Process ID (if running).
    pub pid: Option<u32>,
    /// Time when the process started.
    pub started_at: Option<Instant>,
    /// Exit code of the last run.
    pub exit_code: Option<i32>,
    /// Buffer containing the process's output logs.
    pub logs: LogBuffer,
    /// Current scroll position in the log view.
    pub scroll: usize,
    /// Whether the log view is currently following new output.
    pub follow: bool,
    /// Whether user input is currently directed to this process.
    pub input_active: bool,
    /// Whether the process is considered "ready" (passed readiness check).
    pub ready: bool,
}

impl ProcessState {
    /// Creates a new `ProcessState` from a specification.
    pub fn new(spec: ProcessSpec, max_lines: usize) -> Self {
        let follow = spec.follow;
        Self {
            spec,
            status: ProcessStatus::Idle,
            pid: None,
            started_at: None,
            exit_code: None,
            logs: LogBuffer::new(max_lines),
            scroll: 0,
            follow,
            input_active: false,
            ready: false,
        }
    }
}
