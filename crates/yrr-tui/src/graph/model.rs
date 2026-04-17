use std::time::Instant;

/// Status of a graph node (agent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Pending,
    Idle,
    Busy,
    Stopped,
}

/// A node in the swarm graph (one per agent).
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Agent name / swarm key.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Layer index (horizontal position in the DAG).
    pub layer: usize,
    /// Position within the layer (vertical position).
    pub position: usize,
    /// Current status.
    pub status: NodeStatus,
    /// Whether this is a virtual node (prompt/done).
    pub virtual_node: bool,
    /// Number of replicas.
    pub replicas: u32,
    /// Whether this agent accepts human steering.
    pub steerable: bool,
}

/// A directed edge in the swarm graph (signal).
#[derive(Debug, Clone)]
pub struct GraphEdge {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Signal name.
    pub signal: String,
    /// When this edge last fired (for animation).
    pub last_fired: Option<Instant>,
    /// Whether this is a back edge (cycle).
    pub is_back_edge: bool,
    /// Whether this is a query edge (synchronous request/reply).
    pub is_query: bool,
}

/// Full graph state for the TUI.
#[allow(dead_code)]
pub struct GraphState {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub layer_count: usize,
    pub max_layer_size: usize,
    /// Output stubs: (node_id, signal_name) for signals meant for human consumption.
    pub output_stubs: Vec<(String, String)>,
}

impl GraphState {
    /// Set the status of a node by agent name.
    /// Updates all nodes matching that name (handles replicas showing as one node).
    pub fn set_node_status(&mut self, agent_name: &str, status: NodeStatus) {
        for node in &mut self.nodes {
            if node.id == agent_name {
                node.status = status;
            }
        }
    }

    /// Mark an edge as fired (for animation highlight).
    pub fn fire_edge(&mut self, from_agent: &str, signal: &str) {
        for edge in &mut self.edges {
            if edge.from == from_agent && edge.signal == signal {
                edge.last_fired = Some(Instant::now());
            }
        }
    }

    /// Decay edge highlights older than the threshold.
    pub fn decay_edges(&mut self) {
        let threshold = std::time::Duration::from_secs(3);
        for edge in &mut self.edges {
            if let Some(fired) = edge.last_fired {
                if fired.elapsed() > threshold {
                    edge.last_fired = None;
                }
            }
        }
    }

    /// Check if an edge was recently fired (for rendering).
    pub fn is_edge_active(edge: &GraphEdge) -> bool {
        edge.last_fired.is_some()
    }
}
