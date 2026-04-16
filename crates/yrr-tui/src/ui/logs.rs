use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::app::App;

use super::theme;

/// Truncate or pad a string to exactly `width` characters.
fn fit_column(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count > width {
        let truncated: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{truncated}…")
    } else {
        format!("{:<width$}", s)
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.logs.detail_open && app.logs.selected_payload().is_some() {
        render_detail_pane(frame, app, area);
    } else {
        render_log_list(frame, app, area);
    }
}

fn render_log_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let hint = if app.logs.detail_open {
        ""
    } else {
        "  [Enter] inspect payload"
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Logs ")
        .title_style(Style::default().fg(theme::FG_DIM))
        .border_style(Style::default().fg(theme::BORDER));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Draw column headers.
    let header_style = Style::default().fg(theme::FG_DARK).add_modifier(Modifier::BOLD);
    let header = Line::from(vec![
        Span::styled(" ", header_style),
        Span::styled("TIME        ", header_style),
        Span::styled("AGENT          ", header_style),
        Span::styled("EVENT            ", header_style),
        Span::styled("DETAIL", header_style),
    ]);
    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(Paragraph::new(header), header_area);

    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));

    if app.logs.entries.is_empty() {
        let empty = Paragraph::new("  Waiting for events...")
            .style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(empty, list_area);
        return;
    }

    let visible_height = list_area.height as usize;
    let total = app.logs.entries.len();
    let cursor = app.logs.cursor.min(total.saturating_sub(1));

    // Ensure cursor is within view.
    if app.logs.auto_scroll {
        app.logs.scroll_offset = total.saturating_sub(visible_height);
    } else {
        // Adjust scroll_offset to keep cursor visible.
        if cursor < app.logs.scroll_offset {
            app.logs.scroll_offset = cursor;
        }
        if cursor >= app.logs.scroll_offset + visible_height {
            app.logs.scroll_offset = cursor.saturating_sub(visible_height.saturating_sub(1));
        }
    }

    let start = app.logs.scroll_offset.min(total.saturating_sub(visible_height));
    let end = (start + visible_height).min(total);

    let time_style = Style::default().fg(theme::FG_DARK);
    let agent_style = Style::default().fg(theme::FG_DIM);

    let items: Vec<ListItem> = app
        .logs
        .entries
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
        .map(|(idx, entry)| {
            let time_str = entry.timestamp.format("%H:%M:%S").to_string();
            let agent_col = fit_column(&entry.agent_name, 14);
            let event_col = fit_column(&entry.event_type, 16);

            let event_style = match entry.event_type.as_str() {
                "signal_emitted" => Style::default().fg(theme::GREEN),
                "signal_received" => Style::default().fg(theme::AQUA),
                "activation" => Style::default().fg(theme::YELLOW),
                "error" => Style::default().fg(theme::RED),
                "spawned" => Style::default().fg(theme::BLUE),
                "stopped" => Style::default().fg(theme::FG_DARK),
                "done" => Style::default().fg(theme::VIOLET).add_modifier(Modifier::BOLD),
                "timeout" => Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                "injected" => Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
                "dispatched" => Style::default().fg(theme::TEAL),
                "queued" => Style::default().fg(theme::ASH),
                _ => Style::default().fg(theme::FG),
            };

            let detail_style = Style::default().fg(theme::FG_DIM);

            // Cursor indicator.
            let is_selected = idx == cursor;
            let prefix = if is_selected { ">" } else { " " };
            let prefix_style = if is_selected {
                Style::default()
                    .fg(theme::AQUA)
                    .add_modifier(Modifier::BOLD)
            } else {
                time_style
            };

            // Payload marker for entries that have inspectable content.
            let marker = if entry.payload.is_some() { " " } else { "" };

            let mut item = ListItem::new(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(format!("[{time_str}] "), time_style),
                Span::styled(format!("{agent_col} "), agent_style),
                Span::styled(format!("{event_col} "), event_style),
                Span::styled(&entry.detail, detail_style),
                Span::styled(marker, Style::default().fg(theme::FG_DARK)),
            ]));
            if is_selected {
                item = item.style(Style::default().bg(theme::BG_HIGHLIGHT));
            }
            item
        })
        .collect();

    // Render hint at bottom-right of the block border.
    if !hint.is_empty() {
        let hint_area = Rect::new(
            area.x + area.width.saturating_sub(hint.len() as u16 + 1),
            area.y + area.height.saturating_sub(1),
            hint.len() as u16,
            1,
        );
        let hint_widget =
            Paragraph::new(hint).style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(hint_widget, hint_area);
    }

    let list = List::new(items);
    frame.render_widget(list, list_area);
}

fn render_detail_pane(frame: &mut Frame, app: &mut App, area: Rect) {
    let payload = match app.logs.selected_payload() {
        Some(p) => p.to_string(),
        None => return,
    };

    let entry = &app.logs.entries[app.logs.cursor];
    let title = format!(
        " {} {} ",
        entry.agent_name, entry.event_type
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(theme::AQUA)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Cap detail_scroll — account for wrapped lines, not just source lines.
    let visible = inner.height as usize;
    let w = inner.width as usize;
    let visual_lines: usize = payload
        .split('\n')
        .map(|line| {
            let len = line.chars().count();
            if len == 0 {
                1
            } else {
                (len + w.max(1) - 1) / w.max(1)
            }
        })
        .sum();
    let max_scroll = visual_lines.saturating_sub(visible);
    if app.logs.detail_scroll > max_scroll {
        app.logs.detail_scroll = max_scroll;
    }

    let text = Text::from(payload.clone());
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((app.logs.detail_scroll as u16, 0))
        .style(Style::default().fg(theme::FG));

    frame.render_widget(paragraph, inner);

    // Footer hint.
    let hint = " [Esc] close  [j/k] scroll ";
    let hint_area = Rect::new(
        area.x + area.width.saturating_sub(hint.len() as u16 + 1),
        area.y + area.height.saturating_sub(1),
        hint.len() as u16,
        1,
    );
    let hint_widget = Paragraph::new(hint).style(Style::default().fg(theme::FG_DARK));
    frame.render_widget(hint_widget, hint_area);
}
