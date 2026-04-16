use std::collections::{HashMap, HashSet};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, SPINNER_CHARS};
use crate::graph::layout::{self, NODE_HEIGHT, NODE_WIDTH};
use crate::graph::model::{GraphEdge, GraphState, NodeStatus};

use super::theme;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Swarm Graph ")
        .title_style(Style::default().fg(theme::FG_DIM))
        .border_style(Style::default().fg(theme::BORDER));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.graph.nodes.is_empty() {
        return;
    }

    // Get graph-space positions and total dimensions.
    let (graph_positions, graph_width, graph_height) = layout::graph_space_positions(&app.graph);

    // Compute the offset to translate graph-space → screen-space.
    // If the graph fits, center it. Otherwise, apply the viewport scroll.
    let offset_x = if graph_width <= inner.width as i32 {
        inner.x as i32 + (inner.width as i32 - graph_width) / 2
    } else {
        inner.x as i32 - app.graph_scroll.0
    };

    let offset_y = if graph_height <= inner.height as i32 {
        inner.y as i32 + (inner.height as i32 - graph_height) / 2
    } else {
        inner.y as i32 - app.graph_scroll.1
    };

    // Convert graph-space → screen-space, keeping only nodes that are at least partially visible.
    let positions: HashMap<String, (u16, u16)> = graph_positions
        .iter()
        .filter_map(|(id, &(gx, gy))| {
            let sx = gx + offset_x;
            let sy = gy + offset_y;
            // Skip if entirely off-screen (negative or past the area).
            if sx + NODE_WIDTH <= inner.x as i32
                || sy + NODE_HEIGHT <= inner.y as i32
                || sx >= (inner.x + inner.width) as i32
                || sy >= (inner.y + inner.height) as i32
            {
                return None;
            }
            // We can only draw if the top-left is non-negative (u16).
            if sx >= 0 && sy >= 0 {
                Some((id.clone(), (sx as u16, sy as u16)))
            } else {
                None
            }
        })
        .collect();

    // Get selected node ID for highlighting (all phases).
    let selected_id = app.selected_node_id().map(|s| s.to_string());

    let buf = frame.buffer_mut();

    let node_layer: HashMap<&str, usize> = app
        .graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.layer))
        .collect();

    // Collect edge labels to draw them AFTER nodes (so they're always visible).
    let mut labels: Vec<(u16, u16, String)> = Vec::new();

    // Group back edges by (from, to) so they render as a single arrow with stacked labels.
    let back_edge_groups: Vec<(String, String, Vec<String>)> = {
        let mut group_map: HashMap<(String, String), Vec<String>> = HashMap::new();
        for edge in &app.graph.edges {
            if edge.is_back_edge {
                group_map
                    .entry((edge.from.clone(), edge.to.clone()))
                    .or_default()
                    .push(edge.signal.clone());
            }
        }
        let mut groups: Vec<_> = group_map.into_iter().map(|((f, t), s)| (f, t, s)).collect();
        groups.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        groups
    };
    let mut back_edge_group_routes: Vec<u16> = vec![0; back_edge_groups.len()];

    // Per-node render heights: scale nodes with multiple right-side connections.
    let nh_default = NODE_HEIGHT as u16;
    let mut stub_counts: HashMap<&str, u16> = HashMap::new();
    for (node_id, _) in &app.graph.output_stubs {
        *stub_counts.entry(node_id.as_str()).or_insert(0) += 1;
    }
    let mut back_entry_counts: HashMap<&str, u16> = HashMap::new();
    for (_, to, _) in &back_edge_groups {
        *back_entry_counts.entry(to.as_str()).or_insert(0) += 1;
    }
    let node_heights: HashMap<&str, u16> = app
        .graph
        .nodes
        .iter()
        .map(|n| {
            let s = stub_counts.get(n.id.as_str()).copied().unwrap_or(0);
            let b = back_entry_counts.get(n.id.as_str()).copied().unwrap_or(0);
            let total = s + b;
            (
                n.id.as_str(),
                if total > 1 { total + 3 } else { nh_default },
            )
        })
        .collect();
    let positions: HashMap<String, (u16, u16)> = positions
        .into_iter()
        .map(|(id, (x, y))| {
            let h = node_heights.get(id.as_str()).copied().unwrap_or(nh_default);
            let y_adj = y.saturating_sub(h.saturating_sub(nh_default) / 2);
            (id, (x, y_adj))
        })
        .collect();

    // Pre-compute edge offsets to avoid overlapping arrows.
    let edge_count = app.graph.edges.len();
    let mut start_offsets: Vec<i16> = vec![0; edge_count];
    let mut end_offsets: Vec<i16> = vec![0; edge_count];
    {
        // Fan out forward edges sharing a source or target node.
        let mut outgoing: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, edge) in app.graph.edges.iter().enumerate() {
            if edge.is_back_edge {
                continue;
            }
            if let (Some(&fl), Some(&tl)) = (
                node_layer.get(edge.from.as_str()),
                node_layer.get(edge.to.as_str()),
            ) {
                if tl > fl + 1 {
                    continue;
                }
            }
            if positions.contains_key(&edge.from) && positions.contains_key(&edge.to) {
                outgoing.entry(edge.from.as_str()).or_default().push(i);
                incoming.entry(edge.to.as_str()).or_default().push(i);
            }
        }
        for indices in outgoing.values() {
            if indices.len() <= 1 {
                continue;
            }
            let mut sorted = indices.clone();
            sorted.sort_by_key(|&i| {
                positions
                    .get(&app.graph.edges[i].to)
                    .map(|p| p.0)
                    .unwrap_or(0)
            });
            let n = sorted.len() as i16;
            for (j, &idx) in sorted.iter().enumerate() {
                start_offsets[idx] = j as i16 * 2 - (n - 1);
            }
        }
        for indices in incoming.values() {
            if indices.len() <= 1 {
                continue;
            }
            let mut sorted = indices.clone();
            sorted.sort_by_key(|&i| {
                positions
                    .get(&app.graph.edges[i].from)
                    .map(|p| p.0)
                    .unwrap_or(0)
            });
            let n = sorted.len() as i16;
            for (j, &idx) in sorted.iter().enumerate() {
                end_offsets[idx] = j as i16 * 2 - (n - 1);
            }
        }

        // Route back edge groups to the right of all visible content.
        let nw_u16 = NODE_WIDTH as u16;
        let max_node_right: u16 = positions
            .values()
            .map(|&(x, _)| x + nw_u16)
            .max()
            .unwrap_or(0);
        let fwd_label_right: u16 = app
            .graph
            .edges
            .iter()
            .filter(|e| !e.is_back_edge && !e.is_query)
            .filter_map(|e| {
                positions
                    .get(&e.from)
                    .map(|&(x, _)| x + nw_u16 / 2 + 2 + e.signal.len() as u16)
            })
            .max()
            .unwrap_or(0);
        let stub_right: u16 = app
            .graph
            .output_stubs
            .iter()
            .filter_map(|(node_id, signal)| {
                positions
                    .get(node_id)
                    .map(|&(x, _)| x + nw_u16 + 4 + signal.len() as u16)
            })
            .max()
            .unwrap_or(0);
        let route_start =
            max_node_right.max(fwd_label_right).max(stub_right) + 3;
        let mut side_route_count: u16 = 0;
        for (i, (_, to, _)) in back_edge_groups.iter().enumerate() {
            if positions.contains_key(to) {
                back_edge_group_routes[i] = route_start + side_route_count * 3;
                side_route_count += 1;
            }
        }

    }

    // Collect skip-layer forward edges (edges that skip over intermediate layers).
    let skip_edges: Vec<usize> = app
        .graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(i, edge)| {
            if edge.is_back_edge {
                return None;
            }
            if let (Some(&fl), Some(&tl)) = (
                node_layer.get(edge.from.as_str()),
                node_layer.get(edge.to.as_str()),
            ) {
                if tl > fl + 1 {
                    return Some(i);
                }
            }
            None
        })
        .collect();

    // Draw forward edges (behind nodes).
    let mut labeled_fwd: HashSet<(String, String)> = HashSet::new();
    let mut fwd_label_count: HashMap<String, u16> = HashMap::new();
    let nw_label = NODE_WIDTH as u16;
    for (i, edge) in app.graph.edges.iter().enumerate() {
        if edge.is_back_edge || edge.is_query || skip_edges.contains(&i) {
            continue;
        }
        if let (Some(&from_pos), Some(&to_pos)) =
            (positions.get(&edge.from), positions.get(&edge.to))
        {
            let from_h = node_heights
                .get(edge.from.as_str())
                .copied()
                .unwrap_or(nh_default);
            draw_edge(
                buf,
                inner,
                from_pos,
                to_pos,
                edge,
                &mut labels,
                start_offsets[i],
                end_offsets[i],
                from_h,
            );
            if !edge.signal.is_empty()
                && from_pos.1 + from_h < to_pos.1
                && labeled_fwd.insert((edge.from.clone(), edge.signal.clone()))
            {
                let count = fwd_label_count.entry(edge.from.clone()).or_insert(0);
                let sx = from_pos.0 + nw_label / 2;
                let sy = from_pos.1 + from_h + *count;
                labels.push((sx + 2, sy, edge.signal.clone()));
                *count += 1;
            }
        }
    }

    // Draw query edges (dashed, bidirectional).
    for (i, edge) in app.graph.edges.iter().enumerate() {
        if !edge.is_query {
            continue;
        }
        if let (Some(&from_pos), Some(&to_pos)) =
            (positions.get(&edge.from), positions.get(&edge.to))
        {
            let from_h = node_heights
                .get(edge.from.as_str())
                .copied()
                .unwrap_or(nh_default);
            draw_query_edge(
                buf,
                inner,
                from_pos,
                to_pos,
                edge,
                &mut labels,
                start_offsets[i],
                end_offsets[i],
                from_h,
            );
        }
    }

    // Draw merged back edges (one arrow per unique from→to pair).
    let mut back_y_counter: HashMap<&str, u16> = HashMap::new();
    for (i, (from, to, signals)) in back_edge_groups.iter().enumerate() {
        let to_pos = match positions.get(to) {
            Some(&pos) => pos,
            None => continue,
        };
        let from_pos = positions.get(from).copied().unwrap_or_else(|| {
            if let Some(&(gx, gy)) = graph_positions.get(from) {
                let sx = (gx + offset_x)
                    .clamp(inner.x as i32, (inner.x + inner.width) as i32 - 1)
                    as u16;
                let sy = (gy + offset_y)
                    .clamp(inner.y as i32, (inner.y + inner.height) as i32 - 1)
                    as u16;
                (sx, sy)
            } else {
                to_pos
            }
        });
        {
            let any_active = app.graph.edges.iter().any(|e| {
                e.is_back_edge
                    && e.from == *from
                    && e.to == *to
                    && GraphState::is_edge_active(e)
            });
            let style = if any_active {
                Style::default()
                    .fg(theme::AQUA)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::RED)
            };
            let from_h = node_heights
                .get(from.as_str())
                .copied()
                .unwrap_or(nh_default);
            let from_stubs = stub_counts.get(from.as_str()).copied().unwrap_or(0);
            let exit_y = if from_h > nh_default {
                from_pos.1 + 1 + from_stubs
            } else {
                from_pos.1 + from_h / 2
            };
            let to_h = node_heights
                .get(to.as_str())
                .copied()
                .unwrap_or(nh_default);
            let to_stubs = stub_counts.get(to.as_str()).copied().unwrap_or(0);
            let counter = back_y_counter.entry(to.as_str()).or_insert(0);
            let entry_y = if to_h > nh_default {
                to_pos.1 + 1 + to_stubs + 1 + *counter
            } else {
                to_pos.1 + to_h / 2
            };
            *counter += 1;
            draw_back_edge(
                buf,
                inner,
                from_pos,
                to_pos,
                signals,
                style,
                &mut labels,
                back_edge_group_routes[i],
                exit_y,
                entry_y,
            );
        }
    }

    // Draw skip-layer forward edges routed to the right side.
    if !skip_edges.is_empty() {
        let nw = NODE_WIDTH as u16;
        let max_node_right: u16 = positions
            .values()
            .map(|&(x, _)| x + nw)
            .max()
            .unwrap_or(0);
        let base_route = back_edge_group_routes
            .iter()
            .copied()
            .max()
            .map(|r| r + 3)
            .unwrap_or(max_node_right + 3);
        let mut skip_count: u16 = 0;
        for &i in &skip_edges {
            let edge = &app.graph.edges[i];
            if let (Some(&from_pos), Some(&to_pos)) =
                (positions.get(&edge.from), positions.get(&edge.to))
            {
                let style = if GraphState::is_edge_active(edge) {
                    Style::default()
                        .fg(theme::AQUA)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::FG_DARK)
                };
                let route_x = base_route + skip_count * 3;
                let from_h = node_heights
                    .get(edge.from.as_str())
                    .copied()
                    .unwrap_or(nh_default);
                draw_skip_forward_edge(
                    buf,
                    inner,
                    from_pos,
                    to_pos,
                    &edge.signal,
                    style,
                    &mut labels,
                    route_x,
                    from_h,
                );
                skip_count += 1;
            }
        }
    }

    // Draw output/done stubs as horizontal arrows above the content row.
    if !app.graph.output_stubs.is_empty() {
        let nw = NODE_WIDTH as u16;
        let stub_style = Style::default().fg(theme::GREEN);
        let mut stub_index: HashMap<String, u16> = HashMap::new();
        for (node_id, signal) in &app.graph.output_stubs {
            if let Some(&from_pos) = positions.get(node_id) {
                let idx = stub_index.entry(node_id.clone()).or_insert(0);
                let x = from_pos.0 + nw;
                let y = from_pos.1 + 1 + *idx;
                set_char_clipped(buf, inner, x, y, '─', stub_style);
                set_char_clipped(buf, inner, x + 1, y, '─', stub_style);
                set_char_clipped(buf, inner, x + 2, y, '▶', stub_style);
                labels.push((x + 4, y, signal.clone()));
                *idx += 1;
            }
        }
    }

    // Draw nodes on top.
    let spinner = SPINNER_CHARS[app.spinner_frame % SPINNER_CHARS.len()];
    for node in &app.graph.nodes {
        if let Some(&(x, y)) = positions.get(&node.id) {
            let is_selected = selected_id.as_deref() == Some(&node.id);
            let h = node_heights
                .get(node.id.as_str())
                .copied()
                .unwrap_or(nh_default);
            let stubs = stub_counts.get(node.id.as_str()).copied().unwrap_or(0);
            draw_node(
                buf,
                inner,
                x,
                y,
                node.status,
                &node.label,
                node.virtual_node,
                is_selected,
                node.replicas,
                spinner,
                h,
                stubs,
            );
        }
    }

    // Draw steer indicators for steerable nodes.
    for node in &app.graph.nodes {
        if node.steerable {
            if let Some(&(x, y)) = positions.get(&node.id) {
                let h = node_heights
                    .get(node.id.as_str())
                    .copied()
                    .unwrap_or(nh_default);
                let stubs = stub_counts.get(node.id.as_str()).copied().unwrap_or(0);
                let content_y = if h > nh_default { y + 1 + stubs } else { y + 1 };
                let nw = NODE_WIDTH as u16;
                let left_bound = app
                    .graph
                    .nodes
                    .iter()
                    .filter(|n| n.layer == node.layer && n.id != node.id)
                    .filter_map(|n| positions.get(&n.id))
                    .filter(|&&(px, _)| px + nw <= x)
                    .map(|&(px, _)| px + nw)
                    .max()
                    .unwrap_or(inner.x);
                draw_steer_indicator(buf, inner, x, content_y, left_bound);
            }
        }
    }

    // Draw labels on top of everything so they're always readable.
    let label_style = Style::default().fg(theme::FG_DIM);
    for (lx, ly, text) in &labels {
        set_string_clipped(buf, inner, *lx, *ly, text, label_style);
    }

    // Draw status legend in the bottom-left corner.
    draw_legend(buf, inner, spinner);

    // Draw swarm description box at the top-left.
    if let Some(desc) = &app.swarm_description {
        if !desc.trim().is_empty() {
            draw_description_box(frame, inner, desc);
        }
    }
}

