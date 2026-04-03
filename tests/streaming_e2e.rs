mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn models_ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "data": [{"id": "openai/gpt-oss-20b"}]
    }))
}

fn sse_body_lf() -> String {
    let chunk1 = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": "hello "}}]
    });
    let chunk2 = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": "world"}, "finish_reason": "stop"}]
    });
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", chunk1, chunk2)
}

fn sse_body_crlf() -> String {
    let chunk1 = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": "hello "}}]
    });
    let chunk2 = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": "world"}, "finish_reason": "stop"}]
    });
    format!(
        "data: {}\r\n\r\ndata: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        chunk1, chunk2
    )
}

#[tokio::test]
#[serial]
async fn sse_multi_chunk_concatenated() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(models_ok())
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_body_lf(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("hello world"));
}

#[tokio::test]
#[serial]
async fn sse_crlf_delimiters_work() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(models_ok())
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_body_crlf(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("hello world"));
}

#[tokio::test]
#[serial]
async fn non_sse_content_type_falls_back_to_json() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(models_ok())
        .mount(&server)
        .await;

    // Server ignores stream=true and returns regular JSON with application/json.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-fallback",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "fallback answer"},
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    // Default mode is streaming, so the client should detect non-SSE and fall back.
    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("fallback answer"));
}

#[tokio::test]
#[serial]
async fn sse_ends_without_done_marker_salvages() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(models_ok())
        .mount(&server)
        .await;

    // SSE body without [DONE] — server just stops sending.
    let chunk = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": "salvaged"}, "finish_reason": "stop"}]
    });
    let body = format!("data: {}\n\n", chunk);

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
        )
        .mount(&server)
        .await;

    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("salvaged"));
}
