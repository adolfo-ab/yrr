mod app;
mod event;
mod graph;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use chrono::Utc;
use tracing::info;

use yrr_core::config::find_config;
use yrr_core::loader::{load_swarm, resolve_swarm};
use yrr_core::validation::validate_swarm;
use yrr_bus::bus::SignalBus;
use yrr_bus::zenoh_bus::ZenohBus;
use yrr_runtime::events::SwarmEvent;
use yrr_runtime::orchestrator::SwarmRunner;

use app::{AgentInfo, App, GraphView, Phase, Tab};
use event::AppEvent;

#[derive(Parser)]
#[command(name = "yrr", about = "TUI for yrr swarm visualization")]
struct Cli {
    /// Path to the swarm YAML file
    swarm: PathBuf,

    /// Initial seed message to inject
    #[arg(long)]
    seed: Option<String>,

    /// Timeout in seconds
    #[arg(long)]
    timeout: Option<u64>,

    /// Path to config file
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing to a file so it doesn't interfere with the TUI.
    {
        use tracing_subscriber::fmt;
        use tracing_subscriber::EnvFilter;
        let log_dir = std::env::current_dir()
            .unwrap_or_default()
            .join(".yrr")
            .join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let ts = Utc::now().format("%Y-%m-%dT%H-%M-%S");
        let file = std::fs::File::create(log_dir.join(format!("yrr-{ts}.log"))).ok();
        if let Some(file) = file {
            fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "yrr=debug".parse().unwrap()),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        } else {
            fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "yrr=info".parse().unwrap()),
                )
                .init();
        }
    }

    let cli = Cli::parse();

    // Load config.
    let config = if let Some(config_path) = &cli.config {
        yrr_core::config::load_config(config_path)
            .with_context(|| format!("failed to load config: {}", config_path.display()))?
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        find_config(&cwd).unwrap_or_default()
    };

    let swarm_path = cli.swarm.canonicalize().unwrap_or(cli.swarm.clone());

    let base_dir = swarm_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    // Initial load.
    let (resolved, watch_paths) = load_and_resolve(&swarm_path, &base_dir)?;

    let validation = validate_swarm(&resolved);
    for warning in &validation.warnings {
        info!("validation warning: {warning}");
    }

    let graph_state = graph::layout::build_graph(&resolved);
    let agent_info: Vec<AgentInfo> = resolved.agents.iter().map(AgentInfo::from_resolved).collect();

    // Resolve the seed: CLI flag > swarm default.
    let seed = cli.seed.or(resolved.seed_message.clone());

    let yrr_log_dir = std::env::current_dir()
        .unwrap_or_default()
        .join(".yrr")
        .join("logs");

    // Create the app in preview mode.
    let mut app = App::new(
        resolved.name.clone(),
        resolved.description.clone(),
        swarm_path.clone(),
        graph_state,
        agent_info,
        seed,
    );

    // Setup terminal.
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install panic hook to restore terminal.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    // Create event stream in preview mode (with file watching).
    let mut events = event::EventStream::new_preview(watch_paths);

    // Track shutdown sender for the swarm runner.
    let mut shutdown_tx: Option<tokio::sync::oneshot::Sender<()>> = None;

    // Bus handle for steering (created when swarm starts running).
    let mut steer_bus: Option<std::sync::Arc<ZenohBus>> = None;

    // Main loop.
    loop {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        // Auto-scroll graph to keep selected node visible.
        let viewing_graph = match app.phase {
            Phase::Preview => app.graph_view == GraphView::Overview,
            Phase::Running | Phase::Finished => {
                app.tab == Tab::Graph && app.graph_view == GraphView::Overview
            }
        };
        if viewing_graph {
            let size = terminal.size()?;
            let view_w = size.width.saturating_sub(2);
            let view_h = size.height.saturating_sub(3);
            if app.manual_graph_scroll {
                app.clamp_graph_scroll(view_w, view_h);
            } else {
                app.ensure_selected_visible(view_w, view_h);
            }
        }

        match events.next().await {
            AppEvent::Terminal(evt) => app.handle_terminal_event(evt),
            AppEvent::Swarm(evt) => app.handle_swarm_event(evt),
            AppEvent::Tick => app.handle_tick(),
            AppEvent::FileChanged => {
                // Only reload in preview mode — don't reload while running.
                if app.phase == Phase::Preview {
                    info!("file change detected, reloading swarm");
                    match load_and_resolve(&swarm_path, &base_dir) {
                        Ok((new_resolved, _)) => {
                            let new_graph = graph::layout::build_graph(&new_resolved);
                            let new_info: Vec<AgentInfo> = new_resolved
                                .agents
                                .iter()
                                .map(AgentInfo::from_resolved)
                                .collect();
                            app.reload(new_graph, new_info);
                            info!("swarm reloaded successfully");
                        }
                        Err(e) => {
                            info!("failed to reload swarm: {e}");
                        }
                    }
                }
            }
        }

        // Handle editor open requests — temporarily leave the TUI.
        if let Some(path) = app.open_editor_request.take() {
            open_in_editor(&mut terminal, &path)?;
        }

        // Handle steer request — publish steer message via bus.
        if let Some(req) = app.steer_request.take() {
            if let Some(bus) = &steer_bus {
                let bus = std::sync::Arc::clone(bus);
                let agent_name = req.agent_name.clone();
                let payload = req.payload.clone();

                app.handle_swarm_event(SwarmEvent::SteerSent {
                    agent_name: req.agent_name.clone(),
                    payload: req.payload.clone(),
                });

                tokio::spawn(async move {
                    if let Err(e) = bus.publish_steer(&agent_name, &payload).await {
                        tracing::error!(error = %e, "failed to publish steer");
                    }
                });
            }
        }

        // Handle run request — transition from Preview to Running.
        if app.run_requested && app.phase == Phase::Preview {
            app.run_requested = false;

            // Open timestamped log file for this run.
            let ts = Utc::now().format("%Y-%m-%dT%H-%M-%S");
            let log_path = yrr_log_dir.join(format!("{}-{ts}.log", app.swarm_name));
            if let Ok(file) = std::fs::File::create(&log_path) {
                use std::io::Write;
                let mut writer = std::io::BufWriter::new(file);
                let _ = writeln!(writer, "# yrr swarm log: {}", app.swarm_name);
                let _ = writeln!(writer, "# started: {}", Utc::now().to_rfc3339());
                let _ = writeln!(writer, "#");
                app.logs.log_file = Some(writer);
                info!("logging to {}", log_path.display());
            }

            // Re-resolve in case files changed.
            let run_resolved = match load_and_resolve(&swarm_path, &base_dir) {
                Ok((r, _)) => r,
                Err(e) => {
                    info!("failed to load swarm for run: {e}");
                    continue;
                }
            };

            app.start_running();

            let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SwarmEvent>();
            let (stx, shutdown_rx) = tokio::sync::oneshot::channel();
            shutdown_tx = Some(stx);

            events.attach_swarm_rx(event_rx);

            // Open a separate bus for steering.
            match ZenohBus::new(&run_resolved.name).await {
                Ok(bus) => {
                    steer_bus = Some(std::sync::Arc::new(bus));
                }
                Err(e) => {
                    info!("failed to open steer bus: {e}");
                }
            }

            let runner = SwarmRunner {
                resolved: run_resolved,
                config: config.clone(),
                seed: app.seed.clone(),
                timeout: cli.timeout.map(std::time::Duration::from_secs),
                event_tx: Some(event_tx),
            };

            tokio::spawn(async move {
                match runner.run(shutdown_rx).await {
                    Ok(outcome) => {
                        info!(?outcome, "swarm finished");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "swarm failed");
                    }
                }
            });
        }

        if app.should_quit {
            break;
        }
    }

    // Signal swarm to shut down (if running).
    if let Some(tx) = shutdown_tx {
        let _ = tx.send(());
    }

    // Restore terminal.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Load and resolve a swarm, returning the resolved swarm and the list
/// of file paths to watch for changes.
fn load_and_resolve(
    swarm_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> Result<(yrr_core::loader::ResolvedSwarm, Vec<PathBuf>)> {
    let wf_def = load_swarm(swarm_path)
        .with_context(|| format!("failed to load swarm: {}", swarm_path.display()))?;

    let resolved = resolve_swarm(&wf_def, base_dir).context("failed to resolve swarm")?;

    // Collect all file paths to watch: the swarm file itself + all agent source files.
    let mut watch_paths = vec![swarm_path.to_path_buf()];
    for agent in &resolved.agents {
        if let Some(path) = &agent.source_path {
            watch_paths.push(path.clone());
        }
    }

    Ok((resolved, watch_paths))
}

/// Temporarily leave the TUI, open a file in the user's editor, then return.
fn open_in_editor(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    path: &std::path::Path,
) -> Result<()> {
    // Restore terminal state.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Determine editor.
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    // Launch editor (blocking).
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status();

    match status {
        Ok(s) => info!("editor exited with {s}"),
        Err(e) => info!("failed to launch editor '{editor}': {e}"),
    }

    // Re-enter TUI mode.
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    Ok(())
}
