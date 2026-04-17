use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::PromptInput;

use super::theme;

pub fn render(frame: &mut Frame, input: &PromptInput, area: Rect) {
    // Center a dialog box.
    let width = (area.width - 4).min(80);
    let height = 5;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Prompt message ")
        .title_style(
            Style::default()
                .fg(theme::ORANGE)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // Input line.
    let inner_w = inner.width as usize;
    let text = &input.text;

    // Scroll the visible window so the cursor is always on screen.
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
        let hint =
            Paragraph::new("[Enter] run  [Esc] cancel").style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(hint, hint_area);
    }

    // Place cursor.
    let cursor_x = inner.x + (input.cursor - scroll) as u16;
    frame.set_cursor_position(Position::new(cursor_x, inner.y));
}
