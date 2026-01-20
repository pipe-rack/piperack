//! Terminal User Interface (TUI) rendering and management.
//!
//! This module handles initializing the terminal in raw mode, restoring it on exit,
//! and drawing the application state using `ratatui`.

use std::io::{self, Stdout};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::ansi::ansi_spans;
use crate::app::{App, InputMode};
use crate::output::sanitize_text;
use crate::process::ProcessStatus;

/// Type alias for the specific terminal backend used.
pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Initializes the terminal for TUI mode.
///
/// Enables raw mode, enters the alternate screen, and creates a `ratatui` Terminal instance.
pub fn init_terminal() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// Restores the terminal to its original state.
///
/// Disables raw mode, leaves the alternate screen, and shows the cursor.
pub fn restore_terminal(mut terminal: TuiTerminal) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Draws the current application state to the terminal.
pub fn draw(app: &mut App, terminal: &mut TuiTerminal) -> io::Result<()> {
    let title = window_title(app);
    execute!(terminal.backend_mut(), SetTitle(title))?;
    terminal.draw(|frame| {
        let area = frame.size();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(4)])
            .split(area);
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(vertical[0]);
        
        app.process_list_width = main[0].width;

        let mut list_items = Vec::new();
        let mut ui_selected_index = 0;
        let mut current_ui_index = 0;
        let mut last_tag: Option<String> = None;

        for (proc_idx, process) in app.processes.iter().enumerate() {
            let tag = process.spec.tags.first().map(|s| s.as_str()).unwrap_or("Ungrouped");
            
            if last_tag.as_deref() != Some(tag) {
                // Add header
                let header = ListItem::new(Line::from(vec![
                    Span::styled("▼ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(tag, Style::default().fg(Color::DarkGray)),
                ]));
                list_items.push(header);
                current_ui_index += 1;
                last_tag = Some(tag.to_string());
            }

            // Calculate if this is the selected item
            let is_selected = proc_idx == app.selected;
            if is_selected {
                ui_selected_index = current_ui_index;
            }

            let status = status_char(&process.status, app.use_symbols);
            let preview = process
                .logs
                .iter()
                .last()
                .map(|l| strip_carriage(&sanitize_text(&l.text, true)))
                .unwrap_or_default();
            
            let mut text = Text::default();
            
            let (indent_str, name_mod) = if is_selected {
                ("▶ ", Modifier::BOLD)
            } else {
                ("  ", Modifier::empty())
            };

            // Use dimmed color for non-selected items
            let base_style = if is_selected {
                Style::default()
            } else {
                Style::default().fg(Color::Gray)
            };

            let name_style = if is_selected {
                process_color(process.spec.color.as_deref()).add_modifier(name_mod)
            } else {
                process_color(process.spec.color.as_deref())
            };

            text.lines.push(Line::from(vec![
                Span::styled(indent_str, if is_selected { Style::default().fg(Color::Cyan) } else { base_style }),
                Span::styled(format!("[{}] ", status), if is_selected { status_style(&process.status) } else { status_style(&process.status).add_modifier(Modifier::DIM) }),
                Span::styled(process.spec.name.clone(), name_style),
            ]));
            if !preview.is_empty() {
                let available_width = (main[0].width as usize).saturating_sub(4 + indent_str.len());
                let trimmed = truncate(&preview, available_width);
                text.lines.push(Line::from(vec![
                    Span::raw("  "), // indent preview
                    Span::styled(trimmed, base_style)
                ]));
            }
            list_items.push(ListItem::new(text));
            current_ui_index += 1;
        }

        let border_style = Style::default().fg(Color::DarkGray);
        let input_active = app
            .selected_process()
            .map(|process| process.input_active)
            .unwrap_or(false);
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title("Processes")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border_style),
            )
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(list, main[0], &mut list_state(ui_selected_index, current_ui_index));

        let log_block = Block::default()
            .title(log_title(app))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if input_active {
                Style::default().fg(Color::Green)
            } else {
                border_style
            });
        let log_area = log_block.inner(main[1]);
        let log_height = log_area.height as usize;
        app.set_log_view_height(log_height);

        let (log_lines, total) = render_log_lines(app, log_height, log_area.width as usize);
        let paragraph = Paragraph::new(log_lines).block(log_block).wrap(Wrap { trim: false });

        frame.render_widget(paragraph, main[1]);

        let status_line = app.status_line();
        let default_help = if app.use_symbols {
            "↑/↓ select | Tab cycle | Enter input | f follow | t timeline | a ansi | / search | F filter | n/N next/prev | r restart | g group | R all | k kill | j json | e export | q quit | ? help"
        } else {
            "Up/Down select | Tab cycle | Enter input | f follow | t timeline | a ansi | / search | F filter | n/N next/prev | r restart | g group | R all | k kill | j json | e export | q quit | ? help"
        };
        let mut help_line = app.status_message().unwrap_or(default_help).to_string();
        if app.input_mode == InputMode::Search {
            help_line = format!("Search: {} (Esc to exit)", app.input);
        } else if app.input_mode == InputMode::Filter {
            help_line = format!("Filter: {} (Esc to exit)", app.input);
        } else if app.input_mode == InputMode::Group {
            help_line = format!("Restart Group: {}", app.input);
        } else if app.input_mode == InputMode::Input {
            let cursor = if app.use_symbols { "▌" } else { "|" };
            let divider = if app.use_symbols { " · " } else { " | " };
            help_line = format!(
                "Input {} {}{}Enter to send{}Esc to exit",
                cursor,
                app.input_line(),
                divider,
                divider
            );
        }
        let status = Paragraph::new(Text::from(vec![
            Line::from(Span::raw(status_line)),
            Line::from(Span::styled(help_line, Style::default().fg(Color::DarkGray))),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style),
        );
        frame.render_widget(status, vertical[1]);

        if total == 0 {
            let empty = Paragraph::new("No output yet")
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default());
            frame.render_widget(empty, log_area);
        }

        if app.show_help {
            let popup_area = centered_rect(60, 60, area);
            let help_text = vec![
                "Navigation:",
                "  Up/Down    Select process",
                "  PageUp/Dn  Scroll logs",
                "  Home/End   Scroll to top/bottom",
                "  Tab        Cycle selection",
                "",
                "Actions:",
                "  Enter      Send input to process",
                "  f          Toggle auto-follow",
                "  t          Toggle timeline view",
                "  a          Toggle ANSI stripping",
                "  j          Toggle JSON formatting",
                "  r          Restart selected",
                "  k          Kill selected",
                "  R          Restart ALL",
                "  g          Restart Group (by tag)",
                "  e          Export logs to file",
                "",
                "Search & Filter:",
                "  /          Search (jump to match)",
                "  n/N        Next/Prev match",
                "  F          Filter (hide non-matching)",
                "",
                "General:",
                "  ?          Toggle this help",
                "  q          Quit",
            ]
            .join("\n");

            let help_block = Paragraph::new(help_text)
                .block(
                    Block::default()
                        .title("Help")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded),
                )
                .style(Style::default().bg(Color::DarkGray).fg(Color::White));
            
            // Clear the area first to ensure readability? 
            // Ratatui renders back-to-front, so just drawing on top is fine.
            // But we need a clearing widget if the background is transparent.
            // Paragraph with bg color is enough.
            
            frame.render_widget(ratatui::widgets::Clear, popup_area);
            frame.render_widget(help_block, popup_area);
        }
    })?;
    Ok(())
}

fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn window_title(app: &App) -> String {
    if app.timeline_view {
        return "piperack · timeline".to_string();
    }
    if let Some(process) = app.selected_process() {
        format!("piperack · {}", process.spec.name)
    } else {
        "piperack".to_string()
    }
}

fn log_title(app: &App) -> String {
    if app.timeline_view {
        return "Timeline".to_string();
    }
    if let Some(process) = app.selected_process() {
        match &process.status {
            ProcessStatus::Running => format!("Logs - {} (running)", process.spec.name),
            ProcessStatus::Exited { code } => format!("Logs - {} (exited {:?})", process.spec.name, code),
            ProcessStatus::Failed { .. } => format!("Logs - {} (failed)", process.spec.name),
            ProcessStatus::Starting => format!("Logs - {} (starting)", process.spec.name),
            ProcessStatus::Idle => format!("Logs - {} (idle)", process.spec.name),
        }
    } else {
        "Logs".to_string()
    }
}

fn render_log_lines(app: &App, height: usize, width: usize) -> (Text<'static>, usize) {
    if height == 0 {
        return (Text::default(), 0);
    }

    // Helper to process a single log line
    let process_line = |text: &str, name: &str, color: Option<&str>| -> Vec<Line<'static>> {
        let plain = strip_carriage(&sanitize_text(text, true));
        if let Some(query) = &app.filter_query {
            if !plain.contains(query) {
                return Vec::new();
            }
        }

        let content_plain = if app.json_formatting {
            crate::output::format_json(&plain)
        } else {
            plain.clone()
        };

        let name_style = process_color(color);
        let prefix = format!("{} \u{203a} ", name);
        let prefix_len = prefix.chars().count();
        let indent = " ".repeat(prefix_len);
        let use_ansi = !app.strip_ansi && !app.json_formatting && app.search_query.is_none();

        if use_ansi {
            return text
                .lines()
                .enumerate()
                .map(|(i, line)| {
                    let current_prefix = if i == 0 { &prefix } else { &indent };
                    let mut spans = Vec::new();
                    spans.push(Span::styled(current_prefix.to_string(), name_style));
                    spans.extend(ansi_spans(line));
                    let trimmed = truncate_spans(spans, width.saturating_sub(1));
                    Line::from(trimmed)
                })
                .collect();
        }

        content_plain.lines().enumerate().map(|(i, line)| {
            let current_prefix = if i == 0 { &prefix } else { &indent };
            let combined = format!("{}{}", current_prefix, line);
            let trimmed = truncate(&combined, width.saturating_sub(1));
            
            // Highlighting logic
            if let Some(query) = &app.search_query {
                if !query.is_empty() && trimmed.contains(query) {
                    let mut spans = Vec::new();
                    let highlight_style = Style::default().fg(Color::Black).bg(Color::Yellow);
                    let mut last_idx = 0;

                    for (idx, match_str) in trimmed.match_indices(query) {
                        if idx > last_idx {
                            let pre_match = &trimmed[last_idx..idx];
                            // Apply prefix style if this part overlaps with prefix
                            // This is complex because prefix is also styled.
                            // Simplification: Apply standard prefix styling logic to the whole chunk,
                            // but that's hard if chunk is split.
                            // Better approach: Re-construct spans from the highlighted chunks.
                            
                            // To handle prefix styling correctly with arbitrary highlighting is complex.
                            // We will prioritize highlighting.
                            // But we should try to keep prefix color if possible.
                            
                            // Let's iterate chars or use a simpler heuristic.
                            // If the chunk starts before prefix_len, it is part of prefix.
                            
                            // Actually, let's keep it simple: Highlighting overrides everything.
                            // For non-highlighted parts, we check if they belong to prefix.
                            
                            // Check if this span is fully inside prefix
                            // It's easier to just push spans and let them handle their own style?
                            // No, span style is fixed.
                            
                            spans.push(Span::raw(pre_match.to_string()));
                        }
                        spans.push(Span::styled(match_str.to_string(), highlight_style));
                        last_idx = idx + match_str.len();
                    }
                    if last_idx < trimmed.len() {
                        spans.push(Span::raw(trimmed[last_idx..].to_string()));
                    }
                    
                    // Now fix styles for non-highlighted parts
                    // This is a post-processing step on spans? 
                    // Or we just accept that searching breaks standard coloring for that line.
                    // Let's try to restore prefix color.
                    // This is getting complicated for a "quick" fix.
                    // The simplest "good enough" is: Highlight matches, everything else is raw/default.
                    // The prefix color is nice though.
                    
                    // Let's do this: Iterate the spans we just made.
                    // For each raw span, if it overlaps with the prefix range (0..prefix_len), style that intersection.
                    // Since `trimmed` includes prefix.
                    
                    let mut styled_spans = Vec::new();
                    let mut current_pos = 0;
                    let prefix_width = current_prefix.chars().count(); // approximation
                    
                    for span in spans {
                        let content = span.content.clone();
                        let len = content.chars().count();
                        if span.style == highlight_style {
                            styled_spans.push(span);
                        } else {
                            // This is a non-match span. Check overlap with prefix.
                            let end_pos = current_pos + len;
                            if current_pos < prefix_width {
                                // Simple heuristic: if it ends within or at prefix width, style as prefix.
                                if end_pos <= prefix_width {
                                    styled_spans.push(Span::styled(content, name_style));
                                } else if current_pos >= prefix_width {
                                    styled_spans.push(Span::raw(content));
                                } else {
                                    // Overlaps boundary. Use raw to avoid complexity.
                                    styled_spans.push(Span::raw(content));
                                }
                            } else {
                                styled_spans.push(Span::raw(content));
                            }
                        }
                        current_pos += len;
                    }
                    return Line::from(styled_spans);
                }
            }

            if trimmed.starts_with(current_prefix) {
                let rest = trimmed.strip_prefix(current_prefix).unwrap_or("").to_string();
                Line::from(vec![
                    Span::styled(current_prefix.to_string(), name_style),
                    Span::raw(rest),
                ])
            } else {
                Line::from(Span::raw(trimmed))
            }
        }).collect()
    };

    let mut lines = Vec::new();
    let mut total_filtered = 0;

    if app.timeline_view {
        let _total = app.timeline.len();
        // For timeline, iterating everything might be slow if huge buffer.
        // But for <50k lines it's usually instant in Rust.
        // We collect all matching lines to calculate scroll.
        
        // Optimization: if no filter and not json, keep old logic?
        // Let's rely on speed for now.
        
        // We need to support scrolling.
        // It's hard to map 'scroll' index to filtered index efficiently without caching.
        // Simple approach: Collect ALL matching display lines, then slice.
        
        let mut all_lines = Vec::new();
        for entry in app.timeline.iter() {
             let (name, color) = app.processes.get(entry.process_id)
                .map(|p| (p.spec.name.as_str(), p.spec.color.as_deref()))
                .unwrap_or(("process", None));
             all_lines.extend(process_line(&entry.text, name, color));
        }
        total_filtered = all_lines.len();
        
        let start = if app.timeline_follow {
            total_filtered.saturating_sub(height)
        } else {
            app.timeline_scroll.min(total_filtered.saturating_sub(height))
        };
        let end = (start + height).min(total_filtered);
        lines = all_lines[start..end].to_vec();

    } else if let Some(process) = app.selected_process() {
        let mut all_lines = Vec::new();
        let name = process.spec.name.as_str();
        let color = process.spec.color.as_deref();
        
        for entry in process.logs.iter() {
            // Strip existing prefix if present in raw log to avoid double prefixing?
            // The original logic stripped it.
            let text = strip_existing_prefix(name, &entry.text);
            all_lines.extend(process_line(&text, name, color));
        }
        total_filtered = all_lines.len();

        let start = if process.follow {
            total_filtered.saturating_sub(height)
        } else {
            process.scroll.min(total_filtered.saturating_sub(height))
        };
        let end = (start + height).min(total_filtered);
        lines = all_lines[start..end].to_vec();
    }

    (Text::from(lines), total_filtered)
}

