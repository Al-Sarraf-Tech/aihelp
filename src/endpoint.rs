#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Result};
use futures_util::future::join_all;
use reqwest::StatusCode;

use crate::client::ModelsResponse;
use crate::config::{EndpointConfig, EndpointStrategy};

/// Result of endpoint selection — the chosen endpoint's URL and API key.
#[derive(Debug, Clone)]
pub struct ResolvedEndpoint {
    pub label: String,
    pub url: String,
    pub api_key: String,
}

/// Reachability status for a single endpoint.
#[derive(Debug, Clone)]
pub struct EndpointStatus {
    pub label: String,
    pub url: String,
    pub priority: u8,
    pub reachable: bool,
    /// Populated with model IDs when the endpoint is reachable.
    pub models: Vec<String>,
}

/// Select an endpoint based on strategy, model, and routing table.
///
/// - `cli_endpoint`: if the user passed `--endpoint` on CLI (could be a label or URL)
/// - `endpoints`: the resolved endpoint list from config
/// - `strategy`: the selection strategy
/// - `model`: the model being used (for `ModelRoute` strategy)
/// - `model_routing`: model_id -> endpoint_label map
pub async fn select_endpoint(
    cli_endpoint: Option<&str>,
    endpoints: &[EndpointConfig],
    strategy: EndpointStrategy,
    model: &str,
    model_routing: &HashMap<String, String>,
) -> Result<ResolvedEndpoint> {
    // 1. CLI override: --endpoint flag takes precedence over everything.
    if let Some(cli) = cli_endpoint {
        // Check if it matches a configured label first.
        if let Some(ep) = endpoints.iter().find(|e| e.label == cli) {
            return Ok(ResolvedEndpoint {
                label: ep.label.clone(),
                url: ep.url.clone(),
                api_key: ep.api_key.clone().unwrap_or_default(),
            });
        }
        // Otherwise treat as a raw URL.
        return Ok(ResolvedEndpoint {
            label: cli.to_string(),
            url: cli.to_string(),
            api_key: String::new(),
        });
    }

    if endpoints.is_empty() {
        bail!("no endpoints configured — add at least one [[endpoints]] entry in config.toml or pass --endpoint <URL>");
    }

    // 2. ModelRoute: look up model in routing table.
    if strategy == EndpointStrategy::ModelRoute {
        if let Some(target_label) = model_routing.get(model) {
            if let Some(ep) = endpoints.iter().find(|e| &e.label == target_label) {
                tracing::debug!(
                    model = model,
                    label = %ep.label,
                    "model_routing matched, using endpoint"
                );
                return Ok(ResolvedEndpoint {
                    label: ep.label.clone(),
                    url: ep.url.clone(),
                    api_key: ep.api_key.clone().unwrap_or_default(),
                });
            }
            tracing::warn!(
                model = model,
                target_label = %target_label,
                "model_routing label '{}' not found in endpoints, falling through to Preferred",
                target_label
            );
        }
        // Fall through to Preferred behaviour when no routing match.
    }

    // 3. Sort endpoints by priority (lower number = higher priority).
    let mut sorted: Vec<&EndpointConfig> = endpoints.iter().collect();
    sorted.sort_by_key(|e| e.priority);

    match strategy {
        EndpointStrategy::Preferred | EndpointStrategy::Fallback | EndpointStrategy::ModelRoute => {
            // Try each in priority order, return first reachable.
            for ep in &sorted {
                let alive = probe_endpoint(&ep.url, 2).await;
                tracing::debug!(label = %ep.label, url = %ep.url, alive = alive, "probe result");
                if alive {
                    return Ok(ResolvedEndpoint {
                        label: ep.label.clone(),
                        url: ep.url.clone(),
                        api_key: ep.api_key.clone().unwrap_or_default(),
                    });
                }
            }
        }
        EndpointStrategy::ParallelProbe => {
            // Probe all in parallel, pick first reachable (by priority order).
            let futures: Vec<_> = sorted
                .iter()
                .map(|ep| {
                    let url = ep.url.clone();
                    async move { probe_endpoint(&url, 2).await }
                })
                .collect();
            let results = join_all(futures).await;

            for (ep, alive) in sorted.iter().zip(results.iter()) {
                tracing::debug!(label = %ep.label, url = %ep.url, alive = alive, "probe result");
                if *alive {
                    return Ok(ResolvedEndpoint {
                        label: ep.label.clone(),
                        url: ep.url.clone(),
                        api_key: ep.api_key.clone().unwrap_or_default(),
                    });
                }
            }
        }
    }

    // No endpoint reachable — build a helpful error message.
    let tried: Vec<String> = sorted
        .iter()
        .map(|ep| format!("  - {} ({})", ep.label, ep.url))
        .collect();
    bail!(
        "no reachable endpoint found. Tried:\n{}\n\nEnsure at least one endpoint is running, or pass --endpoint <URL> to override.",
        tried.join("\n")
    );
}

