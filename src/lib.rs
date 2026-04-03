#![forbid(unsafe_code)]

pub mod agent;
pub mod client;
pub mod config;
pub mod endpoint;
pub mod mcp;
pub mod prompt;
pub mod setup;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;
use serde_json::json;
use tracing_subscriber::EnvFilter;

use crate::agent::{run_agent, AgentRunOptions};
use crate::client::OpenAiClient;
use crate::config::{AppConfig, McpAllowPolicy};
use crate::endpoint::select_endpoint;
use crate::mcp::RmcpBackend;
use crate::prompt::read_stdin_context;
use crate::setup::run_setup_wizard;

const HELP_MANPAGE: &str = r#"MANPAGE
  aihelp [OPTIONS] <QUESTION...>

SETUP
  First run (interactive): aihelp auto-runs setup and stores config.toml.
  Re-run setup anytime:    aihelp --setup
  Non-interactive/CI:      set AIHELP_NONINTERACTIVE=1
  Config override dir:     AIHELP_CONFIG_DIR=/path

MODEL WORKFLOW
  List callable models:    aihelp --list-models
  Switch default model:    aihelp --model <ID>
  One-off use + persist:   aihelp --model <ID> "question"

STREAMING
  Default behavior:        streaming output is ON
  Disable for one run:     aihelp --no-stream "question"

RETRIES
  Retry transient errors:  aihelp --retries 2 --retry-backoff-ms 500 "question"
  Increase timeout:         aihelp --timeout-secs 180 "question"

MULTI-ENDPOINT
  List endpoints:          aihelp --list-endpoints
  Override for one run:    aihelp --endpoint http://127.0.0.1:1235 "question"
  Strategies:              preferred (default), fallback, parallel_probe, model_route
  Configure in config.toml [[endpoints]] sections or via --setup.

MCP WORKFLOW
  Enable per-run:          aihelp --mcp "question"
  Disable per-run:         aihelp --no-mcp "question"
  Setup auto-detect:       aihelp --setup (scans local MCP HTTP endpoints)

EXAMPLES
  aihelp "Hello can you hear me?"
  ls | aihelp "what is in this directory?"
  cat script.sh | aihelp "what does this script do?"

TROUBLESHOOT
  LM Studio models:        curl <endpoint>/v1/models
  Endpoint override:       aihelp --endpoint http://127.0.0.1:1234 "question"
  Timeout tuning:          aihelp --timeout-secs 180 --retries 3 "question"
  MCP policy override:     aihelp --mcp-policy allow_list --mcp "question"
"#;

