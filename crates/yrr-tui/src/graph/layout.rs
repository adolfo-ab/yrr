use std::collections::{HashMap, HashSet, VecDeque};

use yrr_core::loader::ResolvedSwarm;

use super::model::{GraphEdge, GraphNode, GraphState, NodeStatus};

/// Node box dimensions (in terminal cells).
pub const NODE_WIDTH: i32 = 24;
pub const NODE_HEIGHT: i32 = 3;
/// Vertical spacing between layers (top-down flow).
pub const LAYER_SPACING: i32 = 5;
/// Horizontal spacing between nodes in a layer.
pub const NODE_SPACING: i32 = 6;

/// Compute node positions in graph space (origin 0,0), top-down layout.
/// Layers flow top-to-bottom, nodes within a layer are centered horizontally.
/// Returns: (positions map, total_width, total_height).
pub fn graph_space_positions(graph: &GraphState) -> (HashMap<String, (i32, i32)>, i32, i32) {
    let mut positions = HashMap::new();

    if graph.layer_count == 0 {
        return (positions, 0, 0);
    }

    // Top-down: height determined by layers, width by widest layer.
    let total_height = graph.layer_count as i32 * (NODE_HEIGHT + LAYER_SPACING);
    let total_width = graph.max_layer_size as i32 * (NODE_WIDTH + NODE_SPACING);

    let mut layers: HashMap<usize, Vec<&GraphNode>> = HashMap::new();
    for node in &graph.nodes {
        layers.entry(node.layer).or_default().push(node);
    }
    for nodes in layers.values_mut() {
        nodes.sort_by_key(|n| n.position);
    }

    for (layer, nodes) in &layers {
        let y = *layer as i32 * (NODE_HEIGHT + LAYER_SPACING);
        let layer_width = nodes.len() as i32 * (NODE_WIDTH + NODE_SPACING);
        let start_x = (total_width - layer_width) / 2;

        for (idx, node) in nodes.iter().enumerate() {
            let x = start_x + idx as i32 * (NODE_WIDTH + NODE_SPACING);
            positions.insert(node.id.clone(), (x, y));
        }
    }

    (positions, total_width, total_height)
}

