mod support;

use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aihelp::config::{AppConfig, EndpointConfig, EndpointStrategy};
use aihelp::endpoint::{list_endpoint_status, select_endpoint};
use std::collections::HashMap;

#[tokio::test]
#[serial]
async fn cli_endpoint_label_matches_configured() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    mount_models_endpoint(&server, vec!["model-a"]).await;

    let endpoints = vec![
        EndpointConfig {
            label: "local".to_string(),
            url: server.uri(),
            api_key: None,
            priority: 0,
        },
        EndpointConfig {
            label: "remote".to_string(),
            url: "http://192.168.50.2:1234".to_string(),
            api_key: None,
            priority: 1,
        },
    ];

    let resolved = select_endpoint(
        Some("local"),
        &endpoints,
        EndpointStrategy::Preferred,
        "model-a",
        &HashMap::new(),
    )
    .await
    .expect("should resolve");

    assert_eq!(resolved.label, "local");
    assert_eq!(resolved.url, server.uri());
}

#[tokio::test]
#[serial]
async fn cli_endpoint_raw_url_used_directly() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    mount_models_endpoint(&server, vec!["model-a"]).await;

    let endpoints = vec![EndpointConfig {
        label: "default".to_string(),
        url: "http://192.168.50.2:1234".to_string(),
        api_key: None,
        priority: 0,
    }];

    let resolved = select_endpoint(
        Some(&server.uri()),
        &endpoints,
        EndpointStrategy::Preferred,
        "model-a",
        &HashMap::new(),
    )
    .await
    .expect("should resolve");

    assert_eq!(resolved.url, server.uri());
}

#[tokio::test]
#[serial]
async fn preferred_strategy_picks_highest_priority_reachable() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    mount_models_endpoint(&server, vec!["model-a"]).await;

    let endpoints = vec![
        EndpointConfig {
            label: "unreachable".to_string(),
            url: "http://127.0.0.1:9".to_string(),
            api_key: None,
            priority: 0,
        },
        EndpointConfig {
            label: "reachable".to_string(),
            url: server.uri(),
            api_key: None,
            priority: 1,
        },
    ];

    let resolved = select_endpoint(
        None,
        &endpoints,
        EndpointStrategy::Preferred,
        "model-a",
        &HashMap::new(),
    )
    .await
    .expect("should resolve");

    assert_eq!(resolved.label, "reachable");
}

#[tokio::test]
#[serial]
async fn model_route_strategy_picks_correct_endpoint() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    mount_models_endpoint(&server, vec!["small-model"]).await;

    let endpoints = vec![
        EndpointConfig {
            label: "remote".to_string(),
            url: "http://127.0.0.1:9".to_string(),
            api_key: None,
            priority: 0,
        },
        EndpointConfig {
            label: "local".to_string(),
            url: server.uri(),
            api_key: None,
            priority: 1,
        },
    ];

    let mut routing = HashMap::new();
    routing.insert("small-model".to_string(), "local".to_string());

    let resolved = select_endpoint(
        None,
        &endpoints,
        EndpointStrategy::ModelRoute,
        "small-model",
        &routing,
    )
    .await
    .expect("should resolve");

    assert_eq!(resolved.label, "local");
}

#[tokio::test]
#[serial]
async fn no_reachable_endpoint_returns_error() {
    let endpoints = vec![
        EndpointConfig {
            label: "bad1".to_string(),
            url: "http://127.0.0.1:9".to_string(),
            api_key: None,
            priority: 0,
        },
        EndpointConfig {
            label: "bad2".to_string(),
            url: "http://127.0.0.1:10".to_string(),
            api_key: None,
            priority: 1,
        },
    ];

    let result = select_endpoint(
        None,
        &endpoints,
        EndpointStrategy::Preferred,
        "any-model",
        &HashMap::new(),
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn list_endpoint_status_shows_reachability() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    mount_models_endpoint(&server, vec!["model-x"]).await;

    let endpoints = vec![
        EndpointConfig {
            label: "good".to_string(),
            url: server.uri(),
            api_key: None,
            priority: 0,
        },
        EndpointConfig {
            label: "bad".to_string(),
            url: "http://127.0.0.1:9".to_string(),
            api_key: None,
            priority: 1,
        },
    ];

    let statuses = list_endpoint_status(&endpoints).await;
    assert_eq!(statuses.len(), 2);

    let good = statuses.iter().find(|s| s.label == "good").unwrap();
    assert!(good.reachable);
    assert!(good.models.contains(&"model-x".to_string()));

    let bad = statuses.iter().find(|s| s.label == "bad").unwrap();
    assert!(!bad.reachable);
}

// Config roundtrip test for the new fields
#[test]
#[serial]
fn config_with_endpoints_roundtrips() {
    let temp = TempDir::new().expect("tempdir");
    std::env::set_var("AIHELP_CONFIG_DIR", temp.path());

    let mut routing = HashMap::new();
    routing.insert("small-model".to_string(), "local".to_string());

    let cfg = AppConfig {
        endpoint: "http://192.168.50.2:1234".to_string(),
        endpoints: vec![
            EndpointConfig {
                label: "local".to_string(),
                url: "http://127.0.0.1:1235".to_string(),
                api_key: None,
                priority: 0,
            },
            EndpointConfig {
                label: "remote".to_string(),
                url: "http://192.168.50.2:1234".to_string(),
                api_key: None,
                priority: 1,
            },
        ],
        endpoint_strategy: EndpointStrategy::Fallback,
        model_routing: routing,
        ..AppConfig::default()
    };

    let path = aihelp::config::config_file_path().expect("path");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    aihelp::config::save_config(&path, &cfg).expect("save");

    let loaded = aihelp::config::load_config(&path).expect("load");
    assert_eq!(loaded.endpoints.len(), 2);
    assert_eq!(loaded.endpoints[0].label, "local");
    assert_eq!(loaded.endpoints[1].label, "remote");
    assert_eq!(loaded.endpoint_strategy, EndpointStrategy::Fallback);
    assert_eq!(loaded.model_routing.get("small-model").unwrap(), "local");

    std::env::remove_var("AIHELP_CONFIG_DIR");
}

// Helper
async fn mount_models_endpoint(server: &MockServer, model_ids: Vec<&str>) {
    let data: Vec<_> = model_ids
        .iter()
        .map(|id| serde_json::json!({"id": id}))
        .collect();
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": data
        })))
        .mount(server)
        .await;
}
