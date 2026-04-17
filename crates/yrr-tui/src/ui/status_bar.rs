use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{App, GraphView, Phase, Tab};

use super::theme;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let (text, style) = match app.phase {
        Phase::Preview => {
            let keys = match app.graph_view {
                GraphView::Overview => {
                    "[←↑↓→] navigate  [Shift+←→] scroll  [Enter] inspect  [e] edit  [r] run  [q] quit"
                }
                GraphView::Inspect => "[↑↓] scroll  [Esc] back  [e] edit  [r] run  [q] back",
            };
            let selected = app.selected_node_id().unwrap_or("-");
            let text = format!(
                " {} | agents: {} | selected: {} | {keys}",
                app.swarm_name,
                app.agent_info.len(),
                selected,
            );
            (
                text,
                Style::default()
                    .bg(theme::STATUS_PREVIEW_BG)
                    .fg(theme::STATUS_PREVIEW_FG),
            )
        }
        Phase::Running => {
            let elapsed = app.elapsed_secs();
            let mins = elapsed / 60;
            let secs = elapsed % 60;
            let keys = match app.tab {
                Tab::Graph => match app.graph_view {
                    GraphView::Overview => {
                        if app.steerable_agents.is_empty() {
                            "[←↑↓→] navigate  [Shift+←→] scroll  [Enter] inspect  [Tab] logs  [q] quit"
                        } else {
                            "[←↑↓→] navigate  [Shift+←→] scroll  [Enter] inspect  [s] steer  [Tab] logs  [q] quit"
                        }
                    }
                    GraphView::Inspect => "[↑↓] scroll  [Esc] back  [Tab] logs  [q] quit",
                },
                Tab::Logs => "[↑↓] scroll  [g/G] top/bottom  [Tab] graph  [q] quit",
            };
            let text = format!(
                " {} | {}m {:02}s | agents: {} | signals: {} | {keys}",
                app.swarm_name, mins, secs, app.active_agents, app.signal_count,
            );
            (
                text,
                Style::default()
                    .bg(theme::STATUS_RUNNING_BG)
                    .fg(theme::STATUS_RUNNING_FG),
            )
        }
        Phase::Finished => {
            let elapsed = app.elapsed_secs();
            let mins = elapsed / 60;
            let secs = elapsed % 60;
            let status = app.outcome_text.as_deref().unwrap_or("finished");
            let text = format!(
                " {} | {}m {:02}s | agents: {} | signals: {} | {status} | [q] quit",
                app.swarm_name, mins, secs, app.active_agents, app.signal_count,
            );
            (
                text,
                Style::default()
                    .bg(theme::STATUS_FINISHED_BG)
                    .fg(theme::STATUS_FINISHED_FG),
            )
        }
    };

    let bar = Paragraph::new(text).style(style);
    frame.render_widget(bar, area);
}
