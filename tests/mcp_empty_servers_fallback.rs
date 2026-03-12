mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[tokio::test]
#[serial]
async fn mcp_enabled_with_no_servers_falls_back_to_non_mcp() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    let config_dir = TempDir::new().expect("tempdir");

    let config = r#"
endpoint = "http://127.0.0.1:1234"
model = "openai/gpt-oss-20b"
max_stdin_bytes = 200000
timeout_secs = 120
retry_attempts = 2
retry_backoff_ms = 100
stream_by_default = true

[mcp]
enabled_by_default = true
allow_policy = "read_only"
max_tool_calls = 8
max_round_trips = 6
servers = []
"#;
    std::fs::write(config_dir.path().join("config.toml"), config).expect("write config");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}]
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-mcp-fallback",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "fallback ok"},
                    "finish_reason": "stop"
                }
            ]
        })))
        .mount(&server)
        .await;

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("explain this")
        .assert()
        .success()
        .stdout(contains("fallback ok"))
        .stderr(contains("MCP is enabled but no MCP servers are configured"));
}
