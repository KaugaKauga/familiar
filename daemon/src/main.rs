mod copilot;
mod github;
mod pipeline;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use tui::{DashboardState, PipelineView};

/// Guild -- an autonomous software factory daemon.
/// Monitors a GitHub repo for labeled issues and drives them through
/// a pipeline that ends with a pull request.
#[derive(Parser, Debug)]
#[command(name = "guild", version, about)]
struct Cli {
    /// GitHub repo in owner/repo format
    #[arg(short, long)]
    repo: String,

    /// Issue label to filter on
    #[arg(short, long, default_value = "guild")]
    label: String,

    /// Seconds between polling cycles
    #[arg(short, long, default_value_t = 30)]
    poll_interval: u64,

    /// Name or path of the copilot CLI binary
    #[arg(long, default_value = "copilot")]
    copilot_cmd: String,

    /// Directory where run artifacts are stored
    #[arg(long, default_value = "./runs")]
    runs_dir: String,
}

/// Runtime configuration derived from CLI arguments.
#[derive(Clone, Debug)]
pub struct Config {
    pub repo: String,
    pub label: String,
    pub poll_interval: u64,
    pub copilot_cmd: String,
    pub runs_dir: PathBuf,
}

impl Config {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            repo: cli.repo.clone(),
            label: cli.label.clone(),
            poll_interval: cli.poll_interval,
            copilot_cmd: cli.copilot_cmd.clone(),
            runs_dir: PathBuf::from(&cli.runs_dir),
        }
    }
}

/// Persist the current pipelines map to state.json inside runs_dir.
fn persist_state(pipelines: &HashMap<u64, pipeline::Pipeline>, runs_dir: &Path) -> Result<()> {
    let state_path = runs_dir.join("state.json");
    let json =
        serde_json::to_string_pretty(pipelines).context("failed to serialize pipeline state")?;
    std::fs::write(&state_path, json)
        .with_context(|| format!("failed to write state file at {}", state_path.display()))?;
    info!(path = %state_path.display(), "persisted pipeline state");
    Ok(())
}

/// Load previously-persisted pipelines from state.json inside runs_dir.
/// Returns an empty map when the file does not exist.
fn load_state(runs_dir: &Path) -> Result<HashMap<u64, pipeline::Pipeline>> {
    let state_path = runs_dir.join("state.json");
    if !state_path.exists() {
        info!("no existing state file found, starting fresh");
        return Ok(HashMap::new());
    }
    let data = std::fs::read_to_string(&state_path)
        .with_context(|| format!("failed to read state file at {}", state_path.display()))?;
    let pipelines: HashMap<u64, pipeline::Pipeline> =
        serde_json::from_str(&data).with_context(|| {
            format!(
                "failed to deserialize state file at {}",
                state_path.display()
            )
        })?;
    info!(
        count = pipelines.len(),
        path = %state_path.display(),
        "loaded persisted pipeline state"
    );
    Ok(pipelines)
}

