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
use crate::process::{ProcessSpec, ProcessState, ProcessStatus};

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
    log_viewport: Option<LogViewport>,
    visible_raw_lines: Vec<String>,
    selection_start: Option<usize>,
    selection_end: Option<usize>,
    selection_active: bool,
    selection_scope: Option<SelectionScope>,
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
    /// Copy selected logs (or full buffer) to clipboard.
    CopySelection,
}

#[derive(Debug, Clone, Copy)]
pub enum StatusLevel {
    Info,
    Warning,
}

#[derive(Debug, Clone, Copy)]
pub struct LogViewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy)]
enum SelectionScope {
    Timeline,
    Process(usize),
}

#[derive(Debug, Clone)]
struct StatusMessage {
    text: String,
    at: Instant,
    ttl: Option<Duration>,
    level: StatusLevel,
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
            log_viewport: None,
            visible_raw_lines: Vec::new(),
            selection_start: None,
            selection_end: None,
            selection_active: false,
            selection_scope: None,
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
            let dropped = process.logs.push(LogLine {
                text: line.clone(),
                stream,
            });
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
                } else if let Some(row) = self.log_row_at(mouse.row, mouse.column) {
                    self.freeze_follow_for_selection();
                    self.selection_start = Some(row);
                    self.selection_end = Some(row);
                    self.selection_active = true;
                    self.selection_scope = Some(self.current_selection_scope());
                }
            }
            MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                if self.selection_active {
                    if let Some(row) = self.log_row_at(mouse.row, mouse.column) {
                        self.selection_end = Some(row);
                    }
                }
            }
            MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                self.selection_active = false;
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
                AppAction::CopySelection
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.exit_input_mode();
                    self.clear_selection();
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
                    self.clear_selection();
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
                    self.clear_selection();
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
                self.clear_selection();
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
        self.clear_selection();
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
            let current = if process.follow {
                max_scroll
            } else {
                process.scroll
            };
            let next = current.saturating_sub(amount).min(max_scroll);
            process.scroll = next;
            process.follow = false;
        }
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let view = self.log_view_height.max(1);
        self.clear_selection();
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
            let current = if process.follow {
                max_scroll
            } else {
                process.scroll
            };
            let next = (current + amount).min(max_scroll);
            process.scroll = next;
            process.follow = next == max_scroll;
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.clear_selection();
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
        self.clear_selection();
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
        let pid = process
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into());
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

    pub fn status_message(&self) -> Option<(&str, StatusLevel)> {
        if let Some(message) = &self.status_message {
            let still_visible = match message.ttl {
                Some(ttl) => message.at.elapsed() < ttl,
                None => true,
            };
            if still_visible {
                return Some((message.text.as_str(), message.level));
            }
        }
        None
    }

    pub fn set_log_viewport(&mut self, viewport: LogViewport) {
        self.log_viewport = Some(viewport);
    }

    pub fn set_visible_raw_lines(&mut self, lines: Vec<String>) {
        self.visible_raw_lines = lines;
    }

    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
        self.selection_active = false;
        self.selection_scope = None;
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let scope = self.selection_scope?;
        if !self.selection_scope_matches(scope) {
            return None;
        }
        let start = self.selection_start?;
        let end = self.selection_end?;
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        if self.visible_raw_lines.is_empty() {
            return None;
        }
        let max_idx = self.visible_raw_lines.len().saturating_sub(1);
        Some((start.min(max_idx), end.min(max_idx)))
    }

    pub fn selection_range_for(&self, len: usize) -> Option<(usize, usize)> {
        let scope = self.selection_scope?;
        if !self.selection_scope_matches(scope) || len == 0 {
            return None;
        }
        let start = self.selection_start?;
        let end = self.selection_end?;
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let max_idx = len.saturating_sub(1);
        Some((start.min(max_idx), end.min(max_idx)))
    }

    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        if start > end || self.visible_raw_lines.is_empty() {
            return None;
        }
        Some(self.visible_raw_lines[start..=end].join("\n"))
    }

    pub fn selected_process_raw_text(&self) -> Option<String> {
        let process = self.selected_process()?;
        let mut lines = Vec::new();
        for entry in process.logs.iter() {
            let text = strip_carriage(&sanitize_text(&entry.text, true));
            for line in text.lines() {
                lines.push(line.to_string());
            }
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.set_status_message_with_level(message, StatusLevel::Info, Some(Duration::from_secs(3)));
    }

    pub fn set_status_warning_for(&mut self, message: impl Into<String>, ttl: Duration) {
        self.set_status_message_with_level(message, StatusLevel::Warning, Some(ttl));
    }

    pub fn set_status_warning_persistent(&mut self, message: impl Into<String>) {
        self.set_status_message_with_level(message, StatusLevel::Warning, None);
    }

    fn set_status_message_with_level(
        &mut self,
        message: impl Into<String>,
        level: StatusLevel,
        ttl: Option<Duration>,
    ) {
        self.status_message = Some(StatusMessage {
            text: message.into(),
            at: Instant::now(),
            ttl,
            level,
        });
    }

    fn log_row_at(&self, row: u16, col: u16) -> Option<usize> {
        let viewport = self.log_viewport?;
        if row < viewport.y || row >= viewport.y + viewport.height {
            return None;
        }
        if col < viewport.x || col >= viewport.x + viewport.width {
            return None;
        }
        Some((row - viewport.y) as usize)
    }

    fn current_selection_scope(&self) -> SelectionScope {
        if self.timeline_view {
            SelectionScope::Timeline
        } else {
            SelectionScope::Process(self.selected)
        }
    }

    fn freeze_follow_for_selection(&mut self) {
        if self.timeline_view {
            self.timeline_follow = false;
            return;
        }
        if let Some(process) = self.selected_process_mut() {
            process.follow = false;
        }
    }

    fn selection_scope_matches(&self, scope: SelectionScope) -> bool {
        match scope {
            SelectionScope::Timeline => self.timeline_view,
            SelectionScope::Process(id) => !self.timeline_view && self.selected == id,
        }
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
            let tag = process
                .spec
                .tags
                .first()
                .map(|s| s.as_str())
                .unwrap_or("Ungrouped");

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

fn strip_carriage(text: &str) -> String {
    text.rsplit('\r').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::output::LogLine;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn make_spec(name: &str) -> ProcessSpec {
        ProcessSpec {
            name: name.to_string(),
            cmd: "echo".to_string(),
            args: Vec::new(),
            cwd: None,
            color: None,
            env: HashMap::new(),
            restart_on_fail: false,
            follow: true,
            pre_cmd: None,
            watch_paths: Vec::new(),
            watch_ignore: Vec::new(),
            watch_ignore_gitignore: false,
            watch_debounce_ms: 200,
            depends_on: Vec::new(),
            ready_check: None,
            tags: Vec::new(),
        }
    }

    fn make_app() -> App {
        App::new(vec![make_spec("api")], 100, false, true)
    }

    #[test]
    fn selection_range_normalizes_and_clamps() {
        let mut app = make_app();
        app.selection_scope = Some(SelectionScope::Process(0));
        app.selection_start = Some(3);
        app.selection_end = Some(1);
        let range = app.selection_range_for(2).unwrap();
        assert_eq!(range, (1, 1));
    }

    #[test]
    fn selection_text_joins_visible_lines() {
        let mut app = make_app();
        app.selection_scope = Some(SelectionScope::Process(0));
        app.selection_start = Some(0);
        app.selection_end = Some(1);
        app.visible_raw_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(app.selection_text().unwrap(), "a\nb");
    }

    #[test]
    fn selected_process_raw_text_strips_ansi_and_skips_pretty() {
        let mut app = make_app();
        if let Some(process) = app.processes.get_mut(0) {
            process.logs.push(LogLine {
                text: "\u{1b}[31mred\u{1b}[0m".to_string(),
                stream: StreamKind::Stdout,
            });
            process.logs.push(LogLine {
                text: "{\"a\":1}".to_string(),
                stream: StreamKind::Stdout,
            });
        }
        app.json_formatting = true;
        assert_eq!(app.selected_process_raw_text().unwrap(), "red\n{\"a\":1}");
    }

    #[test]
    fn mouse_selection_freezes_follow() {
        let mut app = make_app();
        app.set_log_viewport(LogViewport {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        app.process_list_width = 0;
        app.processes[0].follow = true;
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert!(!app.processes[0].follow);
        assert!(app.selection_active);
    }

    #[test]
    fn selection_scope_mismatch_returns_none() {
        let mut app = make_app();
        app.selection_scope = Some(SelectionScope::Process(0));
        app.selection_start = Some(0);
        app.selection_end = Some(1);
        app.timeline_view = true;
        assert!(app.selection_range().is_none());
    }

    #[test]
    fn clear_selection_resets_state() {
        let mut app = make_app();
        app.selection_scope = Some(SelectionScope::Process(0));
        app.selection_start = Some(0);
        app.selection_end = Some(1);
        app.selection_active = true;
        app.clear_selection();
        assert!(app.selection_scope.is_none());
        assert!(app.selection_start.is_none());
        assert!(app.selection_end.is_none());
        assert!(!app.selection_active);
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
