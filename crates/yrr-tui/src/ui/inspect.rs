use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{model_display_name, AgentInfo, App, Phase, PermissionsInfo};

use super::theme;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(info) = app.selected_agent_info() else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" No agent selected ")
            .border_style(Style::default().fg(theme::BORDER));
        frame.render_widget(block, area);
        return;
    };

    let title = format!(" {} ", info.swarm_key);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(theme::AQUA).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(theme::BORDER_HIGHLIGHT));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let spawn_count = info.spawn.as_ref().and_then(|_| {
        if matches!(app.phase, Phase::Running | Phase::Finished) {
            Some(app.spawn_counts.get(&info.swarm_key).copied().unwrap_or(0))
        } else {
            None
        }
    });

    let lines = build_info_lines(info, spawn_count);

    // Cap and apply scroll offset.
    let total_lines = lines.len();
    let visible = inner.height as usize;
    app.cap_inspect_scroll(total_lines, visible);
    let scroll = app.inspect_scroll;

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));

    frame.render_widget(paragraph, inner);

    // Scroll indicator.
    if total_lines > visible {
        let indicator = format!(" {}/{} ", scroll + 1, total_lines.saturating_sub(visible) + 1);
        let indicator_area = Rect::new(
            area.x + area.width.saturating_sub(indicator.len() as u16 + 2),
            area.y,
            indicator.len() as u16,
            1,
        );
        let indicator_widget = Paragraph::new(indicator)
            .style(Style::default().fg(theme::FG_DARK));
        frame.render_widget(indicator_widget, indicator_area);
    }
}

