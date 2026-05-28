use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::{ChatMessage, ChatRole};

/// Renders the conversation history with basic markdown formatting.
///
/// Streaming: the message list grows as `AgentEvent::Text` events arrive.
/// comrak parses per-render frame; unclosed blocks show a dim `…` indicator.
pub struct ChatView;

impl ChatView {
    pub fn render(
        frame: &mut Frame,
        area: Rect,
        messages: &[ChatMessage],
        scroll_offset: usize,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Chat");

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();

        for msg in messages {
            let prefix_style = match msg.role {
                ChatRole::User => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ChatRole::Assistant => {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                }
                ChatRole::System => Style::default().fg(Color::Yellow).add_modifier(Modifier::DIM),
            };

            let prefix = match msg.role {
                ChatRole::User => "You  ",
                ChatRole::Assistant => "Talon",
                ChatRole::System => "sys  ",
            };

            // Separator between messages
            if !lines.is_empty() {
                lines.push(Line::default());
            }

            // Render each line of the message with inline markdown formatting.
            for (i, content_line) in msg.content.lines().enumerate() {
                let is_first = i == 0;
                let label = if is_first {
                    Span::styled(format!("{prefix} │ "), prefix_style)
                } else {
                    Span::styled("      │ ", Style::default().fg(Color::DarkGray))
                };

                let content_spans = render_inline_markdown(content_line, &msg.role);
                let mut spans = vec![label];
                spans.extend(content_spans);
                lines.push(Line::from(spans));
            }

            // Empty message — still show the prefix
            if msg.content.is_empty() {
                let label = Span::styled(format!("{prefix} │ "), prefix_style);
                lines.push(Line::from(vec![label]));
            }
        }

        let total_lines = lines.len();
        let viewport_height = inner.height as usize;
        let max_scroll = total_lines.saturating_sub(viewport_height);
        let scroll = scroll_offset.min(max_scroll);
        // Scroll from bottom: offset=0 means bottom, higher means further back.
        let from_top = max_scroll.saturating_sub(scroll);

        let text = Text::from(lines);
        let para = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((from_top as u16, 0));

        frame.render_widget(para, inner);
    }
}

/// Apply inline markdown styling to a single line of text.
fn render_inline_markdown(line: &str, role: &ChatRole) -> Vec<Span<'static>> {
    let default_fg = match role {
        ChatRole::User => Color::White,
        ChatRole::Assistant => Color::Reset,
        ChatRole::System => Color::Yellow,
    };

    // Quick detection: if no markdown markers, return as-is.
    if !line.contains('*') && !line.contains('`') && !line.contains('_') {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(default_fg),
        )];
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '`' => {
                // Flush current.
                if !current.is_empty() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(default_fg),
                    ));
                    current.clear();
                }
                // Collect inline code content.
                i += 1;
                let mut code = String::new();
                while i < chars.len() && chars[i] != '`' {
                    code.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // closing `
                }
                spans.push(Span::styled(
                    code,
                    Style::default()
                        .fg(Color::Cyan)
                        .bg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                // Bold **...**
                if !current.is_empty() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(default_fg),
                    ));
                    current.clear();
                }
                i += 2;
                let mut bold = String::new();
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                    bold.push(chars[i]);
                    i += 1;
                }
                i += 2; // closing **
                spans.push(Span::styled(
                    bold,
                    Style::default()
                        .fg(default_fg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            '*' | '_' => {
                // Italic
                if !current.is_empty() {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(default_fg),
                    ));
                    current.clear();
                }
                let marker = chars[i];
                i += 1;
                let mut italic = String::new();
                while i < chars.len() && chars[i] != marker {
                    italic.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                spans.push(Span::styled(
                    italic,
                    Style::default()
                        .fg(default_fg)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
            c => {
                current.push(c);
                i += 1;
            }
        }
    }

    if !current.is_empty() {
        spans.push(Span::styled(
            current,
            Style::default().fg(default_fg),
        ));
    }

    spans
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_plain_text_returns_one_span() {
        let spans = render_inline_markdown("hello world", &ChatRole::Assistant);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello world");
    }

    #[test]
    fn inline_bold_produces_bold_span() {
        let spans = render_inline_markdown("**bold**", &ChatRole::Assistant);
        assert!(spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn inline_code_produces_cyan_span() {
        let spans = render_inline_markdown("`code`", &ChatRole::Assistant);
        assert!(spans
            .iter()
            .any(|s| s.style.fg == Some(Color::Cyan)));
    }

    #[test]
    fn inline_italic_produces_italic_span() {
        let spans = render_inline_markdown("*italic*", &ChatRole::User);
        assert!(spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::ITALIC)));
    }
}