/// Draw a single node box, with optional selection highlight and replica count.
fn draw_node(
    buf: &mut Buffer,
    clip: Rect,
    x: u16,
    y: u16,
    status: NodeStatus,
    label: &str,
    virtual_node: bool,
    selected: bool,
    replicas: u32,
    spinner: char,
    height: u16,
    stubs: u16,
) {
    let w = NODE_WIDTH as u16;
    let h = height;

    if x + w > clip.x + clip.width || y + h > clip.y + clip.height {
        return;
    }

    let border_style = if selected {
        Style::default()
            .fg(theme::NODE_SELECTED)
            .add_modifier(Modifier::BOLD)
    } else {
        match status {
            NodeStatus::Idle => Style::default().fg(theme::NODE_IDLE),
            NodeStatus::Busy => Style::default().fg(theme::NODE_BUSY),
            NodeStatus::Stopped => Style::default().fg(theme::NODE_STOPPED),
            NodeStatus::Pending => Style::default().fg(theme::NODE_PENDING),
        }
    };

    let status_char: String = match status {
        NodeStatus::Idle => "●".into(),
        NodeStatus::Busy => spinner.to_string(),
        NodeStatus::Stopped => "○".into(),
        NodeStatus::Pending => "◌".into(),
    };

    let wu = w as usize;
    let inner_w = wu - 2;

    let (tl, horiz, tr, ml, mr, bl, br) = if selected {
        ("╔", "═", "╗", "║", "║", "╚", "╝")
    } else {
        ("╭", "─", "╮", "│", "│", "╰", "╯")
    };

    let top = format!("{tl}{}{tr}", horiz.repeat(wu - 2));
    set_string_clipped(buf, clip, x, y, &top, border_style);

    let content_row = if h > 3 { 1 + stubs } else { 1 };
    for row in 1..h - 1 {
        if row == content_row {
            if virtual_node {
                let padded = format!("{ml}  {:<width$} {mr}", label, width = inner_w - 3);
                set_string_clipped(buf, clip, x, y + row, &padded, border_style);
            } else {
                let content_w = inner_w - 2;
                let replica_tag = if replicas > 1 {
                    format!(" ×{replicas}")
                } else {
                    String::new()
                };
                let tag_len = replica_tag.len();
                let max_name = content_w.saturating_sub(2 + tag_len);
                let truncated = if label.len() > max_name {
                    format!("{}…", &label[..max_name.saturating_sub(1)])
                } else {
                    label.to_string()
                };
                let prefix = format!("{status_char} {truncated}");
                let padding = content_w.saturating_sub(prefix.len() + tag_len);
                let content = format!("{prefix}{}{replica_tag}", " ".repeat(padding));
                let padded = format!("{ml} {content} {mr}");
                set_string_clipped(buf, clip, x, y + row, &padded, border_style);
            }
        } else {
            let empty = format!("{ml}{}{mr}", " ".repeat(inner_w));
            set_string_clipped(buf, clip, x, y + row, &empty, border_style);
        }
    }

    let bottom = format!("{bl}{}{br}", horiz.repeat(wu - 2));
    set_string_clipped(buf, clip, x, y + h - 1, &bottom, border_style);
}

