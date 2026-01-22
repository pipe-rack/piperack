//! ANSI escape sequence parsing for TUI rendering.
//!
//! This module converts ANSI-colored text into Ratatui spans so the TUI can render
//! colors safely without leaking control characters into the terminal.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

#[derive(Debug, Clone)]
struct AnsiState {
    fg: Option<Color>,
    bg: Option<Color>,
    modifiers: Modifier,
}

impl Default for AnsiState {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            modifiers: Modifier::empty(),
        }
    }
}

impl AnsiState {
    fn to_style(&self) -> Style {
        let mut style = Style::default();
        if let Some(color) = self.fg {
            style = style.fg(color);
        }
        if let Some(color) = self.bg {
            style = style.bg(color);
        }
        if !self.modifiers.is_empty() {
            style = style.add_modifier(self.modifiers);
        }
        style
    }
}

pub fn ansi_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buffer = String::new();
    let mut state = AnsiState::default();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                let mut params = String::new();
                let mut final_byte = None;
                while let Some(&c) = chars.peek() {
                    if ('@'..='~').contains(&c) {
                        final_byte = Some(c);
                        chars.next();
                        break;
                    }
                    params.push(c);
                    chars.next();
                }
                if final_byte == Some('m') {
                    flush_span(&mut spans, &mut buffer, &state);
                    apply_sgr(&mut state, &params);
                }
                continue;
            }
            if matches!(chars.peek(), Some(']')) {
                // OSC sequence: skip until BEL or ESC \
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && matches!(chars.peek(), Some('\\')) {
                        chars.next();
                        break;
                    }
                }
                continue;
            }
            // Unknown escape: drop the ESC byte to avoid terminal corruption.
            continue;
        }
        if ch == '\r' {
            // Carriage return: overwrite line from start. Keep only last segment.
            flush_span(&mut spans, &mut buffer, &state);
            spans.clear();
            continue;
        }
        buffer.push(ch);
    }
    flush_span(&mut spans, &mut buffer, &state);
    spans
}

fn flush_span(spans: &mut Vec<Span<'static>>, buffer: &mut String, state: &AnsiState) {
    if buffer.is_empty() {
        return;
    }
    spans.push(Span::styled(std::mem::take(buffer), state.to_style()));
}

