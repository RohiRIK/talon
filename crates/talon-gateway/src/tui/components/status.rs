use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::StatusInfo;

/// One-line status bar showing model, session, tokens, and sandbox mode.
pub struct StatusBar;

impl StatusBar {
    pub fn render(frame: &mut Frame, area: Rect, status: &StatusInfo) {
        let mut spans = vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                &*status.model,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
        ];

        // Session ID (truncated to 8 chars for readability)
        let session_short = if status.session_id.len() > 8 {
            format!("sess:{}", &status.session_id[..8])
        } else {
            format!("sess:{}", status.session_id)
        };
        spans.push(Span::styled(
            session_short,
            Style::default().fg(Color::DarkGray),
        ));

        // Token count
        if let Some(tokens) = status.token_count {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("{tokens} tok"),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Thinking indicator
        if status.thinking {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "⠿ thinking",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Native sandbox badge — always visible when active
        if status.native_sandbox {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "[NATIVE]",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let para = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(para, area);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_info_default_not_thinking() {
        let s = StatusInfo::default();
        assert!(!s.thinking);
        assert!(!s.native_sandbox);
        assert!(s.token_count.is_none());
    }

    #[test]
    fn status_info_model_name() {
        let s = StatusInfo {
            model: "claude-opus-4-7".to_string(),
            ..Default::default()
        };
        assert_eq!(s.model, "claude-opus-4-7");
    }
}
