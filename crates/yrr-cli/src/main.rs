use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use yrr_bus::bus::SignalBus;
use yrr_bus::zenoh_bus::ZenohBus;
use yrr_core::config::{Config, find_config};
use yrr_core::loader::{load_swarm, resolve_swarm};
use yrr_core::message::SignalMessage;
use yrr_core::validation::validate_swarm;
use yrr_runtime::orchestrator::SwarmRunner;

#[derive(Parser)]
#[command(name = "yrr-cli", about = "AI agent orchestrator")]
struct Cli {
    /// Path to config file (default: searches for yrr.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a swarm
    Run {
        /// Path to the swarm YAML file
        swarm: PathBuf,
        /// Initial prompt message to inject
        #[arg(long)]
        prompt: Option<String>,
        /// Timeout in seconds — swarm exits after this duration
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Validate a swarm definition
    Validate {
        /// Path to the swarm YAML file
        swarm: PathBuf,
    },
    /// Inject a signal into a running swarm
    Inject {
        /// Signal name
        signal: String,
        /// Signal payload
        message: String,
        /// Swarm namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Send a steer message to a running agent
    Steer {
        /// Agent name (swarm key)
        agent: String,
        /// Steering message
        message: String,
        /// Swarm namespace
        #[arg(long)]
        namespace: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "yrr=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    // Load config: explicit path > search for yrr.toml > defaults.
    let config = if let Some(config_path) = &cli.config {
        yrr_core::config::load_config(config_path)
            .with_context(|| format!("failed to load config: {}", config_path.display()))?
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        find_config(&cwd).unwrap_or_default()
    };

    info!(
        max_activations = config.safety.max_activations,
        permission_mode = %config.claude.permission_mode,
        default_model = ?config.defaults.model,
        "loaded config"
    );

    match cli.command {
        Commands::Run {
            swarm,
            prompt,
            timeout,
        } => cmd_run(swarm, prompt, timeout, &config).await,
        Commands::Validate { swarm } => cmd_validate(swarm),
        Commands::Inject {
            signal,
            message,
            namespace,
        } => cmd_inject(signal, message, namespace).await,
        Commands::Steer {
            agent,
            message,
            namespace,
        } => cmd_steer(agent, message, namespace).await,
    }
}

async fn cmd_run(
    swarm_path: PathBuf,
    prompt: Option<String>,
    timeout: Option<u64>,
    config: &Config,
) -> Result<()> {
    let base_dir = swarm_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let wf_def = load_swarm(&swarm_path)
        .with_context(|| format!("failed to load swarm: {}", swarm_path.display()))?;

    let resolved = resolve_swarm(&wf_def, base_dir).context("failed to resolve swarm")?;

    // Validate and print warnings.
    let validation = validate_swarm(&resolved);
    for warning in &validation.warnings {
        eprintln!("  warning: {warning}");
    }

    let runner = SwarmRunner {
        resolved,
        config: config.clone(),
        prompt,
        timeout: timeout.map(std::time::Duration::from_secs),
        event_tx: None,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Ctrl+C received, shutting down");
        let _ = shutdown_tx.send(());
    });

    let outcome = runner.run(shutdown_rx).await?;
    info!(outcome = ?outcome, "swarm finished");

    Ok(())
}

fn cmd_validate(swarm_path: PathBuf) -> Result<()> {
    let base_dir = swarm_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let wf_def = load_swarm(&swarm_path)
        .with_context(|| format!("failed to load swarm: {}", swarm_path.display()))?;

    println!("Validating {}...", swarm_path.display());

    let resolved = resolve_swarm(&wf_def, base_dir).context("failed to resolve swarm")?;

    println!("  Resolved {} agents", resolved.agents.len());

    let validation = validate_swarm(&resolved);

    if validation.is_clean() {
        println!("  All checks passed!");
    } else {
        for warning in &validation.warnings {
            println!("  warning: {warning}");
        }
    }

    Ok(())
}

async fn cmd_inject(signal: String, message: String, namespace: Option<String>) -> Result<()> {
    let ns = namespace.unwrap_or_else(|| "default".to_string());
    let bus = ZenohBus::new(&ns)
        .await
        .context("failed to open zenoh bus")?;

    let msg = SignalMessage::prompt(&signal, &message);
    info!(signal = %signal, namespace = %ns, "injecting signal");
    bus.publish(&signal, &msg).await?;

    // Give Zenoh a moment to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    bus.close().await?;

    println!("Injected signal '{signal}' into namespace '{ns}'");
    Ok(())
}

async fn cmd_steer(agent: String, message: String, namespace: Option<String>) -> Result<()> {
    let ns = namespace.unwrap_or_else(|| "default".to_string());
    let bus = ZenohBus::new(&ns)
        .await
        .context("failed to open zenoh bus")?;

    info!(agent = %agent, namespace = %ns, "sending steer message");
    bus.publish_steer(&agent, &message).await?;

    // Give Zenoh a moment to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    bus.close().await?;

    println!("Steered agent '{agent}' in namespace '{ns}'");
    Ok(())
}
