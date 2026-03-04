use std::collections::HashSet;
use std::io::{self, Write};
use std::net::{IpAddr, UdpSocket};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::future::join_all;
use reqwest::StatusCode;

use crate::client::OpenAiClient;
use crate::config::{self, AppConfig, McpServerConfig, DEFAULT_ENDPOINT, DEFAULT_MODEL};

const MCP_COMMON_PORTS: [u16; 8] = [7000, 7001, 7002, 7003, 8000, 8080, 8081, 9000];

pub async fn run_setup_wizard(existing: Option<AppConfig>, quiet: bool) -> Result<AppConfig> {
    let mut config = existing.unwrap_or_default();
    let config_path = config::config_file_path()?;

    if !quiet {
        eprintln!("Starting aihelp setup wizard...");
    }

    let detected_endpoints = detect_lm_studio_endpoints(1).await;
    if !quiet && !detected_endpoints.is_empty() {
        eprintln!("Detected LM Studio endpoints:");
        for endpoint in &detected_endpoints {
            eprintln!("  - {endpoint}");
        }
    }

    let endpoint_default = detected_endpoints
        .first()
        .cloned()
        .unwrap_or_else(|| config.endpoint.clone());
    config.endpoint = prompt_with_default("LM Studio endpoint", &endpoint_default)?;

    let model_timeout = config.timeout_secs.clamp(1, 5);
    let models = fetch_models_for_setup(
        &config.endpoint,
        config.api_key.clone().unwrap_or_default(),
        model_timeout,
    )
    .await;

    if let Err(err) = &models {
        if !quiet {
            eprintln!(
                "Could not query models at {} ({err}). You can still set a model manually.",
                config.endpoint
            );
        }
    }

    let mut model_list = models.unwrap_or_default();
    model_list.sort();
    model_list.dedup();

    if !quiet && !model_list.is_empty() {
        eprintln!("Models returned by endpoint:");
        for model in &model_list {
            eprintln!("  - {model}");
        }
    }

    let suggested_model = pick_suggested_model(&config.model, &model_list);
    config.model = prompt_with_default("Default model", &suggested_model)?;

    config.mcp.enabled_by_default =
        prompt_yes_no("Enable MCP server tools by default? (y/N): ", false)?;

    if config.mcp.enabled_by_default {
        let should_scan = prompt_yes_no("Auto-detect local MCP HTTP servers now? (Y/n): ", true)?;
        if should_scan {
            let detected_mcp = detect_mcp_http_endpoints(400).await;
            if detected_mcp.is_empty() {
                if !quiet {
                    eprintln!("No MCP HTTP endpoints detected on common localhost ports.");
                }
            } else {
                if !quiet {
                    eprintln!("Detected MCP endpoints:");
                    for endpoint in &detected_mcp {
                        eprintln!("  - {endpoint}");
                    }
                }

                if prompt_yes_no("Add detected MCP endpoints to config? (Y/n): ", true)? {
                    add_detected_mcp_servers(&mut config, &detected_mcp);
                }
            }
        }
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }
    config::save_config(&config_path, &config)
        .with_context(|| format!("failed to write setup config to {}", config_path.display()))?;

    if config.mcp.enabled_by_default {
        eprintln!("MCP enabled by default. To disable MCP for a single run: `aihelp --no-mcp ...`");
    } else {
        eprintln!("MCP disabled by default. To enable MCP for a single run: `aihelp --mcp ...`");
    }
    eprintln!("Override anytime with `--mcp` or `--no-mcp`.");
    if !quiet {
        eprintln!("Setup complete. Re-run anytime with `aihelp --setup`.");
        eprintln!("Config saved at {}", config_path.display());
    }

    Ok(config)
}

pub async fn detect_lm_studio_endpoints(timeout_secs: u64) -> Vec<String> {
    find_reachable_lm_studio(lm_studio_candidates(), timeout_secs).await
}

pub async fn detect_mcp_http_endpoints(timeout_millis: u64) -> Vec<String> {
    find_reachable_mcp(mcp_http_candidates(), timeout_millis).await
}

pub async fn find_reachable_lm_studio(candidates: Vec<String>, timeout_secs: u64) -> Vec<String> {
    let checks = candidates.into_iter().map(|candidate| async move {
        if probe_lmstudio_endpoint(&candidate, timeout_secs).await {
            Some(candidate)
        } else {
            None
        }
    });

    join_all(checks).await.into_iter().flatten().collect()
}