fn build_info_lines(info: &AgentInfo, spawn_count: Option<u32>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let label_style = theme::label();
    let value_style = theme::value();
    let dim_style = theme::dim();
    let signal_style = theme::signal();

    // Name.
    lines.push(Line::from(vec![
        Span::styled("  Name:        ", label_style),
        Span::styled(info.name.clone(), value_style),
    ]));

    // Runtime.
    lines.push(Line::from(vec![
        Span::styled("  Runtime:     ", label_style),
        Span::styled(info.runtime.clone(), value_style),
    ]));

    // Model.
    if let Some(model) = &info.model {
        lines.push(Line::from(vec![
            Span::styled("  Model:       ", label_style),
            Span::styled(model_display_name(model).to_string(), value_style),
        ]));
    }

    // Replicas.
    if info.replicas > 1 {
        lines.push(Line::from(vec![
            Span::styled("  Replicas:    ", label_style),
            Span::styled(info.replicas.to_string(), value_style),
        ]));
    }

    // Description.
    if let Some(desc) = &info.description {
        lines.push(Line::from(vec![
            Span::styled("  Description: ", label_style),
            Span::styled(desc.clone(), value_style),
        ]));
    }

    // Source file.
    if let Some(path) = &info.source_path {
        lines.push(Line::from(vec![
            Span::styled("  Source:      ", label_style),
            Span::styled(path.display().to_string(), dim_style),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  Source:      ", label_style),
            Span::styled("(inline)", dim_style),
        ]));
    }

    lines.push(Line::from(""));

    // Subscribe signals.
    if !info.subscribe.is_empty() {
        lines.push(Line::from(Span::styled("  Subscribe:", label_style)));
        for (name, desc) in &info.subscribe {
            let mut spans = vec![
                Span::styled("    - ", dim_style),
                Span::styled(name.clone(), signal_style),
            ];
            if let Some(d) = desc {
                spans.push(Span::styled(format!("  ({d})"), dim_style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    // Publish signals.
    if !info.publish.is_empty() {
        lines.push(Line::from(Span::styled("  Publish:", label_style)));
        for (name, desc) in &info.publish {
            let mut spans = vec![
                Span::styled("    - ", dim_style),
                Span::styled(name.clone(), signal_style),
            ];
            if let Some(d) = desc {
                spans.push(Span::styled(format!("  ({d})"), dim_style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    // Queryable.
    if !info.queryable.is_empty() {
        lines.push(Line::from(Span::styled("  Queryable:", label_style)));
        for (name, desc) in &info.queryable {
            let mut spans = vec![
                Span::styled("    - ", dim_style),
                Span::styled(name.clone(), signal_style),
            ];
            if let Some(d) = desc {
                spans.push(Span::styled(format!("  ({d})"), dim_style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    // Query.
    if !info.query.is_empty() {
        lines.push(Line::from(Span::styled("  Query:", label_style)));
        for (name, desc) in &info.query {
            let mut spans = vec![
                Span::styled("    - ", dim_style),
                Span::styled(name.clone(), signal_style),
            ];
            if let Some(d) = desc {
                spans.push(Span::styled(format!("  ({d})"), dim_style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    // Lifecycle.
    if let Some(lc) = &info.lifecycle {
        lines.push(Line::from(vec![
            Span::styled("  Lifecycle:   ", label_style),
            Span::styled(lc.mode.clone(), value_style),
        ]));
        if let Some(n) = lc.max_activations {
            lines.push(Line::from(vec![
                Span::styled("    max_activations: ", dim_style),
                Span::styled(n.to_string(), value_style),
            ]));
        }
        if let Some(n) = lc.max_turns {
            lines.push(Line::from(vec![
                Span::styled("    max_turns:       ", dim_style),
                Span::styled(n.to_string(), value_style),
            ]));
        }
        if let Some(t) = &lc.idle_timeout {
            lines.push(Line::from(vec![
                Span::styled("    idle_timeout:    ", dim_style),
                Span::styled(t.clone(), value_style),
            ]));
        }
        if let Some(t) = &lc.max_uptime {
            lines.push(Line::from(vec![
                Span::styled("    max_uptime:      ", dim_style),
                Span::styled(t.clone(), value_style),
            ]));
        }
        if let Some(signals) = &lc.die_on {
            lines.push(Line::from(vec![
                Span::styled("    die_on:          ", dim_style),
                Span::styled(signals.join(", "), signal_style),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Dispatch.
    if let Some(d) = &info.dispatch {
        lines.push(Line::from(vec![
            Span::styled("  Dispatch:    ", label_style),
            Span::styled(d.clone(), value_style),
        ]));
        lines.push(Line::from(""));
    }

    // Spawn.
    if let Some(spawn) = &info.spawn {
        lines.push(Line::from(vec![
            Span::styled("  Spawn:       ", label_style),
            Span::styled(format!("on {}", spawn.on), signal_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    max:             ", dim_style),
            Span::styled(spawn.max.to_string(), value_style),
        ]));
        if let Some(count) = spawn_count {
            lines.push(Line::from(vec![
                Span::styled("    active:          ", dim_style),
                Span::styled(count.to_string(), value_style),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Permissions.
    if let Some(perms) = &info.permissions {
        lines.push(Line::from(Span::styled("  Permissions:", label_style)));
        render_permissions(&mut lines, perms, dim_style, value_style, signal_style);
        lines.push(Line::from(""));
    }

    // Context.
    if let Some(ctx) = &info.context {
        lines.push(Line::from(vec![
            Span::styled("  Context:     ", label_style),
            Span::styled(ctx.clone(), value_style),
        ]));
        lines.push(Line::from(""));
    }

    // Steer.
    if let Some(steer) = &info.steer {
        let steer_style = Style::default().fg(theme::NODE_STEER);
        lines.push(Line::from(vec![
            Span::styled("  Steer:       ", label_style),
            Span::styled(steer.clone(), steer_style),
        ]));
        lines.push(Line::from(""));
    }

    // Prompt.
    lines.push(Line::from(Span::styled("  Prompt:", label_style)));
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────",
        dim_style,
    )));
    for line in info.prompt.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            value_style,
        )));
    }
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────",
        dim_style,
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] back  [e] open in editor  [r] run swarm",
        dim_style,
    )));

    lines
}

fn render_permissions(
    lines: &mut Vec<Line<'static>>,
    perms: &PermissionsInfo,
    dim_style: Style,
    value_style: Style,
    signal_style: Style,
) {
    if !perms.tools_allow.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("    tools.allow: ", dim_style),
            Span::styled(perms.tools_allow.join(", "), signal_style),
        ]));
    }
    if !perms.tools_deny.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("    tools.deny:  ", dim_style),
            Span::styled(perms.tools_deny.join(", "), Style::default().fg(theme::RED)),
        ]));
    }
    if !perms.paths_allow.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("    paths.allow: ", dim_style),
            Span::styled(perms.paths_allow.join(", "), signal_style),
        ]));
    }
    if !perms.paths_deny.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("    paths.deny:  ", dim_style),
            Span::styled(perms.paths_deny.join(", "), Style::default().fg(theme::RED)),
        ]));
    }
    if let Some(network) = perms.network {
        let (text, style) = if network {
            ("true", value_style)
        } else {
            ("false", Style::default().fg(theme::RED))
        };
        lines.push(Line::from(vec![
            Span::styled("    network:     ", dim_style),
            Span::styled(text, style),
        ]));
    }
}