#[derive(Debug, Parser, Clone)]
#[command(
    name = "aihelp",
    version,
    about = "CLI helper for LM Studio + optional MCP tools",
    after_help = HELP_MANPAGE
)]
pub struct Cli {
    #[arg(
        value_name = "QUESTION",
        required_unless_present_any = ["list_models", "list_flags", "list_endpoints", "model", "setup", "mcp"]
    )]
    pub question: Vec<String>,

    #[arg(long)]
    pub endpoint: Option<String>,

    #[arg(long = "api-key")]
    pub api_key: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub stream: bool,

    #[arg(long = "no-stream", conflicts_with = "stream")]
    pub no_stream: bool,

    #[arg(long = "max-stdin-bytes")]
    pub max_stdin_bytes: Option<usize>,

    #[arg(long = "timeout-secs")]
    pub timeout_secs: Option<u64>,

    #[arg(long = "retries")]
    pub retries: Option<usize>,

    #[arg(long = "retry-backoff-ms")]
    pub retry_backoff_ms: Option<u64>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub quiet: bool,

    #[arg(long = "print-model")]
    pub print_model: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long, conflicts_with = "no_mcp")]
    pub mcp: bool,

    #[arg(long = "no-mcp", conflicts_with = "mcp")]
    pub no_mcp: bool,

    #[arg(long = "mcp-policy")]
    pub mcp_policy: Option<McpAllowPolicy>,

    #[arg(long = "mcp-max-tool-calls")]
    pub mcp_max_tool_calls: Option<usize>,

    #[arg(long = "mcp-max-round-trips")]
    pub mcp_max_round_trips: Option<usize>,

    #[arg(long = "list-models", conflicts_with_all = ["list_flags", "list_endpoints"])]
    pub list_models: bool,

    #[arg(long = "list-flags", conflicts_with_all = ["list_models", "list_endpoints"])]
    pub list_flags: bool,

    #[arg(long = "list-endpoints", conflicts_with_all = ["list_models", "list_flags"])]
    pub list_endpoints: bool,

    #[arg(
        long = "debug-stream",
        help = "Print per-token timestamps to stderr for stream diagnostics"
    )]
    pub debug_stream: bool,

    #[arg(long = "setup")]
    pub setup: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveSettings {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub max_stdin_bytes: usize,
    pub timeout_secs: u64,
    pub retry_attempts: usize,
    pub retry_backoff_ms: u64,
    pub mcp_enabled: bool,
    pub mcp_policy: McpAllowPolicy,
    pub mcp_max_tool_calls: usize,
    pub mcp_max_round_trips: usize,
    pub stream: bool,
    pub json: bool,
    pub quiet: bool,
    pub print_model: bool,
    pub dry_run: bool,
}