/// Draw a steer indicator: an arrow from the left side pointing into the node.
/// Draws at the content row (y+1) to the left of the node box.
/// `left_bound` is the rightmost x occupied by a neighbor to the left.
fn draw_steer_indicator(
    buf: &mut Buffer,
    clip: Rect,
    node_x: u16,
    content_y: u16,
    left_bound: u16,
) {
    let style = Style::default().fg(theme::NODE_STEER);
    let avail = node_x.saturating_sub(left_bound + 1);

    if avail == 0 {
        return;
    }

    let arrow_end = node_x.saturating_sub(1);

    if avail >= 9 {
        // Full: "steer ──▶"
        let arrow_start = arrow_end.saturating_sub(2);
        for x in arrow_start..arrow_end {
            set_char_clipped(buf, clip, x, content_y, '─', style);
        }
        set_char_clipped(buf, clip, arrow_end, content_y, '▶', style);
        let label_x = arrow_start.saturating_sub(6);
        set_string_clipped(buf, clip, label_x, content_y, "steer", style);
    } else if avail >= 4 {
        // Short: "s ─▶"
        let arrow_start = arrow_end.saturating_sub(1);
        set_char_clipped(buf, clip, arrow_start, content_y, '─', style);
        set_char_clipped(buf, clip, arrow_end, content_y, '▶', style);
        let label_x = arrow_start.saturating_sub(2);
        set_string_clipped(buf, clip, label_x, content_y, "s", style);
    } else if avail >= 2 {
        // Minimal: "─▶"
        set_char_clipped(buf, clip, arrow_end.saturating_sub(1), content_y, '─', style);
        set_char_clipped(buf, clip, arrow_end, content_y, '▶', style);
    } else {
        // Just arrow
        set_char_clipped(buf, clip, arrow_end, content_y, '▶', style);
    }
}