fn list_state(selected: usize, len: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if len > 0 {
        state.select(Some(selected.min(len - 1)));
    }
    state
}

fn process_color(name: Option<&str>) -> Style {
    if let Some(color) = name {
        return Style::default().fg(color_from_name(color).unwrap_or(Color::White));
    }
    Style::default().fg(Color::White)
}

fn color_from_name(name: &str) -> Option<Color> {
    match name.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "white" => Some(Color::White),
        _ => None,
    }
}

fn status_char(status: &ProcessStatus, use_symbols: bool) -> char {
    if use_symbols {
        return match status {
            ProcessStatus::Idle => '·',
            ProcessStatus::Starting => '↻',
            ProcessStatus::Running => '▲',
            ProcessStatus::Exited { .. } => '■',
            ProcessStatus::Failed { .. } => '■',
        };
    }
    match status {
        ProcessStatus::Idle => '.',
        ProcessStatus::Starting => 'S',
        ProcessStatus::Running => 'R',
        ProcessStatus::Exited { code } => {
            if code.unwrap_or(1) == 0 {
                'E'
            } else {
                'X'
            }
        }
        ProcessStatus::Failed { .. } => 'F',
    }
}

fn strip_existing_prefix(name: &str, text: &str) -> String {
    let candidates = [
        format!("[{}] ", name),
        format!("[{}]", name),
        format!("{} \u{203a} ", name),
        format!("{}: ", name),
        format!("{} - ", name),
    ];
    for candidate in candidates {
        if let Some(rest) = text.strip_prefix(&candidate) {
            return rest.trim_start().to_string();
        }
    }
    text.to_string()
}