pub async fn run(cli: Cli) -> Result<()> {
    install_rustls_provider();
    init_tracing(cli.quiet);

    let noninteractive_forced = std::env::var("AIHELP_NONINTERACTIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);

    let stdin_is_tty = std::io::stdin().is_terminal();
    let stdout_is_tty = std::io::stdout().is_terminal();
    let interactive = stdin_is_tty && stdout_is_tty;

    if cli.list_flags {
        print_available_flags(cli.json)?;
        return Ok(());
    }

    if cli.list_endpoints {
        return run_list_endpoints(&cli).await;
    }

    if cli.list_models {
        return run_list_models(&cli).await;
    }

    if cli.setup {
        if noninteractive_forced || !interactive {
            anyhow::bail!(
                "--setup requires an interactive terminal and AIHELP_NONINTERACTIVE must not be 1"
            );
        }

        let existing = load_existing_config_or_default()?;
        let updated = run_setup_wizard(Some(existing), cli.quiet)
            .await
            .context("setup failed")?;

        if cli.json {
            let sanitized = config::sanitized_for_display(&updated);
            println!("{}", serde_json::to_string_pretty(&sanitized)?);
        } else {
            println!("Setup complete.");
        }
        return Ok(());
    }

    let mut config = load_runtime_config(&cli, interactive, noninteractive_forced)
        .await
        .context("failed to load configuration")?;

    let mut settings = resolve_settings(&cli, &config);
    let mut effective_mcp_enabled = settings.mcp_enabled;

    let resolved = select_endpoint(
        cli.endpoint.as_deref(),
        &config.resolved_endpoints(),
        config.endpoint_strategy,
        &settings.model,
        &config.model_routing,
    )
    .await?;
    settings.endpoint = resolved.url.clone();
    settings.api_key = resolved.api_key.clone();

    if effective_mcp_enabled && config.mcp.servers.is_empty() {
        if !settings.quiet {
            eprintln!(
                "MCP is enabled but no MCP servers are configured. Continuing without MCP tools for this run. Add MCP servers with `aihelp --setup`."
            );
        }
        effective_mcp_enabled = false;
    }

    if settings.print_model && !settings.quiet {
        eprintln!("model: {}", settings.model);
    }

    let client = OpenAiClient::new(
        resolved.url.clone(),
        resolved.api_key.clone(),
        settings.timeout_secs,
        settings.retry_attempts,
        settings.retry_backoff_ms,
    )?;

    if is_model_switch_only(&cli) {
        if !settings.dry_run {
            client
                .verify_model_presence(&settings.model)
                .await
                .context("model verification failed")?;
        }

        let outcome = persist_model_selection(&cli, &mut config)?;
        emit_model_switch_message(&settings, outcome, cli.json, cli.quiet)?;
        return Ok(());
    }

    let stdin_context = read_stdin_context(settings.max_stdin_bytes).await?;
    let question = cli.question.join(" ");

    if question.trim().is_empty() && stdin_context.is_none() && !settings.dry_run {
        anyhow::bail!("no question provided. Pass a question as positional argument(s), or pipe content via stdin.");
    }

    if !settings.dry_run {
        client
            .verify_model_presence(&settings.model)
            .await
            .context(
            "model verification failed. Check that LM Studio is running and the model is loaded.",
        )?;
    }

    if !settings.dry_run {
        let _ = persist_model_selection(&cli, &mut config)?;
    }

    let mcp_backend = if effective_mcp_enabled {
        Some(
            RmcpBackend::connect(
                config.mcp.servers.clone(),
                settings.mcp_policy,
                settings.quiet,
            )
            .await
            .context("failed to initialize MCP backend")?,
        )
    } else {
        None
    };

    let agent_opts = AgentRunOptions {
        model: settings.model.clone(),
        stream: settings.stream,
        json: settings.json,
        dry_run: settings.dry_run,
        quiet: settings.quiet,
        mcp_enabled: effective_mcp_enabled,
        mcp_max_tool_calls: settings.mcp_max_tool_calls,
        mcp_max_round_trips: settings.mcp_max_round_trips,
        debug_stream: cli.debug_stream,
    };

    run_agent(
        &client,
        mcp_backend
            .as_ref()
            .map(|b| b as &dyn crate::mcp::McpBackend),
        &question,
        stdin_context.as_ref(),
        &agent_opts,
    )
    .await
}

fn resolve_settings(cli: &Cli, config: &AppConfig) -> EffectiveSettings {
    let endpoint = cli
        .endpoint
        .clone()
        .unwrap_or_else(|| config.endpoint.clone());
    let api_key = cli
        .api_key
        .clone()
        .unwrap_or_else(|| config.api_key.clone().unwrap_or_default());
    let model = cli.model.clone().unwrap_or_else(|| config.model.clone());
    // Cap stdin buffer to 50 MiB to prevent OOM on unreasonable values.
    const MAX_STDIN_CAP: usize = 50 * 1024 * 1024;
    let max_stdin_bytes = cli
        .max_stdin_bytes
        .unwrap_or(config.max_stdin_bytes)
        .min(MAX_STDIN_CAP);
    let timeout_secs = cli.timeout_secs.unwrap_or(config.timeout_secs).max(1);
    let retry_attempts = cli.retries.unwrap_or(config.retry_attempts);
    let retry_backoff_ms = cli
        .retry_backoff_ms
        .unwrap_or(config.retry_backoff_ms)
        .max(1);
    let stream = if cli.no_stream {
        false
    } else if cli.stream {
        true
    } else {
        config.stream_by_default
    };

    let mcp_enabled = if cli.mcp {
        true
    } else if cli.no_mcp {
        false
    } else {
        config.mcp.enabled_by_default
    };

    let mcp_policy = cli.mcp_policy.unwrap_or(config.mcp.allow_policy);
    let mcp_max_tool_calls = cli
        .mcp_max_tool_calls
        .unwrap_or(config.mcp.max_tool_calls.max(1));
    let mcp_max_round_trips = cli
        .mcp_max_round_trips
        .unwrap_or(config.mcp.max_round_trips.max(1));

    EffectiveSettings {
        endpoint,
        api_key,
        model,
        max_stdin_bytes,
        timeout_secs,
        retry_attempts,
        retry_backoff_ms,
        mcp_enabled,
        mcp_policy,
        mcp_max_tool_calls,
        mcp_max_round_trips,
        stream,
        json: cli.json,
        quiet: cli.quiet,
        print_model: cli.print_model,
        dry_run: cli.dry_run,
    }
}

fn init_tracing(quiet: bool) {
    let filter = if quiet {
        EnvFilter::new("warn")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn load_runtime_config(
    cli: &Cli,
    interactive: bool,
    noninteractive_forced: bool,
) -> Result<AppConfig> {
    let config_path = config::config_file_path()?;
    if config_path.exists() {
        return config::load_config(&config_path).context("failed to load existing config");
    }

    if interactive && !noninteractive_forced && !is_model_switch_only(cli) {
        return run_setup_wizard(None, cli.quiet)
            .await
            .context("first-run setup failed");
    }

    config::load_or_init_config(false, true).context("failed to initialize config")
}

fn is_model_switch_only(cli: &Cli) -> bool {
    cli.model.is_some() && cli.question.is_empty()
}

async fn run_list_models(cli: &Cli) -> Result<()> {
    let config = load_existing_config_or_default()?;
    let settings = resolve_settings(cli, &config);

    let resolved = select_endpoint(
        cli.endpoint.as_deref(),
        &config.resolved_endpoints(),
        config.endpoint_strategy,
        &settings.model,
        &config.model_routing,
    )
    .await?;

    let client = OpenAiClient::new(
        resolved.url.clone(),
        resolved.api_key.clone(),
        settings.timeout_secs,
        settings.retry_attempts,
        settings.retry_backoff_ms,
    )?;

    let mut models = client
        .list_models()
        .await
        .context("failed to list models from /v1/models")?;
    models.sort();

    if settings.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "endpoint": resolved.url,
                "selected_model": settings.model,
                "models": models
            }))?
        );
        return Ok(());
    }

    println!("Models available at {}:", resolved.url);
    for model in &models {
        if model == &settings.model {
            println!("* {} (selected)", model);
        } else {
            println!("* {}", model);
        }
    }

    if models.is_empty() {
        println!("(no models returned by endpoint)");
    }

    if !settings.quiet && std::io::stdout().is_terminal() {
        eprintln!("Use --model <ID> to select a model for a run.");
    }

    Ok(())
}

