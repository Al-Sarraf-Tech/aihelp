mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aihelp::agent::{run_agent, AgentRunOptions};
use aihelp::client::OpenAiClient;
use aihelp::mcp::McpBackend;
use async_trait::async_trait;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

#[derive(Clone)]
struct SequenceResponder {
    counter: Arc<AtomicUsize>,
}

impl Respond for SequenceResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let step = self.counter.fetch_add(1, Ordering::SeqCst);

        match step {
            0 => ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-1",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [
                                {
                                    "id": "call_1",
                                    "type": "function",
                                    "function": {
                                        "name": "mcp_list_tools",
                                        "arguments": "{}"
                                    }
                                }
                            ]
                        },
                        "finish_reason": "tool_calls"
                    }
                ]
            })),
            1 => ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-2",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [
                                {
                                    "id": "call_2",
                                    "type": "function",
                                    "function": {
                                        "name": "mcp_call_tool",
                                        "arguments": "{\"server_label\":\"fake\",\"tool_name\":\"read_file\",\"arguments\":{\"path\":\"README.md\"}}"
                                    }
                                }
                            ]
                        },
                        "finish_reason": "tool_calls"
                    }
                ]
            })),
            _ => ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-3",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "Final answer after tool loop"
                        },
                        "finish_reason": "stop"
                    }
                ]
            })),
        }
    }
}

#[derive(Default)]
struct FakeMcpBackend {
    list_tools_calls: AtomicUsize,
    call_tool_calls: AtomicUsize,
}

#[async_trait]
impl McpBackend for FakeMcpBackend {
    async fn list_tools(
        &self,
        _query: Option<&str>,
        _server_label: Option<&str>,
    ) -> anyhow::Result<Value> {
        self.list_tools_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "tools": [
                {
                    "server_label": "fake",
                    "tool_name": "read_file",
                    "description": "Read a file safely",
                    "json_schema": { "type": "object", "properties": { "path": { "type": "string" } } }
                }
            ]
        }))
    }

    async fn call_tool(
        &self,
        server_label: &str,
        tool_name: &str,
        arguments: Value,
    ) -> anyhow::Result<Value> {
        self.call_tool_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({
            "server_label": server_label,
            "tool_name": tool_name,
            "result": {
                "content": format!("fake_result for {}", arguments)
            }
        }))
    }

    async fn list_resources(&self, _server_label: Option<&str>) -> anyhow::Result<Value> {
        Ok(json!({ "resources": [] }))
    }

    async fn read_resource(&self, server_label: &str, uri: &str) -> anyhow::Result<Value> {
        Ok(json!({
            "server_label": server_label,
            "uri": uri,
            "result": "fake_resource"
        }))
    }
}

#[tokio::test]
async fn mcp_virtual_tool_loop_runs_to_completion() {
    let Some(server) = support::start_mock_server_if_available().await else {
        return;
    };

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(SequenceResponder {
            counter: Arc::new(AtomicUsize::new(0)),
        })
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), String::new(), 10, 0, 10).expect("client");
    let backend = FakeMcpBackend::default();

    let opts = AgentRunOptions {
        model: "openai/gpt-oss-20b".to_string(),
        stream: false,
        json: false,
        dry_run: false,
        quiet: true,
        mcp_enabled: true,
        mcp_max_tool_calls: 8,
        mcp_max_round_trips: 6,
    };

    run_agent(
        &client,
        Some(&backend),
        "find tools and summarize",
        None,
        &opts,
    )
    .await
    .expect("agent run should succeed");

    assert_eq!(backend.list_tools_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.call_tool_calls.load(Ordering::SeqCst), 1);
}