/// Build the graph model from a resolved swarm.
///
/// Creates nodes for each agent, plus a virtual "prompt" node.
/// Done/output signals are shown as stubs below their publishing node.
/// Layout uses a layered (Sugiyama-style) algorithm.
pub fn build_graph(swarm: &ResolvedSwarm) -> GraphState {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Collect agent publish/subscribe info.
    let mut agent_publishes: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut agent_subscribes: HashMap<&str, Vec<&str>> = HashMap::new();

    for agent in &swarm.agents {
        let key = agent.swarm_key.as_str();
        agent_publishes.insert(key, agent.def.publish.names().collect());
        agent_subscribes.insert(key, agent.def.subscribe.names().collect());
    }

    // Build signal -> publishers and signal -> subscribers maps.
    let mut signal_publishers: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut signal_subscribers: HashMap<&str, Vec<&str>> = HashMap::new();

    for agent in &swarm.agents {
        let key = agent.swarm_key.as_str();
        for signal in agent.def.publish.names() {
            signal_publishers.entry(signal).or_default().push(key);
        }
        for signal in agent.def.subscribe.names() {
            signal_subscribers.entry(signal).or_default().push(key);
        }
    }

    // Create agent nodes.
    for agent in &swarm.agents {
        nodes.push(GraphNode {
            id: agent.swarm_key.clone(),
            label: agent.swarm_key.clone(),
            layer: 0,
            position: 0,
            status: NodeStatus::Pending,
            virtual_node: false,
            replicas: agent.replicas,
            steerable: agent.def.steer.is_some(),
        });
    }

    // Add virtual "prompt" node.
    nodes.push(GraphNode {
        id: "__prompt__".into(),
        label: "prompt".into(),
        layer: 0,
        position: 0,
        status: NodeStatus::Idle,
        virtual_node: true,
        replicas: 1,
        steerable: false,
    });

    // Output signals shown as outward stubs. Done signals also shown as stubs.
    let output_set: HashSet<&str> = swarm.output.iter().map(|s| s.as_str()).collect();
    let done_set: HashSet<&str> = swarm.done.iter().map(|s| s.as_str()).collect();
    let mut output_stubs: Vec<(String, String)> = Vec::new();
    let mut done_agents: HashSet<String> = HashSet::new();
    for agent in &swarm.agents {
        for signal in agent.def.publish.names() {
            if done_set.contains(signal) {
                if done_agents.insert(agent.swarm_key.clone()) {
                    output_stubs.push((agent.swarm_key.clone(), "done".to_string()));
                }
            } else if output_set.contains(signal) {
                output_stubs.push((agent.swarm_key.clone(), signal.to_string()));
            }
        }
    }

    // Build edges: for each signal, connect each publisher to each subscriber.
    let mut edge_set: HashSet<(String, String, String)> = HashSet::new();

    for agent in &swarm.agents {
        let pub_key = agent.swarm_key.as_str();
        for signal in agent.def.publish.names() {
            if let Some(subscribers) = signal_subscribers.get(signal) {
                for sub_key in subscribers {
                    let tuple = (pub_key.to_string(), sub_key.to_string(), signal.to_string());
                    if edge_set.insert(tuple) {
                        edges.push(GraphEdge {
                            from: pub_key.to_string(),
                            to: sub_key.to_string(),
                            signal: signal.to_string(),
                            last_fired: None,
                            is_back_edge: false,
                            is_query: false,
                        });
                    }
                }
            }
        }
    }

    // Build query edges: connect agents that issue queries to agents that serve them.
    let mut queryable_providers: HashMap<&str, Vec<&str>> = HashMap::new();
    for agent in &swarm.agents {
        for qkey in agent.def.queryable.names() {
            queryable_providers
                .entry(qkey)
                .or_default()
                .push(agent.swarm_key.as_str());
        }
    }
    for agent in &swarm.agents {
        let querier = agent.swarm_key.as_str();
        for qkey in agent.def.query.names() {
            if let Some(providers) = queryable_providers.get(qkey) {
                for provider in providers {
                    let tuple = (querier.to_string(), provider.to_string(), qkey.to_string());
                    if edge_set.insert(tuple) {
                        edges.push(GraphEdge {
                            from: querier.to_string(),
                            to: provider.to_string(),
                            signal: qkey.to_string(),
                            last_fired: None,
                            is_back_edge: false,
                            is_query: true,
                        });
                    }
                }
            }
        }
    }

    // Build spawn edges: connect publishers of a spawn signal to the spawned agent.
    for agent in &swarm.agents {
        if let Some(spawn) = &agent.spawn {
            if let Some(publishers) = signal_publishers.get(spawn.on.as_str()) {
                for pub_key in publishers {
                    let tuple = (
                        pub_key.to_string(),
                        agent.swarm_key.clone(),
                        spawn.on.clone(),
                    );
                    if edge_set.insert(tuple) {
                        edges.push(GraphEdge {
                            from: pub_key.to_string(),
                            to: agent.swarm_key.clone(),
                            signal: spawn.on.clone(),
                            last_fired: None,
                            is_back_edge: false,
                            is_query: false,
                        });
                    }
                }
            }
        }
    }

    // Connect prompt -> agents subscribing to entry signals.
    for entry_signal in &swarm.entry {
        if let Some(subscribers) = signal_subscribers.get(entry_signal.as_str()) {
            for sub_key in subscribers {
                edges.push(GraphEdge {
                    from: "__prompt__".into(),
                    to: sub_key.to_string(),
                    signal: entry_signal.clone(),
                    last_fired: None,
                    is_back_edge: false,
                    is_query: false,
                });
            }
        }
    }

    // Assign layers using BFS from prompt, handling cycles.
    let (layer_assignment, back_edges) = assign_layers(&nodes, &edges);

    // Mark back edges.
    for edge in &mut edges {
        let key = (edge.from.clone(), edge.to.clone(), edge.signal.clone());
        if back_edges.contains(&key) {
            edge.is_back_edge = true;
        }
    }

    // Apply layer assignments to nodes.
    for node in &mut nodes {
        if let Some(&layer) = layer_assignment.get(&node.id) {
            node.layer = layer;
        }
    }

    // Order nodes within layers and assign positions.
    let layer_count = order_within_layers(&mut nodes, &edges);

    // Calculate max layer size.
    let mut layer_sizes: HashMap<usize, usize> = HashMap::new();
    for node in &nodes {
        *layer_sizes.entry(node.layer).or_insert(0) += 1;
    }
    let max_layer_size = layer_sizes.values().copied().max().unwrap_or(1);

    GraphState {
        nodes,
        edges,
        layer_count,
        max_layer_size,
        output_stubs,
    }
}