/// The daemon poll loop — runs as a background tokio task.
async fn run_daemon(
    config: Config,
    dashboard: Arc<Mutex<DashboardState>>,
    shutdown: Arc<AtomicBool>,
) {
    // Load persisted state
    let mut pipelines: HashMap<u64, pipeline::Pipeline> = match load_state(&config.runs_dir) {
        Ok(p) => p,
        Err(e) => {
            error!("failed to load state: {:#}", e);
            HashMap::new()
        }
    };

    // Sync initial pipeline views to dashboard
    update_dashboard(&pipelines, &dashboard);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown flag set, breaking out of main loop");
            break;
        }

        // 1. Fetch open issues that carry the target label.
        let issues = match github::fetch_labeled_issues(&config.repo, &config.label).await {
            Ok(issues) => {
                info!(count = issues.len(), "fetched labeled issues");
                // Clear error on success
                if let Ok(mut state) = dashboard.lock() {
                    state.error_message = None;
                    state.last_poll = Some(std::time::Instant::now());
                }
                issues
            }
            Err(e) => {
                error!("failed to fetch issues: {:#}", e);
                if let Ok(mut state) = dashboard.lock() {
                    state.error_message = Some(format!("Poll error: {}", e));
                    state.last_poll = Some(std::time::Instant::now());
                }
                tokio::time::sleep(std::time::Duration::from_secs(config.poll_interval)).await;
                continue;
            }
        };

        // 2. Create new pipelines for issues we have not seen yet.
        for issue in &issues {
            if !pipelines.contains_key(&issue.number) {
                info!(
                    issue = issue.number,
                    "new issue detected, creating pipeline"
                );
                let p =
                    pipeline::Pipeline::new(issue.number, config.repo.clone(), &config.runs_dir);
                pipelines.insert(issue.number, p);
            }
        }

        // 3. Advance every active pipeline.
        let keys: Vec<u64> = pipelines.keys().copied().collect();
        for key in keys {
            if let Some(p) = pipelines.get_mut(&key) {
                if p.is_done() || p.is_failed() {
                    continue;
                }
                match p.advance(&config).await {
                    Ok(true) => {
                        info!(issue = key, "pipeline made progress");
                    }
                    Ok(false) => {}
                    Err(e) => {
                        error!(issue = key, "pipeline advance error: {:#}", e);
                        warn!(issue = key, "pipeline marked as failed");
                    }
                }
            }
            // Update dashboard after each pipeline advance
            update_dashboard(&pipelines, &dashboard);
        }

        // 4. Remove completed (Done) pipelines.
        pipelines.retain(|issue, p| {
            if p.is_done() {
                info!(issue, "pipeline completed, removing from active set");
                false
            } else {
                true
            }
        });

        // 5. Persist current state.
        if let Err(e) = persist_state(&pipelines, &config.runs_dir) {
            error!("failed to persist state: {:#}", e);
        }

        // 6. Update dashboard
        update_dashboard(&pipelines, &dashboard);

        // 7. Check for shutdown before sleeping.
        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown flag set, breaking out of main loop");
            break;
        }

        info!(
            seconds = config.poll_interval,
            "sleeping until next poll cycle"
        );
        tokio::time::sleep(std::time::Duration::from_secs(config.poll_interval)).await;
    }

    // Final state persist on exit.
    info!("persisting final state before exit");
    if let Err(e) = persist_state(&pipelines, &config.runs_dir) {
        error!("failed to persist state on shutdown: {:#}", e);
    }

    info!("guild daemon loop exited cleanly");
}

/// Sync pipeline state to the TUI dashboard.
fn update_dashboard(
    pipelines: &HashMap<u64, pipeline::Pipeline>,
    dashboard: &Arc<Mutex<DashboardState>>,
) {
    if let Ok(mut state) = dashboard.lock() {
        state.pipelines = pipelines.values().map(PipelineView::from).collect();
        // Sort by issue number for stable display
        state.pipelines.sort_by_key(|p| p.issue_number);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- CLI / config ------------------------------------------------------
    let cli = Cli::parse();
    let config = Config::from_cli(&cli);

    // --- ensure runs dir ---------------------------------------------------
    std::fs::create_dir_all(&config.runs_dir).with_context(|| {
        format!(
            "failed to create runs directory at {}",
            config.runs_dir.display()
        )
    })?;

    // --- tracing to file instead of stdout ---------------------------------
    let log_path = config.runs_dir.join("guild.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open log file at {}", log_path.display()))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("guild=info")),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    info!("=======================================================");
    info!("  Guild daemon starting");
    info!("  repo           : {}", config.repo);
    info!("  label          : {}", config.label);
    info!("  poll_interval  : {}s", config.poll_interval);
    info!("  copilot_cmd    : {}", config.copilot_cmd);
    info!("  runs_dir       : {}", config.runs_dir.display());
    info!("=======================================================");

    // --- Display ASCII art banner ------------------------------------------
    tui::display_banner();
    eprintln!("  Starting Guild for {}...\n", config.repo);
    std::thread::sleep(std::time::Duration::from_secs(2));

    // --- Shared state between daemon and TUI -------------------------------
    let shutdown = Arc::new(AtomicBool::new(false));
    let dashboard = Arc::new(Mutex::new(DashboardState::new(
        config.repo.clone(),
        config.label.clone(),
        config.poll_interval,
    )));

    // --- Spawn daemon poll loop as background task -------------------------
    let daemon_config = config.clone();
    let daemon_dashboard = Arc::clone(&dashboard);
    let daemon_shutdown = Arc::clone(&shutdown);
    let daemon_handle = tokio::spawn(async move {
        run_daemon(daemon_config, daemon_dashboard, daemon_shutdown).await;
    });

    // --- Run TUI on main thread (needs terminal control) -------------------
    let tui_shutdown = Arc::clone(&shutdown);
    let tui_result = tui::run_tui(Arc::clone(&dashboard), tui_shutdown);

    // Signal shutdown to daemon
    shutdown.store(true, Ordering::SeqCst);

    // Wait for daemon to finish (with timeout)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), daemon_handle).await;

    if let Err(e) = tui_result {
        eprintln!("TUI error: {:#}", e);
    }

    eprintln!(
        "\n  Guild shut down cleanly. Logs at: {}\n",
        log_path.display()
    );
    Ok(())
}