pub async fn find_reachable_mcp(candidates: Vec<String>, timeout_millis: u64) -> Vec<String> {
    let checks = candidates.into_iter().map(|candidate| async move {
        if probe_mcp_endpoint(&candidate, timeout_millis).await {
            Some(candidate)
        } else {
            None
        }
    });

    join_all(checks).await.into_iter().flatten().collect()
}

fn lm_studio_candidates() -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    push_unique(&mut out, &mut seen, DEFAULT_ENDPOINT.to_string());
    push_unique(&mut out, &mut seen, "http://127.0.0.1:1234".to_string());
    push_unique(&mut out, &mut seen, "http://localhost:1234".to_string());

    for ip in local_ipv4_strings() {
        push_unique(&mut out, &mut seen, format!("http://{ip}:1234"));
    }

    out
}

fn mcp_http_candidates() -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
    hosts.extend(local_ipv4_strings());

    for host in hosts {
        for port in MCP_COMMON_PORTS {
            push_unique(&mut out, &mut seen, format!("http://{host}:{port}/mcp"));
        }
    }

    out
}

async fn fetch_models_for_setup(
    endpoint: &str,
    api_key: String,
    timeout_secs: u64,
) -> Result<Vec<String>> {
    let client = OpenAiClient::new(endpoint.to_string(), api_key, timeout_secs, 1, 200)?;
    client.list_models().await
}

async fn probe_lmstudio_endpoint(endpoint: &str, timeout_secs: u64) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let url = format!("{}/v1/models", endpoint.trim_end_matches('/'));
    match client.get(url).send().await {
        Ok(resp) => {
            resp.status().is_success()
                || resp.status() == StatusCode::UNAUTHORIZED
                || resp.status() == StatusCode::FORBIDDEN
        }
        Err(_) => false,
    }
}

async fn probe_mcp_endpoint(endpoint: &str, timeout_millis: u64) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_millis.max(50)))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    match client.get(endpoint).send().await {
        Ok(resp) => matches!(
            resp.status(),
            StatusCode::OK
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::METHOD_NOT_ALLOWED
                | StatusCode::BAD_REQUEST
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
        ),
        Err(_) => false,
    }
}

fn pick_suggested_model(current: &str, available: &[String]) -> String {
    if available.iter().any(|id| id == current) {
        return current.to_string();
    }
    if available.iter().any(|id| id == DEFAULT_MODEL) {
        return DEFAULT_MODEL.to_string();
    }
    available
        .first()
        .cloned()
        .unwrap_or_else(|| current.to_string())
}

fn add_detected_mcp_servers(config: &mut AppConfig, endpoints: &[String]) {
    let mut existing_endpoints = HashSet::new();
    let mut existing_labels = HashSet::new();

    for server in &config.mcp.servers {
        match server {
            McpServerConfig::Http {
                label, endpoint, ..
            } => {
                existing_labels.insert(label.clone());
                existing_endpoints.insert(endpoint.clone());
            }
            McpServerConfig::Stdio { label, .. } => {
                existing_labels.insert(label.clone());
            }
        }
    }

    let mut next_label_idx = 1usize;
    for endpoint in endpoints {
        if existing_endpoints.contains(endpoint) {
            continue;
        }

        let label = loop {
            let candidate = format!("auto_http_{next_label_idx}");
            next_label_idx += 1;
            if !existing_labels.contains(&candidate) {
                break candidate;
            }
        };

        existing_labels.insert(label.clone());
        existing_endpoints.insert(endpoint.clone());
        config.mcp.servers.push(McpServerConfig::Http {
            label,
            endpoint: endpoint.clone(),
            allowed_tools: Vec::new(),
            headers: Default::default(),
        });
    }
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    eprint!("{label} [{default}]: ");
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read prompt input")?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(default.to_string());
    }
    Ok(trimmed.to_string())
}

fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool> {
    eprint!("{prompt}");
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read yes/no prompt input")?;
    let trimmed = input.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(default);
    }
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

fn local_ipv4_strings() -> Vec<String> {
    let mut out = Vec::new();

    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        let _ = socket.connect("8.8.8.8:80");
        if let Ok(addr) = socket.local_addr() {
            if let IpAddr::V4(ipv4) = addr.ip() {
                if !ipv4.is_loopback() {
                    out.push(ipv4.to_string());
                }
            }
        }
    }

    out
}

fn push_unique(out: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if seen.insert(value.clone()) {
        out.push(value);
    }
}
