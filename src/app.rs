//! Application state and UI logic.
//!
//! This module holds the core `App` struct, which maintains the state of all processes,
//! the global timeline, search state, and user input buffers. It also defines how
//! user input events are translated into application actions.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::output::{sanitize_text, LogLine, StreamKind, TimelineBuffer, TimelineEntry};
use crate::process::{ProcessState, ProcessStatus, ProcessSpec};

/// Modes of user input interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Standard navigation mode.
    Normal,
    /// Typing a search query.
    Search,
    /// Typing a filter query.
    Filter,
    /// Direct input to a process's stdin.
    Input,
    /// Typing a group name to restart.
    Group,
}

/// The main application state container.
#[derive(Debug)]
pub struct App {
    /// List of all managed processes.
    pub processes: Vec<ProcessState>,
    /// Index of the currently selected process.
    pub selected: usize,
    /// Current input mode.
    pub input_mode: InputMode,
    /// Buffer for search input.
    pub input: String,
    /// Buffer for process stdin input.
    pub input_buffer: String,
    /// Whether input handling is enabled globally.
    pub input_enabled: bool,
    /// Active search query, if any.
    pub search_query: Option<String>,
    /// Indices of log lines matching the current search.
    pub search_matches: Vec<usize>,
    /// Current position within the search matches.
    pub search_index: usize,
    /// Active filter query.
    pub filter_query: Option<String>,
    /// Whether JSON formatting is enabled.
    pub json_formatting: bool,
    /// Flag indicating if the application should exit.
    pub should_quit: bool,
    /// Height of the log view area (for scrolling calculations).
    pub log_view_height: usize,
    /// Width of the process list area (for mouse clicks).
    pub process_list_width: u16,
    /// Whether the timeline view is active.
    pub timeline_view: bool,
    /// Whether the timeline is automatically following new output.
    pub timeline_follow: bool,
    /// Scroll position of the timeline view.
    pub timeline_scroll: usize,
    /// Global buffer of all process output in time order.
    pub timeline: TimelineBuffer,
    /// Whether to strip ANSI codes from the display.
    pub strip_ansi: bool,
    /// Whether to use Unicode symbols.
    pub use_symbols: bool,
    /// Whether to show the help modal/overlay.
    pub show_help: bool,
    status_message: Option<StatusMessage>,
}

/// Actions resulting from user interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    /// No action required.
    None,
    /// Exit the application.
    Quit,
    /// Kill a process.
    Kill(usize),
    /// Restart a process.
    Restart(usize),
    /// Restart all processes in a group/tag.
    RestartGroup(String),
    /// Export logs to a file.
    Export(usize),
    /// Send text to a process's stdin.
    SendInputText(usize, String),
    /// Send raw bytes to a process's stdin.
    SendInputBytes(usize, Vec<u8>),
}

#[derive(Debug, Clone)]
struct StatusMessage {
    text: String,
    at: Instant,
}

impl App {
    /// Creates a new `App` instance.
    pub fn new(
        specs: Vec<ProcessSpec>,
        max_lines: usize,
        use_symbols: bool,
        input_enabled: bool,
    ) -> Self {
        let process_count = specs.len().max(1);
        let timeline_max = max_lines
            .saturating_mul(process_count)
            .min(50_000)
            .max(max_lines);
        let processes = specs
            .into_iter()
            .map(|spec| ProcessState::new(spec, max_lines))
            .collect();
        Self {
            processes,
            selected: 0,
            input_mode: InputMode::Normal,
            input: String::new(),
            input_buffer: String::new(),
            input_enabled,
            search_query: None,
            search_matches: Vec::new(),
            search_index: 0,
            filter_query: None,
            json_formatting: false,
            should_quit: false,
            log_view_height: 0,
            process_list_width: 0,
            timeline_view: false,
            timeline_follow: true,
            timeline_scroll: 0,
            timeline: TimelineBuffer::new(timeline_max),
            strip_ansi: false,
            use_symbols,
            show_help: false,
            status_message: None,
        }
    }

    pub fn selected_process(&self) -> Option<&ProcessState> {
        self.processes.get(self.selected)
    }

    pub fn selected_process_mut(&mut self) -> Option<&mut ProcessState> {
        self.processes.get_mut(self.selected)
    }