/// Assign layers using BFS from prompt node. Returns layer assignments and detected back edges.
fn assign_layers(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
) -> (HashMap<String, usize>, HashSet<(String, String, String)>) {
    // Build adjacency list.
    let mut adj: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for edge in edges {
        if !edge.is_back_edge {
            adj.entry(edge.from.as_str())
                .or_default()
                .push((edge.to.as_str(), edge.signal.as_str()));
        }
    }

    // BFS from prompt.
    let mut layers: HashMap<String, usize> = HashMap::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut back_edges: HashSet<(String, String, String)> = HashSet::new();

    // Start from prompt node.
    queue.push_back(("__prompt__".to_string(), 0));
    layers.insert("__prompt__".to_string(), 0);
    visited.insert("__prompt__".to_string());

    while let Some((current, layer)) = queue.pop_front() {
        if let Some(neighbors) = adj.get(current.as_str()) {
            for (next, signal) in neighbors {
                let next_str = next.to_string();
                if visited.contains(&next_str) {
                    // Already visited — this is a back edge or cross edge.
                    // If the target layer <= current layer, mark as back edge.
                    if let Some(&existing_layer) = layers.get(&next_str) {
                        if existing_layer <= layer {
                            back_edges.insert((current.clone(), next_str, signal.to_string()));
                        }
                    }
                } else {
                    let next_layer = layer + 1;
                    layers.insert(next_str.clone(), next_layer);
                    visited.insert(next_str.clone());
                    queue.push_back((next_str, next_layer));
                }
            }
        }
    }

    // Assign unvisited nodes to layer 1 (disconnected agents).
    for node in nodes {
        if !layers.contains_key(&node.id) {
            layers.insert(node.id.clone(), 1);
        }
    }

    (layers, back_edges)
}

/// Order nodes within their layers using barycenter heuristic.
/// Returns the total number of layers.
fn order_within_layers(nodes: &mut [GraphNode], edges: &[GraphEdge]) -> usize {
    let layer_count = nodes.iter().map(|n| n.layer).max().unwrap_or(0) + 1;

    // Build forward adjacency for barycenter calculation.
    let mut adj_forward: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut adj_backward: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        if !edge.is_back_edge {
            adj_forward
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
            adj_backward
                .entry(edge.to.as_str())
                .or_default()
                .push(edge.from.as_str());
        }
    }

    // Initial ordering: sort by name within each layer.
    nodes.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.id.cmp(&b.id)));

    // Assign initial positions.
    let mut current_layer = 0;
    let mut pos = 0;
    for node in nodes.iter_mut() {
        if node.layer != current_layer {
            current_layer = node.layer;
            pos = 0;
        }
        node.position = pos;
        pos += 1;
    }

    // Barycenter iterations (2 passes forward + backward).
    for _iteration in 0..2 {
        // Forward pass: for each layer (1..n), order nodes by average position
        // of their predecessors in the previous layer.
        for layer in 1..layer_count {
            let prev_positions: HashMap<String, f64> = nodes
                .iter()
                .filter(|n| n.layer == layer - 1)
                .map(|n| (n.id.clone(), n.position as f64))
                .collect();

            let mut layer_nodes: Vec<(usize, f64)> = Vec::new();
            for (idx, node) in nodes.iter().enumerate() {
                if node.layer != layer {
                    continue;
                }
                let preds = adj_backward.get(node.id.as_str());
                let barycenter = if let Some(preds) = preds {
                    let positions: Vec<f64> = preds
                        .iter()
                        .filter_map(|p| prev_positions.get(*p))
                        .copied()
                        .collect();
                    if positions.is_empty() {
                        node.position as f64
                    } else {
                        positions.iter().sum::<f64>() / positions.len() as f64
                    }
                } else {
                    node.position as f64
                };
                layer_nodes.push((idx, barycenter));
            }

            layer_nodes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            for (pos, (idx, _)) in layer_nodes.iter().enumerate() {
                nodes[*idx].position = pos;
            }
        }

        // Backward pass.
        for layer in (0..layer_count.saturating_sub(1)).rev() {
            let next_positions: HashMap<String, f64> = nodes
                .iter()
                .filter(|n| n.layer == layer + 1)
                .map(|n| (n.id.clone(), n.position as f64))
                .collect();

            let mut layer_nodes: Vec<(usize, f64)> = Vec::new();
            for (idx, node) in nodes.iter().enumerate() {
                if node.layer != layer {
                    continue;
                }
                let succs = adj_forward.get(node.id.as_str());
                let barycenter = if let Some(succs) = succs {
                    let positions: Vec<f64> = succs
                        .iter()
                        .filter_map(|s| next_positions.get(*s))
                        .copied()
                        .collect();
                    if positions.is_empty() {
                        node.position as f64
                    } else {
                        positions.iter().sum::<f64>() / positions.len() as f64
                    }
                } else {
                    node.position as f64
                };
                layer_nodes.push((idx, barycenter));
            }

            layer_nodes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            for (pos, (idx, _)) in layer_nodes.iter().enumerate() {
                nodes[*idx].position = pos;
            }
        }
    }

    layer_count
}
