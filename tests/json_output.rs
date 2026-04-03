mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

/// `--json --no-stream` returns the raw JSON response envelope from the API.
#[tokio::test]
#[serial]
async fn json_flag_returns_raw_api_response() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

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
            "id": "chatcmpl-json-test",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "json response body"
                    },
                    "finish_reason": "stop"
                }
            ]
        })))
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    let output = cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--no-stream")
        .arg("--json")
        .arg("hello json")
        .output()
        .expect("failed to run aihelp");

    assert!(output.status.success(), "exit code was non-zero");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(parsed["id"], "chatcmpl-json-test");
    assert_eq!(
        parsed["choices"][0]["message"]["content"],
        "json response body"
    );
}