/// Draw a compact status legend in the bottom-left corner of the graph.
fn draw_legend(buf: &mut Buffer, clip: Rect, spinner: char) {
    let items: Vec<(char, &str, Color)> = vec![
        ('◌', "pending", theme::NODE_PENDING),
        ('●', "idle", theme::NODE_IDLE),
        (spinner, "active", theme::NODE_BUSY),
        ('○', "stopped", theme::NODE_STOPPED),
        ('▲', "query", theme::TEAL),
    ];

    let y = clip.y + clip.height.saturating_sub(1);
    let mut x = clip.x + 1;
    let dim = Style::default().fg(theme::FG_DARK);

    for (i, (icon, label, color)) in items.iter().enumerate() {
        if x + 2 + label.len() as u16 > clip.x + clip.width {
            break;
        }
        set_char_clipped(buf, clip, x, y, *icon, Style::default().fg(*color));
        x += 1;
        set_string_clipped(buf, clip, x, y, &format!(" {label}"), dim);
        x += 1 + label.len() as u16;
        if i < items.len() - 1 {
            set_string_clipped(buf, clip, x, y, "  ", dim);
            x += 2;
        }
    }
}

/// Draw an edge between two node positions (top-down flow).
fn draw_edge(
    buf: &mut Buffer,
    clip: Rect,
    from: (u16, u16),
    to: (u16, u16),
    edge: &GraphEdge,
    _labels: &mut Vec<(u16, u16, String)>,
    start_offset: i16,
    end_offset: i16,
    from_h: u16,
) {
    let style = if GraphState::is_edge_active(edge) {
        Style::default()
            .fg(theme::AQUA)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG_DARK)
    };

    let nw = NODE_WIDTH as u16;

    let start_x = ((from.0 as i32) + (nw as i32) / 2 + start_offset as i32).max(0) as u16;
    let start_y = from.1 + from_h;
    let end_x = ((to.0 as i32) + (nw as i32) / 2 + end_offset as i32).max(0) as u16;
    let end_y = to.1;

    if start_x == end_x {
        // Straight vertical line.
        if end_y > start_y + 1 {
            draw_vertical_line(buf, clip, start_x, start_y, end_y.saturating_sub(2), style);
        }
        set_char_clipped(buf, clip, end_x, end_y.saturating_sub(1), '▼', style);
    } else {
        // L-shaped route with rounded corners.
        let mid_y = start_y + end_y.saturating_sub(start_y) / 2;

        // Vertical segment from source down to bend.
        if mid_y > start_y {
            draw_vertical_line(buf, clip, start_x, start_y, mid_y.saturating_sub(1), style);
        }

        // Horizontal segment with corners.
        if start_x < end_x {
            set_char_clipped(buf, clip, start_x, mid_y, '╰', style);
            if end_x > start_x + 1 {
                draw_horizontal_line(buf, clip, start_x + 1, end_x, mid_y, style);
            }
            set_char_clipped(buf, clip, end_x, mid_y, '╮', style);
        } else {
            set_char_clipped(buf, clip, start_x, mid_y, '╯', style);
            if start_x > end_x + 1 {
                draw_horizontal_line(buf, clip, end_x + 1, start_x, mid_y, style);
            }
            set_char_clipped(buf, clip, end_x, mid_y, '╭', style);
        }

        // Vertical segment from bend down to target.
        if end_y > mid_y + 2 {
            draw_vertical_line(buf, clip, end_x, mid_y + 1, end_y.saturating_sub(2), style);
        }
        set_char_clipped(buf, clip, end_x, end_y.saturating_sub(1), '▼', style);
    }

}