    pub fn on_process_starting(&mut self, id: usize) {
        if let Some(process) = self.processes.get_mut(id) {
            process.status = ProcessStatus::Starting;
            process.pid = None;
            process.exit_code = None;
        }
    }

    pub fn on_process_ready(&mut self, id: usize) {
        if let Some(process) = self.processes.get_mut(id) {
            process.ready = true;
        }
    }

    pub fn on_process_started(&mut self, id: usize, pid: u32) {
        if let Some(process) = self.processes.get_mut(id) {
            process.status = ProcessStatus::Running;
            process.pid = Some(pid);
            process.started_at = Some(Instant::now());
            process.exit_code = None;
        }
    }

    pub fn on_process_output(&mut self, id: usize, line: String, stream: StreamKind) {
        let selected = self.selected == id;
        let selected_follow = selected
            .then(|| self.processes.get(id).map(|p| p.follow).unwrap_or(true))
            .unwrap_or(false);
        if let Some(process) = self.processes.get_mut(id) {
            let dropped = process.logs.push(LogLine { text: line.clone(), stream });
            if dropped && !process.follow && process.scroll > 0 {
                process.scroll -= 1;
            }
        }

        let dropped_timeline = self.timeline.push(TimelineEntry {
            text: line,
            process_id: id,
        });
        if dropped_timeline && !self.timeline_follow && self.timeline_scroll > 0 {
            self.timeline_scroll -= 1;
        }

        if self.timeline_view {
            if self.timeline_follow {
                self.ensure_follow();
            }
        } else if selected && selected_follow {
            self.ensure_follow();
        }

        if self.timeline_view || selected {
            self.update_search_matches();
        }
    }

    pub fn on_process_exited(&mut self, id: usize, code: Option<i32>) {
        if let Some(process) = self.processes.get_mut(id) {
            process.status = ProcessStatus::Exited { code };
            process.exit_code = code;
        }
    }

    pub fn on_process_failed(&mut self, id: usize, error: String) {
        if let Some(process) = self.processes.get_mut(id) {
            process.status = ProcessStatus::Failed { error };
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> AppAction {
        match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                if mouse.column < self.process_list_width {
                    let row = mouse.row.saturating_sub(1); // header offset
                    if let Some(index) = self.process_index_at_visual_row(row) {
                        self.selected = index;
                        self.update_search_matches();
                    }
                }
            }
            MouseEventKind::ScrollDown => self.scroll_down(3),
            MouseEventKind::ScrollUp => self.scroll_up(3),
            _ => {}
        }
        AppAction::None
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        match self.input_mode {
            InputMode::Search => self.handle_search_input(key),
            InputMode::Filter => self.handle_filter_input(key),
            InputMode::Group => self.handle_group_input(key),
            InputMode::Input => self.handle_input_key(key),
            InputMode::Normal => self.handle_normal_input(key),
        }
    }

