use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENDPOINT: &str = "http://192.168.50.2:1234";
pub const DEFAULT_MODEL: &str = "openai/gpt-oss-20b";
pub const DEFAULT_MAX_STDIN_BYTES: usize = 200_000;
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_RETRY_ATTEMPTS: usize = 2;
pub const DEFAULT_RETRY_BACKOFF_MS: u64 = 500;
pub const DEFAULT_STREAM_BY_DEFAULT: bool = true;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointStrategy {
    Preferred,
    Fallback,
    RoundRobin,
    ModelRoute,
}

impl Display for EndpointStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preferred => f.write_str("preferred"),
            Self::Fallback => f.write_str("fallback"),
            Self::RoundRobin => f.write_str("round_robin"),
            Self::ModelRoute => f.write_str("model_route"),
        }
    }
}

impl std::str::FromStr for EndpointStrategy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "preferred" => Ok(Self::Preferred),
            "fallback" => Ok(Self::Fallback),
            "round_robin" => Ok(Self::RoundRobin),
            "model_route" => Ok(Self::ModelRoute),
            _ => bail!("invalid endpoint strategy: {s}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAllowPolicy {
    ReadOnly,
    AllowList,
    All,
}

impl Display for McpAllowPolicy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("read_only"),
            Self::AllowList => f.write_str("allow_list"),
            Self::All => f.write_str("all"),
        }
    }
}

impl std::str::FromStr for McpAllowPolicy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read_only" => Ok(Self::ReadOnly),
            "allow_list" => Ok(Self::AllowList),
            "all" => Ok(Self::All),
            _ => bail!("invalid MCP allow policy: {s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_stdin_bytes")]
    pub max_stdin_bytes: usize,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: usize,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    #[serde(default = "default_stream_by_default")]
    pub stream_by_default: bool,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(default = "default_endpoint_strategy")]
    pub endpoint_strategy: EndpointStrategy,
    #[serde(default)]
    pub model_routing: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: None,
            model: default_model(),
            max_stdin_bytes: default_max_stdin_bytes(),
            timeout_secs: default_timeout_secs(),
            retry_attempts: default_retry_attempts(),
            retry_backoff_ms: default_retry_backoff_ms(),
            stream_by_default: default_stream_by_default(),
            mcp: McpConfig::default(),
            endpoints: Vec::new(),
            endpoint_strategy: default_endpoint_strategy(),
            model_routing: HashMap::new(),
        }
    }
}

impl AppConfig {
    /// Returns the resolved list of endpoints. If `endpoints` vec is non-empty, use it.
    /// Otherwise, synthesize a single endpoint from the legacy `endpoint` + `api_key` fields.
    pub fn resolved_endpoints(&self) -> Vec<EndpointConfig> {
        if !self.endpoints.is_empty() {
            return self.endpoints.clone();
        }
        vec![EndpointConfig {
            label: "default".to_string(),
            url: self.endpoint.clone(),
            api_key: self.api_key.clone(),
            priority: 0,
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled_by_default: bool,
    #[serde(default = "default_allow_policy")]
    pub allow_policy: McpAllowPolicy,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    #[serde(default = "default_max_round_trips")]
    pub max_round_trips: usize,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled_by_default: false,
            allow_policy: default_allow_policy(),
            max_tool_calls: default_max_tool_calls(),
            max_round_trips: default_max_round_trips(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpServerConfig {
    Http {
        label: String,
        endpoint: String,
        #[serde(default)]
        allowed_tools: Vec<String>,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Stdio {
        label: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        allowed_tools: Vec<String>,
    },
}

impl McpServerConfig {
    pub fn label(&self) -> &str {
        match self {
            Self::Http { label, .. } => label,
            Self::Stdio { label, .. } => label,
        }
    }

    pub fn allowed_tools(&self) -> &[String] {
        match self {
            Self::Http { allowed_tools, .. } => allowed_tools,
            Self::Stdio { allowed_tools, .. } => allowed_tools,
        }
    }
}

pub fn sanitized_for_display(config: &AppConfig) -> AppConfig {
    let mut out = config.clone();

    if let Some(api_key) = out.api_key.as_mut() {
        if !api_key.is_empty() {
            *api_key = "***REDACTED***".to_string();
        }
    }

    for ep in &mut out.endpoints {
        if let Some(key) = ep.api_key.as_mut() {
            if !key.is_empty() {
                *key = "***REDACTED***".to_string();
            }
        }
    }

    for server in &mut out.mcp.servers {
        if let McpServerConfig::Http { headers, .. } = server {
            for (name, value) in headers.iter_mut() {
                if is_sensitive_field_name(name) && !value.is_empty() {
                    *value = "***REDACTED***".to_string();
                }
            }
        }
    }

    out
}

pub fn config_dir() -> Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("AIHELP_CONFIG_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let project_dirs = ProjectDirs::from("", "", "aihelp")
        .context("unable to resolve OS-specific config directory")?;
    Ok(project_dirs.config_dir().to_path_buf())
}

pub fn config_file_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn load_or_init_config(interactive: bool, noninteractive_forced: bool) -> Result<AppConfig> {
    let config_path = config_file_path()?;
    if config_path.exists() {
        return load_config(&config_path);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }

    let mut config = AppConfig::default();

    let should_prompt = interactive && !noninteractive_forced;
    if should_prompt {
        eprint!("Enable MCP server tools by default? (y/N): ");
        io::stderr().flush().ok();

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read first-run MCP prompt response")?;

        let choice_yes = matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes");

        config.mcp.enabled_by_default = choice_yes;
        save_config(&config_path, &config)?;

        if choice_yes {
            eprintln!(
                "MCP enabled by default. To disable MCP for a single run: `aihelp --no-mcp ...`"
            );
        } else {
            eprintln!(
                "MCP disabled by default. To enable MCP for a single run: `aihelp --mcp ...`"
            );
        }
        eprintln!("Override anytime with `--mcp` or `--no-mcp`.");
    } else {
        config.mcp.enabled_by_default = false;
        save_config(&config_path, &config)?;
    }

    Ok(config)
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let parsed: AppConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config TOML: {}", path.display()))?;
    Ok(parsed)
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    let raw = toml::to_string_pretty(config).context("failed to serialize config TOML")?;
    fs::write(path, raw)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;
    Ok(())
}

fn default_endpoint() -> String {
    DEFAULT_ENDPOINT.to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_max_stdin_bytes() -> usize {
    DEFAULT_MAX_STDIN_BYTES
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

fn default_retry_attempts() -> usize {
    DEFAULT_RETRY_ATTEMPTS
}

fn default_retry_backoff_ms() -> u64 {
    DEFAULT_RETRY_BACKOFF_MS
}

fn default_stream_by_default() -> bool {
    DEFAULT_STREAM_BY_DEFAULT
}

fn default_priority() -> u8 {
    0
}

fn default_endpoint_strategy() -> EndpointStrategy {
    EndpointStrategy::Preferred
}

fn default_allow_policy() -> McpAllowPolicy {
    McpAllowPolicy::ReadOnly
}

fn default_max_tool_calls() -> usize {
    8
}

fn default_max_round_trips() -> usize {
    6
}

fn is_sensitive_field_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.contains("auth")
        || lowered.contains("token")
        || lowered.contains("key")
        || lowered.contains("secret")
        || lowered.contains("password")
        || lowered.contains("cookie")
}
