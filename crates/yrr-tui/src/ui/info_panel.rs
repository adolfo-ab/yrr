use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::app::{AgentInfo, App, Phase, model_display_name};

use super::theme;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(info) = app.selected_agent_info() else {
        return;
    };

    // Clear the area so graph content doesn't bleed through.
    frame.render_widget(Clear, area);

    let title = format!(" {} ", info.swarm_key);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(
            Style::default()
                .fg(theme::AQUA)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(theme::BORDER));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let spawn_count = info.spawn.as_ref().and_then(|_| {
        if matches!(app.phase, Phase::Running | Phase::Finished) {
            Some(app.spawn_counts.get(&info.swarm_key).copied().unwrap_or(0))
        } else {
            None
        }
    });

    let lines = build_panel_lines(info, spawn_count);
    let text = Text::from(lines);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn build_panel_lines(info: &AgentInfo, spawn_count: Option<u32>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let label = theme::dim();
    let value = theme::value();
    let signal = theme::signal();
    let dim = theme::dim();

    // Runtime.
    lines.push(Line::from(vec![
        Span::styled(" runtime  ", label),
        Span::styled(info.runtime.clone(), value),
    ]));

    // Model.
    if let Some(model) = &info.model {
        lines.push(Line::from(vec![
            Span::styled(" model    ", label),
            Span::styled(model_display_name(model).to_string(), value),
        ]));
    }

    // Replicas.
    if info.replicas > 1 {
        lines.push(Line::from(vec![
            Span::styled(" replicas ", label),
            Span::styled(info.replicas.to_string(), value),
        ]));
    }

    // Lifecycle.
    if let Some(lc) = &info.lifecycle {
        lines.push(Line::from(vec![
            Span::styled(" lifecycle ", label),
            Span::styled(lc.mode.clone(), value),
        ]));
        if let Some(n) = lc.max_activations {
            lines.push(Line::from(vec![
                Span::styled("   max_act ", dim),
                Span::styled(n.to_string(), value),
            ]));
        }
        if let Some(n) = lc.max_turns {
            lines.push(Line::from(vec![
                Span::styled("   turns   ", dim),
                Span::styled(n.to_string(), value),
            ]));
        }
        if let Some(t) = &lc.idle_timeout {
            lines.push(Line::from(vec![
                Span::styled("   idle    ", dim),
                Span::styled(t.clone(), value),
            ]));
        }
        if let Some(t) = &lc.max_uptime {
            lines.push(Line::from(vec![
                Span::styled("   uptime  ", dim),
                Span::styled(t.clone(), value),
            ]));
        }
        if let Some(signals) = &lc.die_on {
            lines.push(Line::from(vec![
                Span::styled("   die_on  ", dim),
                Span::styled(signals.join(", "), signal),
            ]));
        }
    }

    // Dispatch.
    if let Some(d) = &info.dispatch {
        lines.push(Line::from(vec![
            Span::styled(" dispatch ", label),
            Span::styled(d.clone(), value),
        ]));
    }

    // Spawn.
    if let Some(spawn) = &info.spawn {
        let spawn_text = if let Some(count) = spawn_count {
            format!("on {} (max {}, active {})", spawn.on, spawn.max, count)
        } else {
            format!("on {} (max {})", spawn.on, spawn.max)
        };
        lines.push(Line::from(vec![
            Span::styled(" spawn    ", label),
            Span::styled(spawn_text, value),
        ]));
    }

    // Steer.
    if let Some(steer) = &info.steer {
        let steer_style = Style::default().fg(theme::NODE_STEER);
        lines.push(Line::from(vec![
            Span::styled(" steer    ", label),
            Span::styled(steer.clone(), steer_style),
        ]));
    }

    // Separator.
    lines.push(Line::from(""));

    // Subscribe.
    if !info.subscribe.is_empty() {
        lines.push(Line::from(Span::styled(" subscribe", label)));
        for (name, _desc) in &info.subscribe {
            lines.push(Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(name.clone(), signal),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Publish.
    if !info.publish.is_empty() {
        lines.push(Line::from(Span::styled(" publish", label)));
        for (name, _desc) in &info.publish {
            lines.push(Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(name.clone(), signal),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Queryable.
    if !info.queryable.is_empty() {
        lines.push(Line::from(Span::styled(" queryable", label)));
        for (name, _desc) in &info.queryable {
            lines.push(Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(name.clone(), signal),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Query.
    if !info.query.is_empty() {
        lines.push(Line::from(Span::styled(" query", label)));
        for (name, _desc) in &info.query {
            lines.push(Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(name.clone(), signal),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Description (truncated).
    if let Some(desc) = &info.description {
        lines.push(Line::from(Span::styled(" description", label)));
        // Show first ~3 lines of description.
        for (i, line) in desc.lines().enumerate() {
            if i >= 3 {
                lines.push(Line::from(Span::styled("  ...", dim)));
                break;
            }
            lines.push(Line::from(Span::styled(format!("  {line}"), value)));
        }
    }

    lines
}