/// Draw a back edge (feedback loop) routed to the right side, going upward.
fn draw_back_edge(
    buf: &mut Buffer,
    clip: Rect,
    from: (u16, u16),
    to: (u16, u16),
    signals: &[String],
    style: Style,
    labels: &mut Vec<(u16, u16, String)>,
    route_x: u16,
    exit_y: u16,
    entry_y: u16,
) {
    let nw = NODE_WIDTH as u16;

    let start_x = from.0 + nw;
    let start_y = exit_y;
    let end_x = to.0 + nw;
    let end_y = entry_y;

    // Horizontal from source to route column.
    draw_horizontal_line(buf, clip, start_x, route_x, start_y, style);
    set_char_clipped(buf, clip, route_x, start_y, '╯', style);

    // Vertical upward.
    if start_y > end_y + 1 {
        draw_vertical_line(buf, clip, route_x, end_y + 1, start_y.saturating_sub(1), style);
    }

    // Corner at top.
    set_char_clipped(buf, clip, route_x, end_y, '╮', style);

    // Horizontal from route column back to target.
    if route_x > end_x + 1 {
        draw_horizontal_line(buf, clip, end_x + 1, route_x, end_y, style);
    }

    // Arrow pointing left at target.
    set_char_clipped(buf, clip, end_x, end_y, '◀', style);

    // Signal labels beside the vertical segment, stacked vertically.
    let base_y = end_y + start_y.saturating_sub(end_y) / 2;
    for (j, signal) in signals.iter().enumerate() {
        if !signal.is_empty() {
            labels.push((route_x + 2, base_y + j as u16, signal.clone()));
        }
    }
}