async fn run_list_endpoints(cli: &Cli) -> Result<()> {
    let config = load_existing_config_or_default()?;
    let endpoints = config.resolved_endpoints();
    let statuses = crate::endpoint::list_endpoint_status(&endpoints).await;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "strategy": format!("{}", config.endpoint_strategy),
                "endpoints": statuses.iter().map(|s| json!({
                    "label": s.label,
                    "url": s.url,
                    "priority": s.priority,
                    "reachable": s.reachable,
                    "models": s.models,
                })).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    println!("Endpoints (strategy: {}):", config.endpoint_strategy);
    for s in &statuses {
        let status = if s.reachable {
            "\u{2713} reachable"
        } else {
            "\u{2717} unreachable"
        };
        println!(
            "  {} [priority={}] {} ({})",
            s.label, s.priority, s.url, status
        );
        if !s.models.is_empty() {
            for m in &s.models {
                println!("    - {}", m);
            }
        }
    }
    Ok(())
}

fn load_existing_config_or_default() -> Result<AppConfig> {
    let path = config::config_file_path()?;
    if path.exists() {
        return config::load_config(&path).context("failed to load existing config");
    }
    Ok(AppConfig::default())
}

#[derive(Debug, Clone, Serialize)]
struct FlagDescriptor {
    flag: &'static str,
    description: &'static str,
}

