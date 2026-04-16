use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::{App, SteerInput};

use super::theme;

/// Render the agent selector overlay (when multiple steerable agents exist).
pub fn render_selector(frame: &mut Frame, app: &App, area: Rect) {
    let count = app.steerable_agents.len();
    let width = (area.width - 4).min(50);
    let height = (count as u16 + 4).min(area.height - 2);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Steer agent ")
        .title_style(
            Style::default()
                .fg(theme::NODE_STEER)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    for (i, name) in app.steerable_agents.iter().enumerate() {
        if i as u16 >= inner.height.saturating_sub(1) {
            break;
        }
        let style = if i == app.steer_agent_selector {
            Style::default()
                .fg(theme::NODE_STEER)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::FG_DIM)
        };
        let prefix = if i == app.steer_agent_selector {
            "▸ "
        } else {
            "  "
        };
        let line = Paragraph::new(format!("{prefix}{name}")).style(style);
        let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        frame.render_widget(line, row);
    }

    // Hint.
    let hint_y = inner.y + inner.height.saturating_sub(1);
    let hint = Paragraph::new("[Enter] select  [Esc] cancel")
        .style(Style::default().fg(theme::FG_DARK));
    frame.render_widget(hint, Rect::new(inner.x, hint_y, inner.width, 1));
}

/// Render the steer text input overlay.
pub fn render(frame: &mut Frame, input: &SteerInput, area: Rect) {
    let width = (area.width - 4).min(80);
    let height = 5;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let title = format!(" Steer → {} ", input.target_agent);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(
            Style::default()
                .fg(theme::NODE_STEER)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Input line.
    let inner_w = inner.width as usize;
    let text = &input.text;

    let scroll = if input.cursor >= inner_w {
        input.cursor - inner_w + 1
    } else {
        0
    };

    let visible: String = text.chars().skip(scroll).take(inner_w).collect();
    let input_line = Paragraph::new(visible).style(Style::default().fg(theme::FG));
    frame.render_widget(input_line, inner);

    // Hint line.
    if inner.height > 1 {
        let hint_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
        let hint = Paragraph::new("[Enter] send  [Esc] cancel")
            .style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(hint, hint_area);
    }

    // Place cursor.
    let cursor_x = inner.x + (input.cursor - scroll) as u16;
    frame.set_cursor_position(Position::new(cursor_x, inner.y));
}
