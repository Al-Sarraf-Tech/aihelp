use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
#[serial]
fn list_flags_works_without_question() {
    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--list-flags")
        .assert()
        .success()
        .stdout(contains("--list-models"))
        .stdout(contains("--model <ID>"))
        .stdout(contains("--retries <N>"))
        .stdout(contains("--retry-backoff-ms <N>"))
        .stdout(contains("--mcp / --no-mcp"));
}

#[tokio::test]
#[serial]
async fn list_models_hits_models_endpoint_only() {
    let server = MockServer::start().await;
    let config_dir = TempDir::new().expect("tempdir");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "openai/gpt-oss-20b"},
                {"id": "my-local-model"}
            ]
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

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--list-models")
        .assert()
        .success()
        .stdout(contains("openai/gpt-oss-20b (selected)"))
        .stdout(contains("my-local-model"));
}

#[tokio::test]
#[serial]
async fn list_models_json_outputs_models_array() {
    let server = MockServer::start().await;
    let config_dir = TempDir::new().expect("tempdir");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg(server.uri())
        .arg("--list-models")
        .arg("--json")
        .assert()
        .success()
        .stdout(contains("\"models\""))
        .stdout(contains("\"openai/gpt-oss-20b\""))
        .stdout(contains("\"selected_model\""));
}
