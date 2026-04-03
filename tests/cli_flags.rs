mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[test]
fn stream_no_stream_conflict() {
    cargo_bin_cmd!("aihelp")
        .arg("--stream")
        .arg("--no-stream")
        .arg("hello")
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
fn mcp_no_mcp_conflict() {
    cargo_bin_cmd!("aihelp")
        .arg("--mcp")
        .arg("--no-mcp")
        .arg("hello")
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
fn list_models_list_flags_conflict() {
    cargo_bin_cmd!("aihelp")
        .arg("--list-models")
        .arg("--list-flags")
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
#[serial]
fn setup_fails_noninteractive() {
    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--setup")
        .assert()
        .failure()
        .stderr(contains("--setup requires"));
}

#[tokio::test]
#[serial]
async fn dry_run_with_stdin_shows_context() {
    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--dry-run")
        .arg("explain this")
        .write_stdin("stdin_payload_here\n")
        .assert()
        .success()
        .stdout(contains("stdin_payload_here"));
}

#[tokio::test]
#[serial]
async fn print_model_prints_to_stderr() {
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
            "id": "chatcmpl-print",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--print-model")
        .arg("--no-stream")
        .arg("hello")
        .assert()
        .success()
        .stderr(contains("model: openai/gpt-oss-20b"));
}