fn print_available_flags(as_json: bool) -> Result<()> {
    let flags = vec![
        FlagDescriptor {
            flag: "--help",
            description: "Show built-in clap help with all options.",
        },
        FlagDescriptor {
            flag: "--list-flags",
            description: "Show a curated list of useful aihelp flags.",
        },
        FlagDescriptor {
            flag: "--list-models",
            description: "List callable model IDs from <endpoint>/v1/models.",
        },
        FlagDescriptor {
            flag: "--list-endpoints",
            description: "List configured endpoints and their reachability status.",
        },
        FlagDescriptor {
            flag: "--setup",
            description: "Run interactive setup and persist endpoint/model/MCP defaults.",
        },
        FlagDescriptor {
            flag: "--model <ID>",
            description: "Set default model in config and use it for this run.",
        },
        FlagDescriptor {
            flag: "--print-model",
            description: "Print selected model to stderr before request.",
        },
        FlagDescriptor {
            flag: "--endpoint <URL>",
            description: "Override OpenAI-compatible LM Studio base URL.",
        },
        FlagDescriptor {
            flag: "--api-key <KEY>",
            description: "Optional Authorization bearer token.",
        },
        FlagDescriptor {
            flag: "--stream",
            description: "Enable streaming output (enabled by default).",
        },
        FlagDescriptor {
            flag: "--no-stream",
            description: "Disable streaming output for this run.",
        },
        FlagDescriptor {
            flag: "--json",
            description: "Emit JSON (or NDJSON for streaming).",
        },
        FlagDescriptor {
            flag: "--timeout-secs <N>",
            description: "Set request timeout in seconds (default 120).",
        },
        FlagDescriptor {
            flag: "--retries <N>",
            description: "Retry transient LM Studio failures N times (default 2).",
        },
        FlagDescriptor {
            flag: "--retry-backoff-ms <N>",
            description: "Base retry backoff in milliseconds (default 500).",
        },
        FlagDescriptor {
            flag: "--dry-run",
            description: "Print request payloads without calling LM Studio.",
        },
        FlagDescriptor {
            flag: "--mcp / --no-mcp",
            description: "Enable or disable MCP tools for a single run.",
        },
        FlagDescriptor {
            flag: "--mcp-policy <read_only|allow_list|all>",
            description: "Override MCP tool allow policy.",
        },
        FlagDescriptor {
            flag: "--debug-stream",
            description: "Print per-token timestamps to stderr for stream diagnostics.",
        },
    ];

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "flags": flags }))?
        );
        return Ok(());
    }

    println!("aihelp flag reference:");
    for item in flags {
        println!("{:40} {}", item.flag, item.description);
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ModelSwitchOutcome {
    previous: String,
    current: String,
    updated: bool,
    config_path: PathBuf,
}

fn persist_model_selection(cli: &Cli, config: &mut AppConfig) -> Result<ModelSwitchOutcome> {
    let requested = cli
        .model
        .as_ref()
        .cloned()
        .unwrap_or_else(|| config.model.clone());
    let previous = config.model.clone();
    let updated = previous != requested;

    if updated {
        config.model = requested.clone();
        let config_path = config::config_file_path()?;
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory: {}", parent.display())
            })?;
        }
        config::save_config(&config_path, config).with_context(|| {
            format!(
                "failed to persist selected model to {}",
                config_path.display()
            )
        })?;

        return Ok(ModelSwitchOutcome {
            previous,
            current: requested,
            updated,
            config_path,
        });
    }

    let config_path = config::config_file_path()?;
    Ok(ModelSwitchOutcome {
        previous,
        current: requested,
        updated,
        config_path,
    })
}

fn emit_model_switch_message(
    settings: &EffectiveSettings,
    outcome: ModelSwitchOutcome,
    as_json: bool,
    quiet: bool,
) -> Result<()> {
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "selected_model": settings.model,
                "updated": outcome.updated,
                "previous_model": outcome.previous,
                "config_path": outcome.config_path,
            }))?
        );
        return Ok(());
    }

    if outcome.updated {
        println!(
            "Default model switched from '{}' to '{}'.",
            outcome.previous, outcome.current
        );
    } else {
        println!("Default model already set to '{}'.", outcome.current);
    }

    if !quiet {
        eprintln!("Config updated at {}", outcome.config_path.display());
    }

    Ok(())
}