fn status_style(status: &ProcessStatus) -> Style {
    match status {
        ProcessStatus::Idle => Style::default().fg(Color::DarkGray),
        ProcessStatus::Starting => Style::default().fg(Color::Yellow),
        ProcessStatus::Running => Style::default().fg(Color::Green),
        ProcessStatus::Exited { code } => {
            if code.unwrap_or(1) == 0 {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(Color::Red)
            }
        }
        ProcessStatus::Failed { .. } => Style::default().fg(Color::Red),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.len() <= max {
        return text.to_string();
    }
    let mut out = text.chars().take(max.saturating_sub(1)).collect::<String>();
    out.push('~');
    out
}

fn truncate_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    if max == 0 {
        return Vec::new();
    }
    let total_len: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if total_len <= max {
        return spans;
    }

    let mut remaining = max.saturating_sub(1);
    let mut out = Vec::new();
    for span in spans {
        if remaining == 0 {
            break;
        }
        let content = span.content.as_ref();
        let count = content.chars().count();
        if count <= remaining {
            out.push(span);
            remaining -= count;
        } else {
            let truncated = content.chars().take(remaining).collect::<String>();
            out.push(Span::styled(truncated, span.style));
            remaining = 0;
        }
    }

    if let Some(last) = out.last_mut() {
        let mut content = last.content.to_string();
        content.push('~');
        last.content = content.into();
    } else {
        out.push(Span::raw("~"));
    }
    out
}

fn strip_carriage(text: &str) -> String {
    text.rsplit('\r').next().unwrap_or("").to_string()
}
