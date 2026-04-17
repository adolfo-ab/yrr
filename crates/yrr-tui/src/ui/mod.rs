mod graph;
mod info_panel;
mod inspect;
mod logs;
mod prompt_input;
mod quit_confirm;
mod status_bar;
mod steer_input;
pub mod theme;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Tabs};

use crate::app::{App, GraphView, Phase, Tab};

/// Render the full UI.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    match app.phase {
        Phase::Preview => render_preview(frame, app, area),
        Phase::Running | Phase::Finished => render_running(frame, app, area),
    }
}

/// Preview layout: graph (or inspect) + status bar. No tab bar needed.
fn render_preview(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    match app.graph_view {
        GraphView::Overview => {
            if app.selected_agent_info().is_some() {
                let hsplit = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(0),     // Graph
                        Constraint::Length(36), // Info panel
                    ])
                    .split(chunks[0]);
                graph::render(frame, app, hsplit[0]);
                info_panel::render(frame, app, hsplit[1]);
            } else {
                graph::render(frame, app, chunks[0]);
            }
        }
        GraphView::Inspect => inspect::render(frame, app, chunks[0]),
    }

    status_bar::render(frame, app, chunks[1]);

    // Prompt input overlay (on top of everything).
    if let Some(input) = &app.prompt_input {
        prompt_input::render(frame, input, area);
    }

    if app.confirm_quit {
        quit_confirm::render(frame, area);
    }
}

/// Running/Finished layout: tab bar + content + status bar.
fn render_running(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab bar
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Tab bar.
    let tab_titles = vec!["Graph [1]", "Logs [2]"];
    let selected = match app.tab {
        Tab::Graph => 0,
        Tab::Logs => 1,
    };
    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .title(" yrr ")
                .title_style(Style::default().fg(theme::ORANGE))
                .border_style(Style::default().fg(theme::BORDER)),
        )
        .select(selected)
        .style(Style::default().fg(theme::FG_DARK))
        .highlight_style(
            Style::default()
                .fg(theme::AQUA)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Content.
    match app.tab {
        Tab::Graph => match app.graph_view {
            GraphView::Overview => {
                if app.selected_agent_info().is_some() {
                    let hsplit = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(0), Constraint::Length(36)])
                        .split(chunks[1]);
                    graph::render(frame, app, hsplit[0]);
                    info_panel::render(frame, app, hsplit[1]);
                } else {
                    graph::render(frame, app, chunks[1]);
                }
            }
            GraphView::Inspect => inspect::render(frame, app, chunks[1]),
        },
        Tab::Logs => logs::render(frame, app, chunks[1]),
    }

    // Status bar.
    status_bar::render(frame, app, chunks[2]);

    // Steer input overlays (on top of everything).
    if app.steer_selecting_agent {
        steer_input::render_selector(frame, app, area);
    } else if let Some(input) = &app.steer_input {
        steer_input::render(frame, input, area);
    }

    if app.confirm_quit {
        quit_confirm::render(frame, area);
    }
}