    fn handle_group_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input.clear();
                AppAction::None
            }
            KeyCode::Enter => {
                let query = self.input.trim().to_string();
                self.input.clear();
                self.input_mode = InputMode::Normal;
                if !query.is_empty() {
                    AppAction::RestartGroup(query)
                } else {
                    AppAction::None
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                AppAction::None
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return AppAction::None;
                }
                self.input.push(c);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_filter_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input.clear();
                AppAction::None
            }
            KeyCode::Enter => {
                // Confirm the filter
                self.input.clear();
                self.input_mode = InputMode::Normal;
                AppAction::None
            }
            KeyCode::Backspace => {
                if self.input.pop().is_some() {
                    let query = self.input.trim().to_string();
                    self.filter_query = if query.is_empty() { None } else { Some(query) };
                }
                AppAction::None
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return AppAction::None;
                }
                self.input.push(c);
                let query = self.input.trim().to_string();
                self.filter_query = if query.is_empty() { None } else { Some(query) };
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input.clear();
                // We keep the query as is (live updated)
                AppAction::None
            }
            KeyCode::Enter => {
                // Confirm the query
                self.input.clear();
                self.input_mode = InputMode::Normal;
                AppAction::None
            }
            KeyCode::Backspace => {
                if self.input.pop().is_some() {
                    let query = self.input.trim().to_string();
                    self.search_query = if query.is_empty() { None } else { Some(query) };
                    self.update_search_matches();
                }
                AppAction::None
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return AppAction::None;
                }
                self.input.push(c);
                let query = self.input.trim().to_string();
                self.search_query = if query.is_empty() { None } else { Some(query) };
                self.update_search_matches();
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.exit_input_mode();
                AppAction::None
            }
            KeyCode::Enter => {
                let payload = std::mem::take(&mut self.input_buffer);
                AppAction::SendInputText(self.selected, payload)
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
                AppAction::None
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let Some(ctrl) = control_byte(c) {
                        return AppAction::SendInputBytes(self.selected, vec![ctrl]);
                    }
                }
                self.input_buffer.push(c);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn handle_normal_input(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                AppAction::Quit
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.exit_input_mode();
                    self.selected -= 1;
                    self.update_search_matches();
                    if self.selected_following() {
                        self.ensure_follow();
                    }
                }
                AppAction::None
            }
            KeyCode::Down => {
                if self.selected + 1 < self.processes.len() {
                    self.exit_input_mode();
                    self.selected += 1;
                    self.update_search_matches();
                    if self.selected_following() {
                        self.ensure_follow();
                    }
                }
                AppAction::None
            }
            KeyCode::Tab => {
                if !self.processes.is_empty() {
                    self.exit_input_mode();
                    self.selected = (self.selected + 1) % self.processes.len();
                    self.update_search_matches();
                    if self.selected_following() {
                        self.ensure_follow();
                    }
                }
                AppAction::None
            }
            KeyCode::Char('f') => {
                self.toggle_follow();
                AppAction::None
            }
            KeyCode::Char('F') => {
                self.input_mode = InputMode::Filter;
                self.input = self.filter_query.clone().unwrap_or_default();
                AppAction::None
            }
            KeyCode::Char('j') => {
                self.json_formatting = !self.json_formatting;
                AppAction::None
            }
            KeyCode::Enter => {
                if self.input_enabled {
                    self.enter_input_mode();
                }
                AppAction::None
            }
            KeyCode::Char('t') => {
                self.exit_input_mode();
                self.timeline_view = !self.timeline_view;
                self.update_search_matches();
                if self.is_following() {
                    self.ensure_follow();
                }
                AppAction::None
            }
            KeyCode::Char('a') => {
                self.strip_ansi = !self.strip_ansi;
                AppAction::None
            }
            KeyCode::Char('e') => AppAction::Export(self.selected),
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.input = self.search_query.clone().unwrap_or_default();
                AppAction::None
            }
            KeyCode::Char('n') => {
                self.jump_search(true);
                AppAction::None
            }
            KeyCode::Char('N') => {
                self.jump_search(false);
                AppAction::None
            }
            KeyCode::Char('r') => AppAction::Restart(self.selected),
            KeyCode::Char('R') => AppAction::RestartGroup("all".to_string()),
            KeyCode::Char('g') => {
                self.input_mode = InputMode::Group;
                self.input.clear();
                // Pre-fill with current process tag if available?
                // The user request implies they want to *provide* it, so maybe blank is better or
                // maybe prompt with current one.
                // Let's stick to blank for now as per "you can use the input to provide it".
                if let Some(proc) = self.selected_process() {
                    if let Some(tag) = proc.spec.tags.first() {
                        self.input = tag.clone();
                    }
                }
                AppAction::None
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                AppAction::None
            }
            KeyCode::Char('k') => AppAction::Kill(self.selected),
            KeyCode::PageUp => {
                self.scroll_up(self.log_view_height.max(1));
                AppAction::None
            }
            KeyCode::PageDown => {
                self.scroll_down(self.log_view_height.max(1));
                AppAction::None
            }
            KeyCode::Home => {
                self.scroll_to_top();
                AppAction::None
            }
            KeyCode::End => {
                self.ensure_follow();
                if self.timeline_view {
                    self.timeline_follow = true;
                } else if let Some(process) = self.selected_process_mut() {
                    process.follow = true;
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        let view = self.log_view_height.max(1);
        if self.timeline_view {
            let max_scroll = self.timeline.len().saturating_sub(view);
            let current = if self.timeline_follow {
                max_scroll
            } else {
                self.timeline_scroll
            };
            let next = current.saturating_sub(amount).min(max_scroll);
            self.timeline_scroll = next;
            self.timeline_follow = false;
            return;
        }

        if let Some(process) = self.selected_process_mut() {
            let len = process.logs.len();
            let max_scroll = len.saturating_sub(view);
            let current = if process.follow { max_scroll } else { process.scroll };
            let next = current.saturating_sub(amount).min(max_scroll);
            process.scroll = next;
            process.follow = false;
        }
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let view = self.log_view_height.max(1);
        if self.timeline_view {
            let max_scroll = self.timeline.len().saturating_sub(view);
            let current = if self.timeline_follow {
                max_scroll
            } else {
                self.timeline_scroll
            };
            let next = (current + amount).min(max_scroll);
            self.timeline_scroll = next;
            self.timeline_follow = next == max_scroll;
            return;
        }

        if let Some(process) = self.selected_process_mut() {
            let len = process.logs.len();
            let max_scroll = len.saturating_sub(view);
            let current = if process.follow { max_scroll } else { process.scroll };
            let next = (current + amount).min(max_scroll);
            process.scroll = next;
            process.follow = next == max_scroll;
        }
    }

    pub fn scroll_to_top(&mut self) {
        if self.timeline_view {
            self.timeline_scroll = 0;
            self.timeline_follow = false;
            return;
        }
        if let Some(process) = self.selected_process_mut() {
            process.scroll = 0;
            process.follow = false;
        }
    }

    pub fn ensure_follow(&mut self) {
        let view = self.log_view_height.max(1);
        if self.timeline_view {
            let max_scroll = self.timeline.len().saturating_sub(view);
            self.timeline_scroll = max_scroll;
            return;
        }
        if let Some(process) = self.selected_process_mut() {
            let len = process.logs.len();
            let max_scroll = len.saturating_sub(view);
            process.scroll = max_scroll;
        }
    }

    pub fn set_log_view_height(&mut self, height: usize) {
        self.log_view_height = height;
        let view = height.max(1);
        if self.timeline_view {
            let max_scroll = self.timeline.len().saturating_sub(view);
            if self.timeline_follow {
                self.timeline_scroll = max_scroll;
            } else {
                self.timeline_scroll = self.timeline_scroll.min(max_scroll);
            }
            return;
        }

        if let Some(process) = self.selected_process_mut() {
            let max_scroll = process.logs.len().saturating_sub(view);
            if process.follow {
                process.scroll = max_scroll;
            } else {
                process.scroll = process.scroll.min(max_scroll);
            }
        }
    }

    fn update_search_matches(&mut self) {
        self.search_index = 0;
        let Some(query) = self.search_query.clone() else {
            self.search_matches.clear();
            return;
        };
        let mut matches = Vec::new();
        if self.timeline_view {
            for (idx, entry) in self.timeline.iter().enumerate() {
                if entry.text.contains(&query) {
                    matches.push(idx);
                }
            }
        } else if let Some(process) = self.selected_process() {
            for (idx, line) in process.logs.iter().enumerate() {
                if line.text.contains(&query) {
                    matches.push(idx);
                }
            }
        }
        self.search_matches = matches;
    }

    fn jump_search(&mut self, forward: bool) {
        if self.search_matches.is_empty() {
            return;
        }
        if forward {
            self.search_index = (self.search_index + 1) % self.search_matches.len();
        } else if self.search_index == 0 {
            self.search_index = self.search_matches.len() - 1;
        } else {
            self.search_index -= 1;
        }
        let view = self.log_view_height.max(1);
        let target_line = self.selected_match_line();
        if let Some(line) = target_line {
            if self.timeline_view {
                let max_scroll = self.timeline.len().saturating_sub(view);
                let target = line.saturating_sub(view / 2).min(max_scroll);
                self.timeline_scroll = target;
                self.timeline_follow = false;
            } else if let Some(process) = self.selected_process_mut() {
                let max_scroll = process.logs.len().saturating_sub(view);
                let target = line.saturating_sub(view / 2).min(max_scroll);
                process.scroll = target;
                process.follow = false;
            }
        }
    }

    pub fn selected_match_line(&self) -> Option<usize> {
        if self.search_matches.is_empty() {
            None
        } else {
            self.search_matches.get(self.search_index).copied()
        }
    }

    pub fn status_line(&self) -> String {
        if self.timeline_view {
            return format!(
                "Timeline | lines: {} | follow: {} | ansi: {}",
                self.timeline.len(),
                if self.timeline_follow { "on" } else { "off" },
                if self.strip_ansi { "off" } else { "on" }
            );
        }
        let Some(process) = self.selected_process() else {
            return "No processes".to_string();
        };
        let status = match &process.status {
            ProcessStatus::Idle => "idle".to_string(),
            ProcessStatus::Starting => "starting".to_string(),
            ProcessStatus::Running => "running".to_string(),
            ProcessStatus::Exited { code } => {
                let code = code.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
                format!("exited ({})", code)
            }
            ProcessStatus::Failed { error } => format!("failed ({})", error),
        };
        let pid = process.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        let lines = process.logs.len();
        let elapsed = process
            .started_at
            .map(|t| format_duration(t.elapsed()))
            .unwrap_or_else(|| "-".into());
        format!(
            "{} | status: {} | pid: {} | lines: {} | elapsed: {} | follow: {} | ansi: {} | input: {}",
            process.spec.name,
            status,
            pid,
            lines,
            elapsed,
            if process.follow { "on" } else { "off" },
            if self.strip_ansi { "off" } else { "on" },
            if process.input_active { "on" } else { "off" }
        )
    }

    pub fn status_message(&self) -> Option<&str> {
        if let Some(message) = &self.status_message {
            if message.at.elapsed() < Duration::from_secs(3) {
                return Some(message.text.as_str());
            }
        }
        None
    }

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(StatusMessage {
            text: message.into(),
            at: Instant::now(),
        });
    }

    pub fn input_line(&self) -> &str {
        &self.input_buffer
    }

    pub fn export_selected_logs(&mut self) -> Result<PathBuf> {
        let Some(process) = self.selected_process() else {
            anyhow::bail!("no process selected");
        };
        let dir = PathBuf::from("piperack-logs");
        fs::create_dir_all(&dir).context("failed to create piperack-logs directory")?;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let name = sanitize_name(&process.spec.name);
        let path = dir.join(format!("{}-{}.log", name, epoch));
        let mut output = String::new();
        for line in process.logs.iter() {
            if line.stream == StreamKind::Stderr {
                output.push_str("[stderr] ");
            }
            let text = sanitize_text(&line.text, self.strip_ansi);
            output.push_str(&text);
            output.push('\n');
        }
        fs::write(&path, output).with_context(|| format!("failed to write {}", path.display()))?;
        self.set_status_message(format!("Exported logs to {}", path.display()));
        Ok(path)
    }

    fn selected_following(&self) -> bool {
        self.selected_process().map(|p| p.follow).unwrap_or(true)
    }

    fn toggle_follow(&mut self) {
        if self.timeline_view {
            self.timeline_follow = !self.timeline_follow;
            if self.timeline_follow {
                self.ensure_follow();
            }
            return;
        }
        if let Some(process) = self.selected_process_mut() {
            process.follow = !process.follow;
            if process.follow {
                self.ensure_follow();
            }
        }
    }

    fn is_following(&self) -> bool {
        if self.timeline_view {
            self.timeline_follow
        } else {
            self.selected_process().map(|p| p.follow).unwrap_or(true)
        }
    }

    fn enter_input_mode(&mut self) {
        self.input_mode = InputMode::Input;
        self.input_buffer.clear();
        if let Some(process) = self.selected_process_mut() {
            process.input_active = true;
        }
    }

    fn exit_input_mode(&mut self) {
        if self.input_mode == InputMode::Input {
            self.input_mode = InputMode::Normal;
            self.input_buffer.clear();
        }
        for process in &mut self.processes {
            process.input_active = false;
        }
    }

    /// Maps a visual row index (accounting for group headers) to a process index.
    pub fn process_index_at_visual_row(&self, row: u16) -> Option<usize> {
        let mut current_ui_index = 0;
        let mut last_tag: Option<&str> = None;

        for (i, process) in self.processes.iter().enumerate() {
            let tag = process.spec.tags.first().map(|s| s.as_str()).unwrap_or("Ungrouped");
            
            if last_tag != Some(tag) {
                // This is a header row
                if current_ui_index == row {
                    return None; // Clicked on header
                }
                current_ui_index += 1;
                last_tag = Some(tag);
            }

            if current_ui_index == row {
                return Some(i);
            }
            current_ui_index += 1;
        }
        None
    }
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let minutes = secs / 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn control_byte(c: char) -> Option<u8> {
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let upper = c.to_ascii_uppercase() as u8;
    Some(upper.saturating_sub(b'@'))
}