/// Draw a forward edge that skips layers, routed to the right side to avoid crossing nodes.
fn draw_skip_forward_edge(
    buf: &mut Buffer,
    clip: Rect,
    from: (u16, u16),
    to: (u16, u16),
    signal: &str,
    style: Style,
    labels: &mut Vec<(u16, u16, String)>,
    route_x: u16,
    from_h: u16,
) {
    let nw = NODE_WIDTH as u16;

    let start_x = from.0 + nw;
    let start_y = from.1 + from_h / 2;
    let end_x = to.0 + nw / 2;
    let end_y = to.1;
    let turn_y = end_y.saturating_sub(2);

    // Horizontal from source right side to route column.
    if route_x > start_x {
        draw_horizontal_line(buf, clip, start_x, route_x, start_y, style);
    }
    set_char_clipped(buf, clip, route_x, start_y, '╮', style);

    // Vertical downward.
    if turn_y > start_y + 1 {
        draw_vertical_line(buf, clip, route_x, start_y + 1, turn_y.saturating_sub(1), style);
    }

    // Bottom-right corner.
    set_char_clipped(buf, clip, route_x, turn_y, '╯', style);

    // Horizontal from route column back toward target center.
    if route_x > end_x + 1 {
        draw_horizontal_line(buf, clip, end_x + 1, route_x, turn_y, style);
    }

    // Bottom-left corner turning down into target.
    set_char_clipped(buf, clip, end_x, turn_y, '╭', style);

    // Arrow into target.
    set_char_clipped(buf, clip, end_x, end_y.saturating_sub(1), '▼', style);

    // Label beside the vertical segment.
    if !signal.is_empty() {
        let label_y = start_y + turn_y.saturating_sub(start_y) / 2;
        labels.push((route_x + 2, label_y, signal.to_string()));
    }
}

