use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::tui::app::LayoutMode;

/// Areas returned by `SplitPane::split`.
#[derive(Debug, Clone, Copy)]
pub struct TuiAreas {
    pub chat: Rect,
    pub tool_panel: Option<Rect>,
    pub input: Rect,
    pub status: Rect,
}

/// Adaptive layout: stacked (<80 cols) or side-by-side (≥120 cols).
pub struct SplitPane;

impl SplitPane {
    /// Compute layout rectangles from the terminal frame area.
    pub fn split(area: Rect, mode: LayoutMode, tool_panel_visible: bool) -> TuiAreas {
        match mode {
            LayoutMode::Stacked => Self::stacked(area),
            LayoutMode::SideBySide => Self::side_by_side(area, tool_panel_visible),
        }
    }

    fn stacked(area: Rect) -> TuiAreas {
        // ┌───────────────┐
        // │    chat       │ flex
        // ├───────────────┤
        // │    input      │ 3 rows
        // ├───────────────┤
        // │    status     │ 1 row
        // └───────────────┘
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        TuiAreas {
            chat: vertical[0],
            tool_panel: None,
            input: vertical[1],
            status: vertical[2],
        }
    }

    fn side_by_side(area: Rect, tool_panel_visible: bool) -> TuiAreas {
        // ┌────────────────┬──────────────┐
        // │     chat       │ tool panel   │ flex
        // ├────────────────┴──────────────┤
        // │           input               │ 3 rows
        // ├───────────────────────────────┤
        // │           status              │ 1 row
        // └───────────────────────────────┘
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        let main_area = vertical[0];

        let (chat, tool_panel) = if tool_panel_visible {
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(main_area);
            (horizontal[0], Some(horizontal[1]))
        } else {
            (main_area, None)
        };

        TuiAreas {
            chat,
            tool_panel,
            input: vertical[1],
            status: vertical[2],
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rect(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn stacked_has_no_tool_panel() {
        let areas = SplitPane::split(rect(79, 40), LayoutMode::Stacked, true);
        assert!(areas.tool_panel.is_none());
    }

    #[test]
    fn stacked_status_height_is_one() {
        let areas = SplitPane::split(rect(79, 40), LayoutMode::Stacked, false);
        assert_eq!(areas.status.height, 1);
    }

    #[test]
    fn stacked_input_height_is_three() {
        let areas = SplitPane::split(rect(79, 40), LayoutMode::Stacked, false);
        assert_eq!(areas.input.height, 3);
    }

    #[test]
    fn side_by_side_with_panel_splits_main() {
        let areas = SplitPane::split(rect(120, 40), LayoutMode::SideBySide, true);
        assert!(areas.tool_panel.is_some());
        // Chat + tool panel widths should sum to full width
        let chat_w = areas.chat.width;
        let panel_w = areas.tool_panel.expect("panel").width;
        assert_eq!(chat_w + panel_w, 120);
    }

    #[test]
    fn side_by_side_no_panel_when_hidden() {
        let areas = SplitPane::split(rect(120, 40), LayoutMode::SideBySide, false);
        assert!(areas.tool_panel.is_none());
        assert_eq!(areas.chat.width, 120);
    }
}
