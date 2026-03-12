mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, ResponseTemplate};

#[tokio::test]
#[serial]
async fn model_switch_without_question_updates_config() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}, {"id": "my-local-model"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--model")
        .arg("my-local-model")
        .assert()
        .success()
        .stdout(contains("Default model switched"));

    let config_path = config_dir.path().join("config.toml");
    let raw = std::fs::read_to_string(config_path).expect("config should exist");
    assert!(raw.contains("model = \"my-local-model\""));
}

#[tokio::test]
#[serial]
async fn switched_model_is_used_on_next_run_without_override() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}, {"id": "my-local-model"}]
        })))
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"model\":\"my-local-model\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-switch",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "switched model used"
                    },
                    "finish_reason": "stop"
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--model")
        .arg("my-local-model")
        .assert()
        .success();

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("switched model used"));
}