/// Probe an endpoint by hitting `/v1/models` with a short timeout.
///
/// Returns `true` if the endpoint responds with 200, 401, or 403
/// (all indicate the service is alive).
pub async fn probe_endpoint(url: &str, timeout_secs: u64) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let probe_url = format!("{}/v1/models", url.trim_end_matches('/'));
    match client.get(&probe_url).send().await {
        Ok(resp) => {
            resp.status().is_success()
                || resp.status() == StatusCode::UNAUTHORIZED
                || resp.status() == StatusCode::FORBIDDEN
        }
        Err(_) => false,
    }
}

/// List all endpoints with their reachability status.
///
/// Used for `aihelp --list-endpoints`. Probes all endpoints in parallel.
pub async fn list_endpoint_status(endpoints: &[EndpointConfig]) -> Vec<EndpointStatus> {
    let futures: Vec<_> = endpoints
        .iter()
        .map(|ep| {
            let url = ep.url.clone();
            let api_key = ep.api_key.clone().unwrap_or_default();
            let label = ep.label.clone();
            let priority = ep.priority;
            async move {
                let reachable = probe_endpoint(&url, 2).await;
                let models = if reachable {
                    fetch_models_quiet(&url, &api_key, 3).await
                } else {
                    Vec::new()
                };
                EndpointStatus {
                    label,
                    url,
                    priority,
                    reachable,
                    models,
                }
            }
        })
        .collect();

    join_all(futures).await
}

/// Fetch the model list from an endpoint, returning an empty vec on any failure.
async fn fetch_models_quiet(url: &str, api_key: &str, timeout_secs: u64) -> Vec<String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let models_url = format!("{}/v1/models", url.trim_end_matches('/'));
    let mut req = client.get(&models_url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let resp = match req.send().await {
        Ok(r) if r.status() == StatusCode::OK => r,
        _ => return Vec::new(),
    };

    match resp.json::<ModelsResponse>().await {
        Ok(parsed) => parsed.data.into_iter().map(|m| m.id).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EndpointConfig, EndpointStrategy};

    fn make_endpoint(label: &str, url: &str, priority: u8) -> EndpointConfig {
        EndpointConfig {
            label: label.to_string(),
            url: url.to_string(),
            api_key: None,
            priority,
        }
    }

    #[tokio::test]
    async fn cli_endpoint_matches_label() {
        let eps = vec![
            make_endpoint("local", "http://127.0.0.1:1234", 0),
            make_endpoint("remote", "http://10.0.0.1:1234", 1),
        ];
        let routing = HashMap::new();

        let resolved = select_endpoint(
            Some("remote"),
            &eps,
            EndpointStrategy::Preferred,
            "test-model",
            &routing,
        )
        .await
        .unwrap();

        assert_eq!(resolved.label, "remote");
        assert_eq!(resolved.url, "http://10.0.0.1:1234");
    }

    #[tokio::test]
    async fn cli_endpoint_raw_url() {
        let eps = vec![make_endpoint("local", "http://127.0.0.1:1234", 0)];
        let routing = HashMap::new();

        let resolved = select_endpoint(
            Some("http://custom:9999"),
            &eps,
            EndpointStrategy::Preferred,
            "test-model",
            &routing,
        )
        .await
        .unwrap();

        assert_eq!(resolved.url, "http://custom:9999");
        assert!(resolved.api_key.is_empty());
    }

    #[tokio::test]
    async fn empty_endpoints_errors() {
        let routing = HashMap::new();
        let result = select_endpoint(
            None,
            &[],
            EndpointStrategy::Preferred,
            "test-model",
            &routing,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn no_reachable_endpoints_errors() {
        // Use unreachable addresses so probes fail quickly.
        let eps = vec![
            make_endpoint("bad1", "http://192.0.2.1:1", 0),
            make_endpoint("bad2", "http://192.0.2.2:1", 1),
        ];
        let routing = HashMap::new();

        let result = select_endpoint(
            None,
            &eps,
            EndpointStrategy::Preferred,
            "test-model",
            &routing,
        )
        .await;

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("bad1"));
        assert!(msg.contains("bad2"));
    }
}
