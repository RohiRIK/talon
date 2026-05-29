use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders},
};
use tui_textarea::TextArea;

use crate::tui::app::PendingApproval;

/// Multi-line input bar using `tui-textarea`.
///
/// - Ctrl+Enter / Alt+Enter → submit
/// - ↑↓ → history navigation (managed by the TuiGateway event loop)
/// - Tab → autocomplete placeholder (Phase 5)
pub struct InputBar;

impl InputBar {
    /// Render the input textarea.
    pub fn render(
        frame: &mut Frame,
        area: Rect,
        textarea: &mut TextArea,
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

        textarea.set_block(block);
        frame.render_widget(&*textarea, area);
    }

    /// Create a fresh `TextArea` with Talon styling.
    pub fn new_textarea() -> TextArea<'static> {
        let mut ta = TextArea::default();
        ta.set_cursor_line_style(Style::default());
        ta.set_placeholder_text("Type a message…");
        ta.set_placeholder_style(Style::default().fg(Color::DarkGray));
        ta
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_textarea_is_empty() {
        let ta = InputBar::new_textarea();
        assert_eq!(ta.lines(), &["".to_string()]);
    }
}
