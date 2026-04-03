use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serial_test::serial;
use tempfile::TempDir;

/// `--dry-run` prints the request payload as JSON without making any API calls.
/// No mock server is needed because the binary must not contact any endpoint.
#[tokio::test]
#[serial]
async fn dry_run_prints_request_payload_without_api_call() {
    let config_dir = TempDir::new().expect("tempdir");

    cargo_bin_cmd!("aihelp")
        .env("AIHELP_CONFIG_DIR", config_dir.path())
        .env("AIHELP_NONINTERACTIVE", "1")
        .arg("--endpoint")
        .arg("http://127.0.0.1:19999") // unreachable; proves no HTTP call is made
        .arg("--no-stream")
        .arg("--dry-run")
        .arg("hello dry run")
        .assert()
        .success()
        .stdout(contains("\"method\": \"POST\""))
        .stdout(contains("/v1/chat/completions"))
        .stdout(contains("hello dry run"));
}
