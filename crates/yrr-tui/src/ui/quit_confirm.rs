use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use super::theme;

pub fn render(frame: &mut Frame, area: Rect) {
    let width = 36;
    let height = 5;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Quit? ")
        .title_style(
            Style::default()
                .fg(theme::RED)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let prompt = Paragraph::new("Quit yrr?").style(Style::default().fg(theme::FG));
    frame.render_widget(prompt, inner);

    if inner.height > 1 {
        let hint_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
        let hint = Paragraph::new("[y/Enter] quit  [any] cancel")
            .style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(hint, hint_area);
    }
}
