use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::PendingApproval;

/// Self-contained multi-line text input.
///
/// Replaces `tui-textarea` (which lags ratatui releases) with a small editor
/// owned by Talon. The editing surface is intentionally minimal — the keys the
/// TUI event loop needs and nothing more:
///
/// - printable chars → insert at cursor
/// - Enter → newline (Ctrl/Alt+Enter submit is handled by the event loop)
/// - Backspace / Delete → delete around cursor (joins lines at boundaries)
/// - ← → ↑ ↓ / Home / End → cursor movement
///
/// Cursor column is tracked as a `char` index; display width is computed with
/// `unicode-width` when placing the terminal cursor.
#[derive(Debug, Clone)]
pub struct Input {
    lines: Vec<String>,
    /// Cursor line (0-based).
    row: usize,
    /// Cursor column as a char index within `lines[row]`.
    col: usize,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
        }
    }
}

impl Input {
    /// Create a fresh, empty input.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current buffer, one entry per line. Always at least one element.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The full buffer as a single `\n`-joined string.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Reset to a single empty line with the cursor at the start.
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.row = 0;
        self.col = 0;
    }

    fn line_chars(&self, row: usize) -> Vec<char> {
        self.lines[row].chars().collect()
    }

    fn set_line(&mut self, row: usize, chars: &[char]) {
        self.lines[row] = chars.iter().collect();
    }

    /// Feed a terminal event. Only key-press events mutate the buffer; every
    /// other event (mouse, resize, key release) is ignored.
    pub fn input(&mut self, event: &Event) {
        let Event::Key(key) = event else {
            return;
        };
        // Ignore key-release noise (Windows emits Release events).
        if key.kind == KeyEventKind::Release {
            return;
        }

        match key.code {
            KeyCode::Char(c) => {
                // Control/Alt/Super combos are commands handled upstream, not text.
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                {
                    return;
                }
                let mut chars = self.line_chars(self.row);
                chars.insert(self.col, c);
                self.set_line(self.row, &chars);
                self.col += 1;
            }
            KeyCode::Enter => {
                let chars = self.line_chars(self.row);
                let head: String = chars[..self.col].iter().collect();
                let tail: String = chars[self.col..].iter().collect();
                self.lines[self.row] = head;
                self.lines.insert(self.row + 1, tail);
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Backspace => {
                if self.col > 0 {
                    let mut chars = self.line_chars(self.row);
                    chars.remove(self.col - 1);
                    self.set_line(self.row, &chars);
                    self.col -= 1;
                } else if self.row > 0 {
                    let current = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.lines[self.row].chars().count();
                    self.lines[self.row].push_str(&current);
                }
            }
            KeyCode::Delete => {
                let len = self.lines[self.row].chars().count();
                if self.col < len {
                    let mut chars = self.line_chars(self.row);
                    chars.remove(self.col);
                    self.set_line(self.row, &chars);
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].push_str(&next);
                }
            }
            KeyCode::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.lines[self.row].chars().count();
                }
            }
            KeyCode::Right => {
                let len = self.lines[self.row].chars().count();
                if self.col < len {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            KeyCode::Up if self.row > 0 => {
                self.row -= 1;
                self.col = self.col.min(self.lines[self.row].chars().count());
            }
            KeyCode::Down if self.row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = self.col.min(self.lines[self.row].chars().count());
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.lines[self.row].chars().count(),
            _ => {}
        }
    }

    /// True when the buffer holds no text.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Display column (terminal cells) of the cursor within its line.
    fn cursor_display_col(&self) -> u16 {
        let prefix: String = self.lines[self.row].chars().take(self.col).collect();
        UnicodeWidthStr::width(prefix.as_str()) as u16
    }
}

/// Renders an [`Input`] inside a titled block, placeholder and cursor included.
pub struct InputBar;

impl InputBar {
    /// Render the input and position the terminal cursor.
    pub fn render(
        frame: &mut Frame,
        area: Rect,
        input: &Input,
        thinking: bool,
        pending_approval: Option<&PendingApproval>,
    ) {
        let title = if let Some(approval) = pending_approval {
            format!(" Approve {}? [y/n] ", approval.tool_name)
        } else if thinking {
            " Thinking… ".to_string()
        } else {
            " Message (Ctrl+Enter to send) ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(if thinking {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Blue)
            });

        let body = if input.is_empty() {
            Text::from(Line::styled(
                "Type a message…",
                Style::default().fg(Color::DarkGray),
            ))
        } else {
            Text::from(
                input
                    .lines()
                    .iter()
                    .map(|l| Line::raw(l.clone()))
                    .collect::<Vec<_>>(),
            )
        };

        frame.render_widget(Paragraph::new(body).block(block), area);

        // Place the cursor inside the bordered area (1-cell border offset).
        let cursor_x = area.x + 1 + input.cursor_display_col();
        let cursor_y = area.y + 1 + input.row as u16;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::empty()))
    }

    #[test]
    fn new_input_is_empty() {
        let input = Input::new();
        assert!(input.is_empty());
        assert_eq!(input.lines(), &["".to_string()]);
        assert_eq!(input.text(), "");
    }

    #[test]
    fn typing_inserts_chars() {
        let mut input = Input::new();
        for c in "hi".chars() {
            input.input(&key(KeyCode::Char(c)));
        }
        assert_eq!(input.text(), "hi");
        assert!(!input.is_empty());
    }

    #[test]
    fn enter_splits_into_lines() {
        let mut input = Input::new();
        input.input(&key(KeyCode::Char('a')));
        input.input(&key(KeyCode::Enter));
        input.input(&key(KeyCode::Char('b')));
        assert_eq!(input.text(), "a\nb");
        assert_eq!(input.lines().len(), 2);
    }

    #[test]
    fn backspace_deletes_and_joins() {
        let mut input = Input::new();
        input.input(&key(KeyCode::Char('a')));
        input.input(&key(KeyCode::Enter));
        // cursor at start of line 2 → backspace joins the two lines
        input.input(&key(KeyCode::Backspace));
        assert_eq!(input.text(), "a");
        // delete the remaining char
        input.input(&key(KeyCode::Backspace));
        assert!(input.is_empty());
    }

    #[test]
    fn ctrl_char_is_not_inserted() {
        let mut input = Input::new();
        input.input(&Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(input.is_empty());
    }

    #[test]
    fn clear_resets_buffer() {
        let mut input = Input::new();
        input.input(&key(KeyCode::Char('x')));
        input.clear();
        assert!(input.is_empty());
    }
}
