mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

#[derive(Clone)]
struct DelayedThenSuccess {
    counter: Arc<AtomicUsize>,
    delay_ms: u64,
    first_body: serde_json::Value,
    success_body: serde_json::Value,
}

impl Respond for DelayedThenSuccess {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let step = self.counter.fetch_add(1, Ordering::SeqCst);
        if step == 0 {
            return ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(self.delay_ms))
                .set_body_json(self.first_body.clone());
        }
        ResponseTemplate::new(200).set_body_json(self.success_body.clone())
    }
}

#[derive(Clone)]
struct AlwaysDelayed {
    delay_ms: u64,
    body: serde_json::Value,
}

impl Respond for AlwaysDelayed {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_delay(Duration::from_millis(self.delay_ms))
            .set_body_json(self.body.clone())
    }
}

#[tokio::test]
#[serial]
async fn chat_completion_retries_after_timeout_and_succeeds() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    let config_dir = TempDir::new().expect("tempdir");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}]
        })))
        .mount(&server)
        .await;

    let attempts = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(DelayedThenSuccess {
            counter: attempts.clone(),
            delay_ms: 1500,
            first_body: serde_json::json!({
                "id": "chatcmpl-delayed",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "too slow"},
                    "finish_reason": "stop"
                }]
            }),
            success_body: serde_json::json!({
                "id": "chatcmpl-success",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "retry recovered"},
                    "finish_reason": "stop"
                }]
            }),
        })
        .mount(&server)
        .await;

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--no-stream")
        .arg("--timeout-secs")
        .arg("1")
        .arg("--retries")
        .arg("1")
        .arg("--retry-backoff-ms")
        .arg("10")
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("retry recovered"));

    assert!(attempts.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
#[serial]
async fn model_listing_retries_after_timeout_and_succeeds() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    let config_dir = TempDir::new().expect("tempdir");

    let model_attempts = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(DelayedThenSuccess {
            counter: model_attempts.clone(),
            delay_ms: 1500,
            first_body: serde_json::json!({
                "data": [{"id": "openai/gpt-oss-20b"}]
            }),
            success_body: serde_json::json!({
                "data": [{"id": "openai/gpt-oss-20b"}]
            }),
        })
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-model-retry",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "model retry worked"},
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--no-stream")
        .arg("--timeout-secs")
        .arg("1")
        .arg("--retries")
        .arg("1")
        .arg("--retry-backoff-ms")
        .arg("10")
        .arg("hello")
        .assert()
        .success()
        .stdout(contains("model retry worked"));

    assert!(model_attempts.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
#[serial]
async fn retry_exhaustion_error_is_actionable() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };
    let config_dir = TempDir::new().expect("tempdir");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}]
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(AlwaysDelayed {
            delay_ms: 1500,
            body: serde_json::json!({
                "id": "chatcmpl-timeout",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "still too slow"},
                    "finish_reason": "stop"
                }]
            }),
        })
        .mount(&server)
        .await;

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--no-stream")
        .arg("--timeout-secs")
        .arg("1")
        .arg("--retries")
        .arg("1")
        .arg("--retry-backoff-ms")
        .arg("10")
        .arg("hello")
        .assert()
        .failure()
        .stderr(contains("after 2 attempts"))
        .stderr(contains("--timeout-secs"))
        .stderr(contains("--retries"));
}