/// Draw a query edge (dashed line, bidirectional arrow) between two nodes.
fn draw_query_edge(
    buf: &mut Buffer,
    clip: Rect,
    from: (u16, u16),
    to: (u16, u16),
    edge: &GraphEdge,
    labels: &mut Vec<(u16, u16, String)>,
    start_offset: i16,
    end_offset: i16,
    from_h: u16,
) {
    let style = if GraphState::is_edge_active(edge) {
        Style::default()
            .fg(theme::AQUA)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::TEAL)
    };

    let nw = NODE_WIDTH as u16;

    let start_x = ((from.0 as i32) + (nw as i32) / 2 + start_offset as i32).max(0) as u16;
    let start_y = from.1 + from_h;
    let end_x = ((to.0 as i32) + (nw as i32) / 2 + end_offset as i32).max(0) as u16;
    let end_y = to.1;

    if start_x == end_x {
        set_char_clipped(buf, clip, start_x, start_y, '▲', style);
        if end_y > start_y + 2 {
            draw_dashed_vertical(buf, clip, start_x, start_y + 1, end_y.saturating_sub(2), style);
        }
        set_char_clipped(buf, clip, end_x, end_y.saturating_sub(1), '▼', style);
    } else {
        let mid_y = start_y + end_y.saturating_sub(start_y) / 2;

        set_char_clipped(buf, clip, start_x, start_y, '▲', style);
        if mid_y > start_y + 1 {
            draw_dashed_vertical(buf, clip, start_x, start_y + 1, mid_y.saturating_sub(1), style);
        }

        if start_x < end_x {
            set_char_clipped(buf, clip, start_x, mid_y, '╰', style);
            if end_x > start_x + 1 {
                draw_dashed_horizontal(buf, clip, start_x + 1, end_x, mid_y, style);
            }
            set_char_clipped(buf, clip, end_x, mid_y, '╮', style);
        } else {
            set_char_clipped(buf, clip, start_x, mid_y, '╯', style);
            if start_x > end_x + 1 {
                draw_dashed_horizontal(buf, clip, end_x + 1, start_x, mid_y, style);
            }
            set_char_clipped(buf, clip, end_x, mid_y, '╭', style);
        }

        if end_y > mid_y + 2 {
            draw_dashed_vertical(buf, clip, end_x, mid_y + 1, end_y.saturating_sub(2), style);
        }
        set_char_clipped(buf, clip, end_x, end_y.saturating_sub(1), '▼', style);
    }

    // Label below the source node, on the left side.
    if !edge.signal.is_empty() && start_y < end_y {
        let label_text = format!("?{}", edge.signal);
        let label_x = start_x.saturating_sub(2 + label_text.len() as u16);
        let label_y = start_y;
        labels.push((label_x, label_y, label_text));
    }
}