fn apply_sgr(state: &mut AnsiState, params: &str) {
    let values = parse_params(params);
    let mut i = 0;
    while i < values.len() {
        match values[i] {
            0 => {
                *state = AnsiState::default();
                i += 1;
            }
            1 => {
                add_modifier(state, Modifier::BOLD);
                i += 1;
            }
            2 => {
                add_modifier(state, Modifier::DIM);
                i += 1;
            }
            3 => {
                add_modifier(state, Modifier::ITALIC);
                i += 1;
            }
            4 => {
                add_modifier(state, Modifier::UNDERLINED);
                i += 1;
            }
            5 => {
                add_modifier(state, Modifier::SLOW_BLINK);
                i += 1;
            }
            6 => {
                add_modifier(state, Modifier::RAPID_BLINK);
                i += 1;
            }
            7 => {
                add_modifier(state, Modifier::REVERSED);
                i += 1;
            }
            8 => {
                add_modifier(state, Modifier::HIDDEN);
                i += 1;
            }
            9 => {
                add_modifier(state, Modifier::CROSSED_OUT);
                i += 1;
            }
            22 => {
                remove_modifier(state, Modifier::BOLD | Modifier::DIM);
                i += 1;
            }
            23 => {
                remove_modifier(state, Modifier::ITALIC);
                i += 1;
            }
            24 => {
                remove_modifier(state, Modifier::UNDERLINED);
                i += 1;
            }
            25 => {
                remove_modifier(state, Modifier::SLOW_BLINK | Modifier::RAPID_BLINK);
                i += 1;
            }
            27 => {
                remove_modifier(state, Modifier::REVERSED);
                i += 1;
            }
            28 => {
                remove_modifier(state, Modifier::HIDDEN);
                i += 1;
            }
            29 => {
                remove_modifier(state, Modifier::CROSSED_OUT);
                i += 1;
            }
            30..=37 => {
                state.fg = basic_color(values[i] - 30, false);
                i += 1;
            }
            90..=97 => {
                state.fg = basic_color(values[i] - 90, true);
                i += 1;
            }
            40..=47 => {
                state.bg = basic_color(values[i] - 40, false);
                i += 1;
            }
            100..=107 => {
                state.bg = basic_color(values[i] - 100, true);
                i += 1;
            }
            39 => {
                state.fg = None;
                i += 1;
            }
            49 => {
                state.bg = None;
                i += 1;
            }
            38 | 48 => {
                let is_fg = values[i] == 38;
                if let Some((advance, color)) = parse_extended_color(&values[i + 1..]) {
                    if is_fg {
                        state.fg = Some(color);
                    } else {
                        state.bg = Some(color);
                    }
                    i += 1 + advance;
                } else {
                    i += 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
}

fn parse_params(params: &str) -> Vec<i32> {
    if params.is_empty() {
        return vec![0];
    }
    let mut values = Vec::new();
    for part in params.split(';') {
        if part.is_empty() {
            values.push(0);
        } else if let Ok(value) = part.parse::<i32>() {
            values.push(value);
        }
    }
    if values.is_empty() {
        values.push(0);
    }
    values
}

fn parse_extended_color(values: &[i32]) -> Option<(usize, Color)> {
    if values.is_empty() {
        return None;
    }
    match values[0] {
        5 => {
            let index = *values.get(1)?;
            let index = u8::try_from(index).ok()?;
            Some((2, Color::Indexed(index)))
        }
        2 => {
            let r = *values.get(1)?;
            let g = *values.get(2)?;
            let b = *values.get(3)?;
            let r = u8::try_from(r).ok()?;
            let g = u8::try_from(g).ok()?;
            let b = u8::try_from(b).ok()?;
            Some((4, Color::Rgb(r, g, b)))
        }
        _ => None,
    }
}

fn add_modifier(state: &mut AnsiState, modifier: Modifier) {
    state.modifiers = state.modifiers.union(modifier);
}

fn remove_modifier(state: &mut AnsiState, modifier: Modifier) {
    state.modifiers = state.modifiers.difference(modifier);
}

fn basic_color(index: i32, bright: bool) -> Option<Color> {
    let color = match (index, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        (7, true) => Color::White,
        _ => return None,
    };
    Some(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_spans_plain_text() {
        let spans = ansi_spans("hello");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello");
        assert_eq!(spans[0].style.fg, None);
    }

    #[test]
    fn ansi_spans_respects_sgr_color() {
        let spans = ansi_spans("\u{1b}[31mred\u{1b}[0m");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "red");
        assert_eq!(spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn ansi_spans_skips_osc_sequences() {
        let spans = ansi_spans("hi\u{1b}]0;title\u{7}there");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hithere");
    }

    #[test]
    fn ansi_spans_handles_carriage_return() {
        let spans = ansi_spans("abc\rdef");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "def");
    }

    #[test]
    fn parse_params_defaults_to_reset() {
        assert_eq!(parse_params(""), vec![0]);
        assert_eq!(parse_params(";"), vec![0, 0]);
        assert_eq!(parse_params("1;"), vec![1, 0]);
    }

    #[test]
    fn parse_extended_color_handles_index_and_rgb() {
        let indexed = parse_extended_color(&[5, 120]).unwrap();
        assert_eq!(indexed.0, 2);
        assert_eq!(indexed.1, Color::Indexed(120));

        let rgb = parse_extended_color(&[2, 1, 2, 3]).unwrap();
        assert_eq!(rgb.0, 4);
        assert_eq!(rgb.1, Color::Rgb(1, 2, 3));

        assert!(parse_extended_color(&[9]).is_none());
    }

    #[test]
    fn basic_color_rejects_out_of_range() {
        assert!(basic_color(9, false).is_none());
    }
}
