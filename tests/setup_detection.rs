mod support;

use aihelp::setup::{find_reachable_lm_studio, find_reachable_mcp};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[tokio::test]
#[serial]
async fn finds_reachable_lm_studio_endpoint() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "openai/gpt-oss-20b"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let candidates = vec!["http://127.0.0.1:9".to_string(), server.uri()];
    let found = find_reachable_lm_studio(candidates, 1).await;

    assert_eq!(found, vec![server.uri()]);
}

#[tokio::test]
#[serial]
async fn finds_reachable_mcp_endpoint() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("GET"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(405))
        .expect(1)
        .mount(&server)
        .await;

    let good = format!("{}/mcp", server.uri());
    let candidates = vec!["http://127.0.0.1:9/mcp".to_string(), good.clone()];
    let found = find_reachable_mcp(candidates, 300).await;

    assert_eq!(found, vec![good]);
}
