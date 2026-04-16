use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Utc};
use crossterm::event::{Event, KeyCode, KeyModifiers};

use yrr_core::loader::ResolvedAgent;
use yrr_runtime::events::SwarmEvent;

use crate::graph::layout::{self, NODE_HEIGHT, NODE_WIDTH};
use crate::graph::model::{GraphState, NodeStatus};

/// Application phase — controls what the TUI shows and which keys are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Visualizing the swarm before running. Arrow keys navigate nodes.
    Preview,
    /// Swarm is executing. Shows live graph + logs.
    Running,
    /// Swarm has finished.
    Finished,
}

/// Active tab in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Graph,
    Logs,
}

/// What the user is currently viewing in the Graph tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphView {
    /// Normal graph overview.
    Overview,
    /// Inspecting a specific agent.
    Inspect,
}

/// State for the seed input prompt.
#[derive(Debug, Clone)]
pub struct SeedInput {
    pub text: String,
    pub cursor: usize,
}

impl SeedInput {
    pub fn new(prefill: &str) -> Self {
        Self {
            cursor: prefill.len(),
            text: prefill.to_string(),
        }
    }
}

/// State for the steer input prompt (mid-execution human guidance).
#[derive(Debug, Clone)]
pub struct SteerInput {
    pub text: String,
    pub cursor: usize,
    /// The target agent's swarm_key.
    pub target_agent: String,
}

impl SteerInput {
    pub fn new(target_agent: String) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            target_agent,
        }
    }
}

/// Pending steer request to be published by the event loop.
#[derive(Debug, Clone)]
pub struct SteerRequest {
    pub agent_name: String,
    pub payload: String,
}

/// A single log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub agent_name: String,
    pub event_type: String,
    pub detail: String,
    /// Full message payload for signal events (may be large).
    pub payload: Option<String>,
}

/// Log state with scrolling support and cursor selection.
pub struct LogState {
    pub entries: VecDeque<LogEntry>,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    /// Index of the cursor-selected entry (for payload inspection).
    pub cursor: usize,
    /// Whether the detail pane is open showing the full payload.
    pub detail_open: bool,
    /// Scroll offset within the detail pane.
    pub detail_scroll: usize,
    max_entries: usize,
    pub log_file: Option<std::io::BufWriter<std::fs::File>>,
}

