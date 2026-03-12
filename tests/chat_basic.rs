mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[tokio::test]
#[serial]
async fn chat_completion_prints_answer() {
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
            "id": "chatcmpl-basic",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "- item one\\n- item two"
                    },
                    "finish_reason": "stop"
                }
            ]
        })))
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("what is in this directory?")
        .write_stdin("file_a\nfile_b\n")
        .assert()
        .success()
        .stdout(contains("item one"));
}