fn draw_dashed_horizontal(buf: &mut Buffer, clip: Rect, x1: u16, x2: u16, y: u16, style: Style) {
    let (start, end) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
    for x in start..end {
        let ch = if (x - start) % 2 == 0 { '╌' } else { ' ' };
        set_char_clipped(buf, clip, x, y, ch, style);
    }
}

fn draw_dashed_vertical(buf: &mut Buffer, clip: Rect, x: u16, y1: u16, y2: u16, style: Style) {
    let (start, end) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
    for y in start..=end {
        let ch = if (y - start) % 2 == 0 { '╎' } else { ' ' };
        set_char_clipped(buf, clip, x, y, ch, style);
    }
}

fn draw_horizontal_line(buf: &mut Buffer, clip: Rect, x1: u16, x2: u16, y: u16, style: Style) {
    let (start, end) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
    for x in start..end {
        set_char_clipped(buf, clip, x, y, '─', style);
    }
}

fn draw_vertical_line(buf: &mut Buffer, clip: Rect, x: u16, y1: u16, y2: u16, style: Style) {
    let (start, end) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
    for y in start..=end {
        set_char_clipped(buf, clip, x, y, '│', style);
    }
}

fn set_char_clipped(buf: &mut Buffer, clip: Rect, x: u16, y: u16, ch: char, style: Style) {
    if x >= clip.x && x < clip.x + clip.width && y >= clip.y && y < clip.y + clip.height {
        buf[(x, y)].set_char(ch).set_style(style);
    }
}

fn set_string_clipped(buf: &mut Buffer, clip: Rect, x: u16, y: u16, s: &str, style: Style) {
    if y < clip.y || y >= clip.y + clip.height {
        return;
    }
    for (i, ch) in s.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= clip.x && cx < clip.x + clip.width {
            buf[(cx, y)].set_char(ch).set_style(style);
        }
    }
}

fn draw_description_box(frame: &mut Frame, area: Rect, description: &str) {
    let text = description.trim().replace('\n', " ");
    let max_w = 44.min(area.width.saturating_sub(2)) as usize;
    if max_w < 10 || area.height < 4 {
        return;
    }

    let inner_w = max_w - 2;
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        if let Some(last) = lines.last_mut() {
            if last.len() + 1 + word.len() <= inner_w {
                last.push(' ');
                last.push_str(word);
                continue;
            }
        }
        lines.push(word.to_string());
    }

    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(1));
    let w = max_w as u16;

    let box_area = Rect::new(area.x + 1, area.y, w, h);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(Text::from(text))
        .block(block)
        .style(Style::default().fg(theme::FG_DIM))
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, box_area);
}
