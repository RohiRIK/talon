use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use crate::tui::app::{ActiveTool, ToolState};

/// Collapsible panel showing active and completed tool calls.
///
/// Tab toggles visibility. Each tool call shows a spinner while running
/// and a green/red indicator on completion.
pub struct ToolPanel;

impl ToolPanel {
    pub fn render(frame: &mut Frame, area: Rect, tool_calls: &[ActiveTool]) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Tools (Tab to toggle) ");

        let items: Vec<ListItem> = tool_calls
            .iter()
            .map(|t| {
                let (icon, style) = match t.state {
                    ToolState::Running => (
                        "⠿ ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    ToolState::Done => ("✓ ", Style::default().fg(Color::Green)),
                    ToolState::Error => ("✗ ", Style::default().fg(Color::Red)),
                };

                let name_span = Span::styled(t.name.clone(), style);
                let icon_span = Span::styled(icon.to_string(), style);

                let output_line = t.output.as_deref().map(|o| {
                    // Truncate to 60 chars to fit the panel.
                    let truncated = if o.len() > 60 {
                        format!("{}…", &o[..57])
                    } else {
                        o.to_string()
                    };
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(truncated, Style::default().fg(Color::DarkGray)),
                    ])
                });

                let mut lines = vec![Line::from(vec![icon_span, name_span])];
                if let Some(out) = output_line {
                    lines.push(out);
                }

                ListItem::new(lines)
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_state_variants_are_distinct() {
        assert_ne!(ToolState::Running, ToolState::Done);
        assert_ne!(ToolState::Done, ToolState::Error);
    }

    #[test]
    fn active_tool_fields() {
        let t = ActiveTool {
            id: "t1".to_string(),
            name: "read_file".to_string(),
            state: ToolState::Running,
            output: None,
        };
        assert_eq!(t.name, "read_file");
        assert!(t.output.is_none());
    }
}