impl LogState {
    fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            scroll_offset: 0,
            auto_scroll: true,
            cursor: 0,
            detail_open: false,
            detail_scroll: 0,
            max_entries: 2000,
            log_file: None,
        }
    }

    fn push(&mut self, entry: LogEntry) {
        if let Some(writer) = &mut self.log_file {
            use std::io::Write;
            let ts = entry.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ");
            let _ = writeln!(
                writer,
                "[{ts}] {:<12} {:<18} {}",
                entry.agent_name, entry.event_type, entry.detail
            );
            if let Some(payload) = &entry.payload {
                let _ = writeln!(writer, "  payload: {payload}");
            }
            let _ = writer.flush();
        }
        self.entries.push_back(entry);
        if self.entries.len() > self.max_entries {
            self.entries.pop_front();
            if self.scroll_offset > 0 {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            self.cursor = self.cursor.saturating_sub(1);
        }
        if self.auto_scroll {
            self.cursor = self.entries.len().saturating_sub(1);
            self.scroll_to_bottom();
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.entries.len().saturating_sub(1);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.auto_scroll = false;
        self.cursor = self.cursor.saturating_sub(n);
        self.scroll_offset = self.scroll_offset.min(self.cursor);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.auto_scroll = false;
        let max = self.entries.len().saturating_sub(1);
        self.cursor = (self.cursor + n).min(max);
        // scroll_offset is adjusted during render to keep cursor visible.
    }

    pub fn go_top(&mut self) {
        self.auto_scroll = false;
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn go_bottom(&mut self) {
        self.auto_scroll = true;
        self.cursor = self.entries.len().saturating_sub(1);
        self.scroll_to_bottom();
    }

    pub fn toggle_detail(&mut self) {
        if let Some(entry) = self.entries.get(self.cursor) {
            if entry.payload.is_some() {
                self.detail_open = !self.detail_open;
                self.detail_scroll = 0;
            }
        }
    }

    /// Get the payload of the cursor-selected entry.
    pub fn selected_payload(&self) -> Option<&str> {
        self.entries
            .get(self.cursor)
            .and_then(|e| e.payload.as_deref())
    }
}

/// Spinner frames for busy-node animation.
pub const SPINNER_CHARS: &[char] = &['◜', '◝', '◞', '◟'];

/// Derive a human-readable display name from a full model ID.
/// E.g. "claude-sonnet-4-6" → "Sonnet 4.6", "claude-opus-4-7" → "Opus 4.7".
/// Unrecognised strings are returned as-is.
pub fn model_display_name(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("claude-") {
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() >= 3 {
            let mut family = parts[0].to_string();
            if let Some(first) = family.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            return format!("{} {}.{}", family, parts[1], parts[2]);
        }
    }
    id.to_string()
}

/// Info about an agent for the inspect panel.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub swarm_key: String,
    pub name: String,
    pub description: Option<String>,
    pub runtime: String,
    pub prompt: String,
    pub subscribe: Vec<(String, Option<String>)>,
    pub publish: Vec<(String, Option<String>)>,
    pub queryable: Vec<(String, Option<String>)>,
    pub query: Vec<(String, Option<String>)>,
    pub replicas: u32,
    pub model: Option<String>,
    pub lifecycle: Option<LifecycleInfo>,
    pub dispatch: Option<String>,
    pub source_path: Option<PathBuf>,
    pub permissions: Option<PermissionsInfo>,
    pub context: Option<String>,
    pub steer: Option<String>,
    pub spawn: Option<SpawnInfo>,
}

/// Formatted spawn info for display.
#[derive(Debug, Clone)]
pub struct SpawnInfo {
    pub on: String,
    pub max: u32,
}

/// Formatted lifecycle info for display.
#[derive(Debug, Clone)]
pub struct LifecycleInfo {
    pub mode: String,
    pub max_activations: Option<u32>,
    pub max_turns: Option<u32>,
    pub idle_timeout: Option<String>,
    pub max_uptime: Option<String>,
    pub die_on: Option<Vec<String>>,
}

/// Formatted permissions info for display.
#[derive(Debug, Clone)]
pub struct PermissionsInfo {
    pub tools_allow: Vec<String>,
    pub tools_deny: Vec<String>,
    pub paths_allow: Vec<String>,
    pub paths_deny: Vec<String>,
    pub network: Option<bool>,
}

impl AgentInfo {
    pub fn from_resolved(agent: &ResolvedAgent) -> Self {
        let lifecycle_desc = agent.lifecycle.as_ref().map(|lc| LifecycleInfo {
            mode: format!("{:?}", lc.mode).to_lowercase(),
            max_activations: lc.max_activations,
            max_turns: lc.max_turns,
            idle_timeout: lc.idle_timeout.clone(),
            max_uptime: lc.max_uptime.clone(),
            die_on: lc.die_on.clone(),
        });

        let dispatch_desc = agent.dispatch.as_ref().map(|d| format!("{d:?}"));

        let permissions = agent.def.permissions.as_ref().map(|p| {
            PermissionsInfo {
                tools_allow: p.tools.as_ref().map(|t| t.allow.clone()).unwrap_or_default(),
                tools_deny: p.tools.as_ref().map(|t| t.deny.clone()).unwrap_or_default(),
                paths_allow: p.paths.as_ref().map(|t| t.allow.clone()).unwrap_or_default(),
                paths_deny: p.paths.as_ref().map(|t| t.deny.clone()).unwrap_or_default(),
                network: p.network,
            }
        });

        let context_desc = agent.def.context.as_ref().map(|ctx| {
            format!("max_tokens: {}, on_limit: {:?}", ctx.max_tokens, ctx.on_limit)
        });

        let steer_desc = agent.def.steer.as_ref().map(|s| {
            match s {
                yrr_core::schema::Steer::Enabled => "true".to_string(),
                yrr_core::schema::Steer::Described(d) => d.clone(),
            }
        });

        let spawn_info = agent.spawn.as_ref().map(|s| SpawnInfo {
            on: s.on.clone(),
            max: s.max,
        });

        Self {
            swarm_key: agent.swarm_key.clone(),
            name: agent.def.name.clone(),
            description: agent.def.description.clone(),
            runtime: agent.def.runtime.clone(),
            prompt: agent.def.prompt.clone(),
            subscribe: agent
                .def
                .subscribe
                .iter()
                .map(|e| (e.name.clone(), e.description.clone()))
                .collect(),
            publish: agent
                .def
                .publish
                .iter()
                .map(|e| (e.name.clone(), e.description.clone()))
                .collect(),
            queryable: agent
                .def
                .queryable
                .iter()
                .map(|e| (e.name.clone(), e.description.clone()))
                .collect(),
            query: agent
                .def
                .query
                .iter()
                .map(|e| (e.name.clone(), e.description.clone()))
                .collect(),
            replicas: agent.replicas,
            model: agent
                .def
                .config
                .as_ref()
                .and_then(|c| c.get("model"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            lifecycle: lifecycle_desc,
            dispatch: dispatch_desc,
            source_path: agent.source_path.clone(),
            permissions,
            context: context_desc,
            steer: steer_desc,
            spawn: spawn_info,
        }
    }
}

/// Main application state.
pub struct App {
    pub phase: Phase,
    pub tab: Tab,
    pub graph_view: GraphView,
    pub graph: GraphState,
    pub logs: LogState,
    pub swarm_name: String,
    pub swarm_description: Option<String>,
    pub swarm_path: PathBuf,
    pub start_time: Option<Instant>,
    pub signal_count: u32,
    pub active_agents: u32,
    pub outcome_text: Option<String>,
    pub should_quit: bool,

    /// Agent info for the inspect panel, keyed by swarm_key.
    pub agent_info: Vec<AgentInfo>,

    /// Index of the currently selected node in graph.nodes (non-virtual only).
    pub selected_node: usize,

    /// Scroll offset within the inspect panel.
    pub inspect_scroll: usize,

    /// Whether the user wants to start running the swarm.
    pub run_requested: bool,

    /// Active seed input prompt (Preview mode).
    pub seed_input: Option<SeedInput>,

    /// The resolved seed message to use when running.
    pub seed: Option<String>,

    /// Whether the user wants to open a file in the editor.
    pub open_editor_request: Option<PathBuf>,

    /// Viewport scroll offset for the graph (x, y) in graph-space pixels.
    pub graph_scroll: (i32, i32),

    /// Spinner frame counter for busy-node animation.
    pub spinner_frame: usize,

    /// Active steer input prompt (Running mode).
    pub steer_input: Option<SteerInput>,

    /// Pending steer request to be published by the event loop.
    pub steer_request: Option<SteerRequest>,

    /// Names of steerable agents (for selection).
    pub steerable_agents: Vec<String>,

    /// Index into steerable_agents for agent selection.
    pub steer_agent_selector: usize,

    /// Whether the user is in agent selection mode (before typing steer text).
    pub steer_selecting_agent: bool,

    /// Whether the quit confirmation dialog is showing.
    pub confirm_quit: bool,

    /// Whether the user is manually scrolling the graph viewport.
    pub manual_graph_scroll: bool,

    /// Live spawn instance counts per agent name (for agents with spawn config).
    pub spawn_counts: std::collections::HashMap<String, u32>,
}

impl App {
    pub fn new(
        swarm_name: String,
        swarm_description: Option<String>,
        swarm_path: PathBuf,
        graph: GraphState,
        agent_info: Vec<AgentInfo>,
        seed: Option<String>,
    ) -> Self {
        let steerable_agents: Vec<String> = agent_info
            .iter()
            .filter(|a| a.steer.is_some())
            .map(|a| a.swarm_key.clone())
            .collect();

        Self {
            phase: Phase::Preview,
            tab: Tab::Graph,
            graph_view: GraphView::Overview,
            graph,
            logs: LogState::new(),
            swarm_name,
            swarm_description,
            swarm_path,
            start_time: None,
            signal_count: 0,
            active_agents: 0,
            outcome_text: None,
            should_quit: false,
            agent_info,
            selected_node: 0,
            inspect_scroll: 0,
            run_requested: false,
            seed_input: None,
            seed,
            open_editor_request: None,
            graph_scroll: (0, 0),
            spinner_frame: 0,
            steer_input: None,
            steer_request: None,
            steerable_agents,
            steer_agent_selector: 0,
            steer_selecting_agent: false,
            confirm_quit: false,
            manual_graph_scroll: false,
            spawn_counts: std::collections::HashMap::new(),
        }
    }

    fn is_spawn_agent(&self, name: &str) -> bool {
        self.agent_info.iter().any(|a| a.swarm_key == name && a.spawn.is_some())
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.start_time
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// Get the list of selectable (non-virtual) node IDs in navigation order.
    fn selectable_nodes(&self) -> Vec<&str> {
        let mut nodes: Vec<_> = self
            .graph
            .nodes
            .iter()
            .filter(|n| !n.virtual_node)
            .collect();
        nodes.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.position.cmp(&b.position)));
        nodes.iter().map(|n| n.id.as_str()).collect()
    }

    /// Get the currently selected node ID (non-virtual).
    pub fn selected_node_id(&self) -> Option<&str> {
        let nodes = self.selectable_nodes();
        nodes.get(self.selected_node).copied()
    }

    /// Get AgentInfo for the currently selected node.
    pub fn selected_agent_info(&self) -> Option<&AgentInfo> {
        let id = self.selected_node_id()?;
        self.agent_info.iter().find(|a| a.swarm_key == id)
    }

    /// Reload the graph and agent info after file changes.
    pub fn reload(&mut self, graph: GraphState, agent_info: Vec<AgentInfo>) {
        // Preserve selection if possible.
        let prev_id = self.selected_node_id().map(|s| s.to_string());
        self.steerable_agents = agent_info
            .iter()
            .filter(|a| a.steer.is_some())
            .map(|a| a.swarm_key.clone())
            .collect();
        self.graph = graph;
        self.agent_info = agent_info;

        // Try to restore selection.
        if let Some(prev) = prev_id {
            let nodes = self.selectable_nodes();
            if let Some(idx) = nodes.iter().position(|&id| id == prev) {
                self.selected_node = idx;
            } else {
                self.selected_node = 0;
            }
        }
    }

    /// Adjust graph_scroll so the selected node is visible within the given viewport.
    pub fn ensure_selected_visible(&mut self, view_width: u16, view_height: u16) {
        let Some(id) = self.selected_node_id() else {
            return;
        };
        let id = id.to_string();

        let (positions, graph_width, graph_height) = layout::graph_space_positions(&self.graph);
        let Some(&(node_x, node_y)) = positions.get(&id) else {
            return;
        };

        let vw = view_width as i32;
        let vh = view_height as i32;
        let h_margin = 4i32;
        // Vertical margin large enough to keep virtual seed/done nodes visible
        // when navigating to agents near the graph edges.
        let v_margin = (NODE_HEIGHT + layout::LAYER_SPACING) as i32;

        // Horizontal.
        if graph_width > vw {
            if node_x - self.graph_scroll.0 < h_margin {
                self.graph_scroll.0 = node_x - h_margin;
            }
            if node_x + NODE_WIDTH - self.graph_scroll.0 > vw - h_margin {
                self.graph_scroll.0 = node_x + NODE_WIDTH - vw + h_margin;
            }
            self.graph_scroll.0 = self.graph_scroll.0.max(0).min(graph_width - vw);
        } else {
            self.graph_scroll.0 = 0;
        }

        // Vertical.
        if graph_height > vh {
            if node_y - self.graph_scroll.1 < v_margin {
                self.graph_scroll.1 = node_y - v_margin;
            }
            if node_y + NODE_HEIGHT - self.graph_scroll.1 > vh - v_margin {
                self.graph_scroll.1 = node_y + NODE_HEIGHT - vh + v_margin;
            }
            self.graph_scroll.1 = self.graph_scroll.1.max(0).min(graph_height - vh);
        } else {
            self.graph_scroll.1 = 0;
        }
    }

    /// Clamp graph_scroll to valid bounds for the given viewport.
    pub fn clamp_graph_scroll(&mut self, view_width: u16, view_height: u16) {
        let (_, graph_width, graph_height) = layout::graph_space_positions(&self.graph);
        let vw = view_width as i32;
        let vh = view_height as i32;
        self.graph_scroll.0 = self.graph_scroll.0.clamp(0, (graph_width - vw).max(0));
        self.graph_scroll.1 = self.graph_scroll.1.clamp(0, (graph_height - vh).max(0));
    }

    /// Cap inspect_scroll so it doesn't exceed the content.
    pub fn cap_inspect_scroll(&mut self, content_lines: usize, visible_lines: usize) {
        let max = content_lines.saturating_sub(visible_lines);
        if self.inspect_scroll > max {
            self.inspect_scroll = max;
        }
    }

    pub fn handle_terminal_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            // Quit confirmation intercepts all keys.
            if self.confirm_quit {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => self.should_quit = true,
                    _ => self.confirm_quit = false,
                }
                return;
            }

            // Steer input mode intercepts all keys.
            if self.steer_input.is_some() || self.steer_selecting_agent {
                self.handle_steer_input_key(key.code, key.modifiers);
                return;
            }

            // Seed input mode intercepts all keys.
            if self.seed_input.is_some() {
                self.handle_seed_input_key(key.code, key.modifiers);
                return;
            }

            // Global keys.
            match key.code {
                KeyCode::Char('q') => {
                    if self.graph_view == GraphView::Inspect {
                        self.graph_view = GraphView::Overview;
                        self.inspect_scroll = 0;
                        return;
                    }
                    self.confirm_quit = true;
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                    return;
                }
                _ => {}
            }

            // Manual viewport scrolling (Shift+arrows) in graph overview.
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                let viewing_graph = match self.phase {
                    Phase::Preview => self.graph_view == GraphView::Overview,
                    Phase::Running | Phase::Finished => {
                        self.tab == Tab::Graph && self.graph_view == GraphView::Overview
                    }
                };
                if viewing_graph {
                    let amount = 4;
                    match key.code {
                        KeyCode::Left => {
                            self.graph_scroll.0 = (self.graph_scroll.0 - amount).max(0);
                            self.manual_graph_scroll = true;
                            return;
                        }
                        KeyCode::Right => {
                            self.graph_scroll.0 += amount;
                            self.manual_graph_scroll = true;
                            return;
                        }
                        KeyCode::Up => {
                            self.graph_scroll.1 = (self.graph_scroll.1 - amount).max(0);
                            self.manual_graph_scroll = true;
                            return;
                        }
                        KeyCode::Down => {
                            self.graph_scroll.1 += amount;
                            self.manual_graph_scroll = true;
                            return;
                        }
                        _ => {}
                    }
                }
            }

            match self.phase {
                Phase::Preview => self.handle_preview_keys(key.code),
                Phase::Running => self.handle_running_keys(key.code),
                Phase::Finished => self.handle_running_keys(key.code),
            }
        }
    }

    fn handle_seed_input_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let input = self.seed_input.as_mut().unwrap();
        match code {
            KeyCode::Esc => {
                self.seed_input = None;
            }
            KeyCode::Enter => {
                let text = input.text.trim().to_string();
                self.seed = if text.is_empty() { None } else { Some(text) };
                self.seed_input = None;
                self.run_requested = true;
            }
            KeyCode::Backspace => {
                if input.cursor > 0 {
                    input.text.remove(input.cursor - 1);
                    input.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if input.cursor < input.text.len() {
                    input.text.remove(input.cursor);
                }
            }
            KeyCode::Left => {
                input.cursor = input.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                input.cursor = (input.cursor + 1).min(input.text.len());
            }
            KeyCode::Home | KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.cursor = 0;
            }
            KeyCode::End | KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.cursor = input.text.len();
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.text.drain(..input.cursor);
                input.cursor = 0;
            }
            KeyCode::Char(c) => {
                input.text.insert(input.cursor, c);
                input.cursor += 1;
            }
            _ => {}
        }
    }

    fn handle_preview_keys(&mut self, code: KeyCode) {
        match self.graph_view {
            GraphView::Overview => match code {
                // Navigation between nodes.
                KeyCode::Up | KeyCode::Char('k') => self.navigate_left(),
                KeyCode::Down | KeyCode::Char('j') => self.navigate_right(),
                KeyCode::Left | KeyCode::Char('h') => self.navigate_up(),
                KeyCode::Right | KeyCode::Char('l') => self.navigate_down(),
                // Inspect selected agent.
                KeyCode::Enter | KeyCode::Char('i') => {
                    if self.selected_agent_info().is_some() {
                        self.graph_view = GraphView::Inspect;
                        self.inspect_scroll = 0;
                    }
                }
                // Open in editor.
                KeyCode::Char('e') => {
                    if let Some(info) = self.selected_agent_info() {
                        if let Some(path) = &info.source_path {
                            self.open_editor_request = Some(path.clone());
                        }
                    }
                }
                // Open swarm file in editor.
                KeyCode::Char('E') => {
                    self.open_editor_request = Some(self.swarm_path.clone());
                }
                // Run the swarm — open seed input prompt.
                KeyCode::Char('r') => {
                    self.open_seed_input();
                }
                _ => {}
            },
            GraphView::Inspect => match code {
                // Go back to graph overview.
                KeyCode::Esc | KeyCode::Backspace => {
                    self.graph_view = GraphView::Overview;
                    self.inspect_scroll = 0;
                }
                // Scroll within inspect panel.
                KeyCode::Up | KeyCode::Char('k') => {
                    self.inspect_scroll = self.inspect_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.inspect_scroll += 1;
                }
                KeyCode::PageUp => {
                    self.inspect_scroll = self.inspect_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.inspect_scroll += 20;
                }
                // Open in editor from inspect view.
                KeyCode::Char('e') => {
                    if let Some(info) = self.selected_agent_info() {
                        if let Some(path) = &info.source_path {
                            self.open_editor_request = Some(path.clone());
                        }
                    }
                }
                // Run the swarm — open seed input prompt.
                KeyCode::Char('r') => {
                    self.open_seed_input();
                }
                _ => {}
            },
        }
    }

    fn open_steer_input(&mut self) {
        if self.steerable_agents.is_empty() {
            return;
        }
        if self.steerable_agents.len() == 1 {
            self.steer_input = Some(SteerInput::new(self.steerable_agents[0].clone()));
        } else {
            self.steer_selecting_agent = true;
            self.steer_agent_selector = 0;
        }
    }

    fn handle_steer_input_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.steer_selecting_agent {
            match code {
                KeyCode::Esc => {
                    self.steer_selecting_agent = false;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.steer_agent_selector =
                        self.steer_agent_selector.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let max = self.steerable_agents.len().saturating_sub(1);
                    self.steer_agent_selector =
                        (self.steer_agent_selector + 1).min(max);
                }
                KeyCode::Enter => {
                    let agent = self.steerable_agents[self.steer_agent_selector].clone();
                    self.steer_selecting_agent = false;
                    self.steer_input = Some(SteerInput::new(agent));
                }
                _ => {}
            }
            return;
        }

        let input = self.steer_input.as_mut().unwrap();
        match code {
            KeyCode::Esc => {
                self.steer_input = None;
            }
            KeyCode::Enter => {
                let text = input.text.trim().to_string();
                if !text.is_empty() {
                    self.steer_request = Some(SteerRequest {
                        agent_name: input.target_agent.clone(),
                        payload: text,
                    });
                }
                self.steer_input = None;
            }
            KeyCode::Backspace => {
                if input.cursor > 0 {
                    input.text.remove(input.cursor - 1);
                    input.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if input.cursor < input.text.len() {
                    input.text.remove(input.cursor);
                }
            }
            KeyCode::Left => {
                input.cursor = input.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                input.cursor = (input.cursor + 1).min(input.text.len());
            }
            KeyCode::Home | KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.cursor = 0;
            }
            KeyCode::End | KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.cursor = input.text.len();
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.text.drain(..input.cursor);
                input.cursor = 0;
            }
            KeyCode::Char(c) => {
                input.text.insert(input.cursor, c);
                input.cursor += 1;
            }
            _ => {}
        }
    }

    fn open_seed_input(&mut self) {
        let prefill = self.seed.as_deref().unwrap_or("");
        self.seed_input = Some(SeedInput::new(prefill));
    }

    fn handle_running_keys(&mut self, code: KeyCode) {
        match code {
            KeyCode::Tab => {
                self.tab = match self.tab {
                    Tab::Graph => Tab::Logs,
                    Tab::Logs => Tab::Graph,
                };
            }
            KeyCode::Char('1') => self.tab = Tab::Graph,
            KeyCode::Char('2') => self.tab = Tab::Logs,
            _ => match self.tab {
                Tab::Graph => self.handle_running_graph_keys(code),
                Tab::Logs => self.handle_running_log_keys(code),
            },
        }
    }

    fn handle_running_graph_keys(&mut self, code: KeyCode) {
        match self.graph_view {
            GraphView::Overview => match code {
                KeyCode::Up | KeyCode::Char('k') => self.navigate_left(),
                KeyCode::Down | KeyCode::Char('j') => self.navigate_right(),
                KeyCode::Left | KeyCode::Char('h') => self.navigate_up(),
                KeyCode::Right | KeyCode::Char('l') => self.navigate_down(),
                KeyCode::Enter | KeyCode::Char('i') => {
                    if self.selected_agent_info().is_some() {
                        self.graph_view = GraphView::Inspect;
                        self.inspect_scroll = 0;
                    }
                }
                KeyCode::Char('s') => self.open_steer_input(),
                _ => {}
            },
            GraphView::Inspect => match code {
                KeyCode::Esc | KeyCode::Backspace => {
                    self.graph_view = GraphView::Overview;
                    self.inspect_scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.inspect_scroll = self.inspect_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.inspect_scroll += 1;
                }
                KeyCode::PageUp => {
                    self.inspect_scroll = self.inspect_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.inspect_scroll += 20;
                }
                _ => {}
            },
        }
    }

    fn handle_running_log_keys(&mut self, code: KeyCode) {
        if self.logs.detail_open {
            // Detail pane is open — scroll detail or close it.
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.logs.detail_open = false;
                    self.logs.detail_scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.logs.detail_scroll = self.logs.detail_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.logs.detail_scroll += 1;
                }
                KeyCode::PageUp => {
                    self.logs.detail_scroll = self.logs.detail_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.logs.detail_scroll += 20;
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.logs.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.logs.scroll_down(1),
            KeyCode::PageUp => self.logs.scroll_up(20),
            KeyCode::PageDown => self.logs.scroll_down(20),
            KeyCode::Char('g') => self.logs.go_top(),
            KeyCode::Char('G') => self.logs.go_bottom(),
            KeyCode::Enter => self.logs.toggle_detail(),
            _ => {}
        }
    }

    // ── Node navigation ──────────────────────────────────────────────────────

    fn navigate_left(&mut self) {
        self.manual_graph_scroll = false;
        let nodes = self.selectable_nodes();
        if nodes.is_empty() {
            return;
        }
        let current = self
            .graph
            .nodes
            .iter()
            .find(|n| !n.virtual_node && n.id == nodes[self.selected_node]);
        let Some(current) = current else { return };
        let current_layer = current.layer;
        let current_pos = current.position;

        // Find the closest node in the previous layer.
        let mut best: Option<(usize, usize)> = None; // (index_in_selectable, position_distance)
        for (i, &id) in nodes.iter().enumerate() {
            if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                if node.layer < current_layer {
                    let dist = (node.position as isize - current_pos as isize).unsigned_abs();
                    let layer_dist = current_layer - node.layer;
                    if layer_dist == 1 {
                        if best.is_none() || dist < best.unwrap().1 {
                            best = Some((i, dist));
                        }
                    }
                }
            }
        }
        // Fallback: any node in a previous layer.
        if best.is_none() {
            for (i, &id) in nodes.iter().enumerate() {
                if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                    if node.layer < current_layer {
                        let dist = (node.position as isize - current_pos as isize).unsigned_abs();
                        if best.is_none() || dist < best.unwrap().1 {
                            best = Some((i, dist));
                        }
                    }
                }
            }
        }
        if let Some((idx, _)) = best {
            self.selected_node = idx;
        }
    }

    fn navigate_right(&mut self) {
        self.manual_graph_scroll = false;
        let nodes = self.selectable_nodes();
        if nodes.is_empty() {
            return;
        }
        let current = self
            .graph
            .nodes
            .iter()
            .find(|n| !n.virtual_node && n.id == nodes[self.selected_node]);
        let Some(current) = current else { return };
        let current_layer = current.layer;
        let current_pos = current.position;

        let mut best: Option<(usize, usize)> = None;
        for (i, &id) in nodes.iter().enumerate() {
            if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                if node.layer > current_layer {
                    let dist = (node.position as isize - current_pos as isize).unsigned_abs();
                    let layer_dist = node.layer - current_layer;
                    if layer_dist == 1 {
                        if best.is_none() || dist < best.unwrap().1 {
                            best = Some((i, dist));
                        }
                    }
                }
            }
        }
        if best.is_none() {
            for (i, &id) in nodes.iter().enumerate() {
                if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                    if node.layer > current_layer {
                        let dist = (node.position as isize - current_pos as isize).unsigned_abs();
                        if best.is_none() || dist < best.unwrap().1 {
                            best = Some((i, dist));
                        }
                    }
                }
            }
        }
        if let Some((idx, _)) = best {
            self.selected_node = idx;
        }
    }

    fn navigate_up(&mut self) {
        self.manual_graph_scroll = false;
        let nodes = self.selectable_nodes();
        if nodes.is_empty() {
            return;
        }
        let current = self
            .graph
            .nodes
            .iter()
            .find(|n| !n.virtual_node && n.id == nodes[self.selected_node]);
        let Some(current) = current else { return };
        let current_layer = current.layer;
        let current_pos = current.position;

        // Find the node above in the same layer (lower position).
        let mut best: Option<usize> = None;
        let mut best_pos: Option<usize> = None;
        for (i, &id) in nodes.iter().enumerate() {
            if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                if node.layer == current_layer && node.position < current_pos {
                    if best_pos.is_none() || node.position > best_pos.unwrap() {
                        best = Some(i);
                        best_pos = Some(node.position);
                    }
                }
            }
        }
        if let Some(idx) = best {
            self.selected_node = idx;
        }
    }

    fn navigate_down(&mut self) {
        self.manual_graph_scroll = false;
        let nodes = self.selectable_nodes();
        if nodes.is_empty() {
            return;
        }
        let current = self
            .graph
            .nodes
            .iter()
            .find(|n| !n.virtual_node && n.id == nodes[self.selected_node]);
        let Some(current) = current else { return };
        let current_layer = current.layer;
        let current_pos = current.position;

        let mut best: Option<usize> = None;
        let mut best_pos: Option<usize> = None;
        for (i, &id) in nodes.iter().enumerate() {
            if let Some(node) = self.graph.nodes.iter().find(|n| n.id == id) {
                if node.layer == current_layer && node.position > current_pos {
                    if best_pos.is_none() || node.position < best_pos.unwrap() {
                        best = Some(i);
                        best_pos = Some(node.position);
                    }
                }
            }
        }
        if let Some(idx) = best {
            self.selected_node = idx;
        }
    }

    // ── Swarm events ──────────────────────────────────────────────────────

    pub fn start_running(&mut self) {
        self.phase = Phase::Running;
        self.start_time = Some(Instant::now());
        self.graph_view = GraphView::Overview;
    }

    pub fn handle_swarm_event(&mut self, event: SwarmEvent) {
        match &event {
            SwarmEvent::AgentSpawned {
                agent_name,
                agent_id,
                replica_idx,
                model,
            } => {
                self.active_agents += 1;
                self.graph.set_node_status(agent_name, NodeStatus::Idle);
                if self.is_spawn_agent(agent_name) {
                    *self.spawn_counts.entry(agent_name.clone()).or_insert(0) += 1;
                }
                let model_str = model
                    .as_deref()
                    .map(|m| format!(" [{}]", model_display_name(m)))
                    .unwrap_or_default();
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "spawned".into(),
                    detail: format!("replica {replica_idx} ({agent_id}){model_str}"),
                    payload: None,
                });
            }
            SwarmEvent::AgentStopped {
                agent_id: _,
                agent_name,
                reason,
            } => {
                self.active_agents = self.active_agents.saturating_sub(1);
                self.graph.set_node_status(agent_name, NodeStatus::Stopped);
                if let Some(count) = self.spawn_counts.get_mut(agent_name) {
                    *count = count.saturating_sub(1);
                }
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "stopped".into(),
                    detail: reason.clone(),
                    payload: None,
                });
            }
            SwarmEvent::SignalReceived {
                agent_name,
                signal,
                from_agent,
                payload,
                ..
            } => {
                self.signal_count += 1;
                let truncated = truncate_for_log(payload, 80);
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "signal_received".into(),
                    detail: format!("{signal} from {from_agent}: {truncated}"),
                    payload: Some(payload.clone()),
                });
            }
            SwarmEvent::SignalEmitted {
                agent_name,
                signal,
                payload,
                ..
            } => {
                self.graph.fire_edge(agent_name, signal);
                let truncated = truncate_for_log(payload, 80);
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "signal_emitted".into(),
                    detail: format!("{signal}: {truncated}"),
                    payload: Some(payload.clone()),
                });
            }
            SwarmEvent::ActivationStarted {
                agent_name,
                trigger_signal,
                ..
            } => {
                self.graph.set_node_status(agent_name, NodeStatus::Busy);
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "activation".into(),
                    detail: format!("started (trigger: {trigger_signal})"),
                    payload: None,
                });
            }
            SwarmEvent::ActivationCompleted {
                agent_name,
                duration_ms,
                ..
            } => {
                self.graph.set_node_status(agent_name, NodeStatus::Idle);
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "activation".into(),
                    detail: format!("completed ({duration_ms}ms)"),
                    payload: None,
                });
            }
            SwarmEvent::ActivationFailed {
                agent_name, error, ..
            } => {
                self.graph.set_node_status(agent_name, NodeStatus::Idle);
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "error".into(),
                    detail: format!("activation failed: {error}"),
                    payload: None,
                });
            }
            SwarmEvent::SeedInjected { signal } => {
                self.signal_count += 1;
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "seed".into(),
                    event_type: "injected".into(),
                    detail: signal.clone(),
                    payload: None,
                });
            }
            SwarmEvent::DoneReceived { signal } => {
                self.phase = Phase::Finished;
                self.outcome_text = Some(format!("done: {signal}"));
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "swarm".into(),
                    event_type: "done".into(),
                    detail: signal.clone(),
                    payload: None,
                });
            }
            SwarmEvent::SwarmTimeout => {
                self.phase = Phase::Finished;
                self.outcome_text = Some("timeout".into());
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "swarm".into(),
                    event_type: "timeout".into(),
                    detail: String::new(),
                    payload: None,
                });
            }
            SwarmEvent::SwarmInterrupted => {
                self.phase = Phase::Finished;
                self.outcome_text = Some("interrupted".into());
            }
            SwarmEvent::SignalDispatched {
                signal,
                target_agent_id,
            } => {
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "dispatcher".into(),
                    event_type: "dispatched".into(),
                    detail: format!("{signal} -> {target_agent_id}"),
                    payload: None,
                });
            }
            SwarmEvent::SignalQueued { signal, queue_len } => {
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "dispatcher".into(),
                    event_type: "queued".into(),
                    detail: format!("{signal} (queue: {queue_len})"),
                    payload: None,
                });
            }
            SwarmEvent::SteerReceived {
                agent_name,
                payload,
                ..
            } => {
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: agent_name.clone(),
                    event_type: "steer_received".into(),
                    detail: truncate_for_log(&payload, 80),
                    payload: Some(payload.clone()),
                });
            }
            SwarmEvent::SteerSent {
                agent_name,
                payload,
            } => {
                self.logs.push(LogEntry {
                    timestamp: Utc::now(),
                    agent_name: "human".into(),
                    event_type: "steer_sent".into(),
                    detail: format!("→ {agent_name}: {}", truncate_for_log(&payload, 60)),
                    payload: Some(payload.clone()),
                });
            }
        }
    }

    pub fn handle_tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        self.graph.decay_edges();
    }
}

/// Truncate a payload for inline display in the log view.
/// Replaces newlines with spaces and caps length.
fn truncate_for_log(s: &str, max_len: usize) -> String {
    let oneline: String = s.trim().chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if oneline.len() <= max_len {
        oneline
    } else {
        let mut end = max_len;
        while !oneline.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &oneline[..end])
    }
}
