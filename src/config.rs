//! Configuration management for Piperack.
//!
//! This module defines the structure of the `piperack.toml` configuration file
//! and provides functionality to load and parse it.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level configuration structure corresponding to `piperack.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Maximum number of lines to keep in memory per process.
    pub max_lines: Option<usize>,
    /// Whether to use Unicode symbols in the TUI (default: true).
    pub symbols: Option<bool>,
    /// Whether to use raw output mode (no TUI) (default: false).
    pub raw: Option<bool>,
    /// Template for line prefixes (e.g., "[{name}]").
    pub prefix: Option<String>,
    /// Fixed length for prefixes (padding/truncation).
    pub prefix_length: Option<usize>,
    /// Whether to colorize prefixes in non-TUI output.
    pub prefix_colors: Option<bool>,
    /// Whether to prepend timestamps to log lines.
    pub timestamp: Option<bool>,
    /// Output mode for non-TUI usage ("combined", "grouped", "raw").
    pub output: Option<String>,
    /// Success policy ("first", "last", "all").
    pub success: Option<String>,
    /// Whether to kill all other processes if one exits.
    pub kill_others: Option<bool>,
    /// Whether to kill all other processes if one fails.
    pub kill_others_on_fail: Option<bool>,
    /// Maximum number of restart attempts.
    pub restart_tries: Option<u32>,
    /// Delay in milliseconds before restarting a process.
    pub restart_delay_ms: Option<u64>,
    /// Whether to handle stdin input (default: true).
    pub handle_input: Option<bool>,
    /// Template for log file paths.
    pub log_file: Option<String>,
    /// List of processes to run.
    #[serde(rename = "process")]
    pub processes: Vec<ProcessConfig>,
}

/// Configuration for a single process.
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessConfig {
    /// Display name of the process.
    pub name: String,
    /// Command to execute.
    pub cmd: String,
    /// Working directory for the process.
    pub cwd: Option<String>,
    /// Color override for the process name in logs.
    pub color: Option<String>,
    /// Environment variables to set for the process.
    pub env: Option<HashMap<String, String>>,
    /// Whether to restart the process if it fails.
    pub restart_on_fail: Option<bool>,
    /// Whether to automatically follow the logs of this process (default: true).
    pub follow: Option<bool>,
    /// Command to run before starting the main process.
    pub pre_cmd: Option<String>,
    /// List of file paths or patterns to watch for changes.
    pub watch: Option<Vec<String>>,
    /// List of patterns to ignore when watching.
    pub watch_ignore: Option<Vec<String>>,
    /// Whether to respect .gitignore when watching (default: false).
    pub watch_ignore_gitignore: Option<bool>,
    /// Debounce interval in milliseconds for watch events.
    pub watch_debounce_ms: Option<u64>,
    /// List of process names this process depends on.
    pub depends_on: Option<Vec<String>>,
    /// Readiness check configuration.
    pub ready_check: Option<ReadinessCheck>,
    /// Tags for grouping processes.
    pub tags: Option<Vec<String>>,
}

/// Configuration for process readiness checks.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCheck {
    /// Wait for a TCP port to accept connections.
    Tcp(u16),
    /// Wait for a specific duration (milliseconds).
    Delay(u64),
    /// Wait for a log line matching a regex.
    Log(String),
}

/// Loads and parses the configuration from a file path.
pub fn load_config(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let config: Config = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_optional_fields() {
        let raw = r#"
max_lines = 200
symbols = false
raw = true
prefix = "[{name}]"
prefix_length = 12
prefix_colors = true
timestamp = true
output = "combined"
success = "all"
kill_others = true
kill_others_on_fail = false
restart_tries = 3
restart_delay_ms = 250
handle_input = true
log_file = "logs/{name}.log"

[[process]]
name = "api"
cmd = "cargo run"
pre_cmd = "pnpm i"
restart_on_fail = true
follow = false
watch = ["src", "Cargo.toml"]
watch_ignore = ["target", "**/*.log"]
watch_ignore_gitignore = true
watch_debounce_ms = 150

[[process]]
name = "web"
cmd = "pnpm dev"
"#;
        let config: Config = toml::from_str(raw).unwrap();
        assert_eq!(config.max_lines, Some(200));
        assert_eq!(config.symbols, Some(false));
        assert_eq!(config.raw, Some(true));
        assert_eq!(config.prefix.as_deref(), Some("[{name}]"));
        assert_eq!(config.prefix_length, Some(12));
        assert_eq!(config.prefix_colors, Some(true));
        assert_eq!(config.timestamp, Some(true));
        assert_eq!(config.output.as_deref(), Some("combined"));
        assert_eq!(config.success.as_deref(), Some("all"));
        assert_eq!(config.kill_others, Some(true));
        assert_eq!(config.kill_others_on_fail, Some(false));
        assert_eq!(config.restart_tries, Some(3));
        assert_eq!(config.restart_delay_ms, Some(250));
        assert_eq!(config.handle_input, Some(true));
        assert_eq!(config.log_file.as_deref(), Some("logs/{name}.log"));
        assert_eq!(config.processes.len(), 2);
        assert_eq!(config.processes[0].restart_on_fail, Some(true));
        assert_eq!(config.processes[0].follow, Some(false));
    }
}
