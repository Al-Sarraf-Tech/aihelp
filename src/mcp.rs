use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, RoleClient, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::client::{FunctionDefinition, ToolDefinition};
use crate::config::{McpAllowPolicy, McpServerConfig};

#[async_trait]
pub trait McpBackend: Send + Sync {
    async fn list_tools(&self, query: Option<&str>, server_label: Option<&str>) -> Result<Value>;

    async fn call_tool(
        &self,
        server_label: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value>;

    async fn list_resources(&self, server_label: Option<&str>) -> Result<Value>;

    async fn read_resource(&self, server_label: &str, uri: &str) -> Result<Value>;
}

pub struct RmcpBackend {
    policy: McpAllowPolicy,
    servers: Vec<ServerConnection>,
}

struct ServerConnection {
    label: String,
    allowed_tools: Vec<String>,
    peer: Peer<RoleClient>,
    _service: RunningService<RoleClient, ()>,
}

impl RmcpBackend {
    pub async fn connect(
        servers: Vec<McpServerConfig>,
        policy: McpAllowPolicy,
        quiet: bool,
    ) -> Result<Self> {
        let mut out = Vec::new();

        for server in servers {
            let label = server.label().to_string();
            if !quiet {
                eprintln!("connecting MCP server: {label}");
            }

            let allowed_tools = server.allowed_tools().to_vec();

            let service = match &server {
                McpServerConfig::Http {
                    endpoint, headers, ..
                } => connect_http(endpoint, headers)
                    .await
                    .with_context(|| format!("failed to connect MCP HTTP server '{label}'"))?,
                McpServerConfig::Stdio { command, args, .. } => connect_stdio(command, args)
                    .await
                    .with_context(|| format!("failed to connect MCP stdio server '{label}'"))?,
            };

            let peer = service.peer().clone();

            out.push(ServerConnection {
                label,
                allowed_tools,
                peer,
                _service: service,
            });
        }

        Ok(Self {
            policy,
            servers: out,
        })
    }

    fn find_server(&self, label: &str) -> Result<&ServerConnection> {
        self.servers
            .iter()
            .find(|s| s.label == label)
            .with_context(|| format!("unknown MCP server label: {label}"))
    }

    fn tool_allowed(&self, server: &ServerConnection, tool_name: &str) -> bool {
        is_tool_allowed(self.policy, &server.allowed_tools, tool_name)
    }
}

#[async_trait]
impl McpBackend for RmcpBackend {
    async fn list_tools(&self, query: Option<&str>, server_label: Option<&str>) -> Result<Value> {
        let mut out = Vec::new();
        let query_lc = query.map(|q| q.to_ascii_lowercase());

        for server in &self.servers {
            if let Some(filter_label) = server_label {
                if filter_label != server.label {
                    continue;
                }
            }

            let tools = server
                .peer
                .list_all_tools()
                .await
                .with_context(|| format!("list_tools failed for server '{}'", server.label))?;

            for tool in tools {
                let tool_name = tool.name.to_string();
                if !self.tool_allowed(server, &tool_name) {
                    continue;
                }

                let description = tool.description.map(|d| d.to_string());

                if let Some(q) = &query_lc {
                    let haystack = format!(
                        "{} {}",
                        tool_name.to_ascii_lowercase(),
                        description
                            .as_deref()
                            .unwrap_or_default()
                            .to_ascii_lowercase()
                    );
                    if !haystack.contains(q) {
                        continue;
                    }
                }

                let schema = serde_json::to_value(&*tool.input_schema).unwrap_or(Value::Null);

                out.push(json!({
                    "server_label": server.label,
                    "tool_name": tool_name,
                    "description": description,
                    "json_schema": schema,
                }));
            }
        }

        Ok(json!({ "tools": out }))
    }

    async fn call_tool(
        &self,
        server_label: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let server = self.find_server(server_label)?;

        if !self.tool_allowed(server, tool_name) {
            tracing::warn!(
                policy = %self.policy,
                server = server_label,
                tool = tool_name,
                "MCP tool call blocked by allow policy"
            );
            bail!(
                "MCP tool blocked by allow policy '{}' (server={}, tool={})",
                self.policy,
                server_label,
                tool_name
            );
        }

        let arguments_obj = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => bail!("mcp_call_tool.arguments must be an object"),
        };

        let mut params = CallToolRequestParams::new(tool_name.to_string());
        if let Some(args) = arguments_obj {
            params = params.with_arguments(args);
        }

        let result = server.peer.call_tool(params).await.with_context(|| {
            format!("call_tool failed for server '{server_label}', tool '{tool_name}'")
        })?;

        let result_json =
            serde_json::to_value(result).context("failed to serialize MCP tool result")?;

        Ok(json!({
            "server_label": server_label,
            "tool_name": tool_name,
            "result": result_json
        }))
    }

    async fn list_resources(&self, server_label: Option<&str>) -> Result<Value> {
        let mut resources_out = Vec::new();

        for server in &self.servers {
            if let Some(filter_label) = server_label {
                if filter_label != server.label {
                    continue;
                }
            }

            let resources =
                server.peer.list_all_resources().await.with_context(|| {
                    format!("list_resources failed for server '{}'", server.label)
                })?;

            for resource in resources {
                resources_out.push(json!({
                    "server_label": server.label,
                    "uri": resource.uri,
                    "name": resource.name,
                    "title": resource.title,
                    "description": resource.description,
                    "mime_type": resource.mime_type,
                }));
            }
        }

        Ok(json!({ "resources": resources_out }))
    }

    async fn read_resource(&self, server_label: &str, uri: &str) -> Result<Value> {
        let server = self.find_server(server_label)?;

        let result = server
            .peer
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .with_context(|| {
                format!("read_resource failed for server '{server_label}', uri '{uri}'")
            })?;

        let result_json =
            serde_json::to_value(result).context("failed to serialize MCP resource result")?;

        Ok(json!({
            "server_label": server_label,
            "uri": uri,
            "result": result_json
        }))
    }
}

async fn connect_http(
    endpoint: &str,
    headers: &HashMap<String, String>,
) -> Result<RunningService<RoleClient, ()>> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint.to_string());
    let mut custom_headers = HashMap::<HeaderName, HeaderValue>::new();

    for (k, v) in headers {
        if k.eq_ignore_ascii_case("authorization") {
            let token = v
                .strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
                .unwrap_or(v)
                .to_string();
            config = config.auth_header(token);
            continue;
        }

        let header_name = HeaderName::from_bytes(k.as_bytes())
            .with_context(|| format!("invalid HTTP header name in MCP config: {k}"))?;
        let header_value = HeaderValue::from_str(v)
            .with_context(|| format!("invalid HTTP header value for '{k}'"))?;

        custom_headers.insert(header_name, header_value);
    }

    config = config.custom_headers(custom_headers);
    let transport = StreamableHttpClientTransport::from_config(config);

    ().serve(transport)
        .await
        .context("MCP HTTP client handshake failed")
}

/// Environment variables safe to pass through to MCP stdio child processes.
/// Everything else (API keys, tokens, secrets) is stripped via `env_clear()`.
const PASSTHROUGH_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "COLORTERM",
    "RUST_LOG",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
];

async fn connect_stdio(command: &str, args: &[String]) -> Result<RunningService<RoleClient, ()>> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);

    // Prevent leaking API keys and other secrets to MCP child processes.
    cmd.env_clear();
    for key in PASSTHROUGH_ENV {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }

    let child_transport = TokioChildProcess::new(cmd)
        .with_context(|| format!("failed to spawn MCP stdio server command: {command}"))?;

    ().serve(child_transport)
        .await
        .context("MCP stdio client handshake failed")
}

pub fn is_tool_allowed(policy: McpAllowPolicy, allowed_tools: &[String], tool_name: &str) -> bool {
    match policy {
        McpAllowPolicy::ReadOnly => is_read_only_tool_name(tool_name),
        McpAllowPolicy::AllowList => allowed_tools
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(tool_name)),
        McpAllowPolicy::All => true,
    }
}

pub fn is_read_only_tool_name(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();

    let positives = [
        "read", "list", "get", "fetch", "search", "query", "inspect", "describe",
    ];
    let negatives = [
        "write", "delete", "remove", "edit", "update", "create", "exec", "run", "shell", "spawn",
    ];

    let positive_match = positives
        .iter()
        .any(|token| name.starts_with(token) || name.contains(token));

    if !positive_match {
        return false;
    }

    if negatives.iter().any(|token| name.contains(token)) {
        return false;
    }

    // rm must be treated carefully to avoid blocking words like "format".
    if contains_rm_token(&name) {
        return false;
    }

    true
}

fn contains_rm_token(name: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "rm")
}

pub fn virtual_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            kind: "function".to_string(),
            function: FunctionDefinition {
                name: "mcp_list_tools".to_string(),
                description: "List available MCP tools across servers. Use this to search available tools before calling one.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Optional text to filter tool names/descriptions." },
                        "server_label": { "type": "string", "description": "Optional MCP server label to scope results." }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: FunctionDefinition {
                name: "mcp_call_tool".to_string(),
                description: "Call an MCP tool by server label, tool name, and JSON arguments.".to_string(),
                parameters: json!({
                    "type": "object",
                    "required": ["server_label", "tool_name", "arguments"],
                    "properties": {
                        "server_label": { "type": "string" },
                        "tool_name": { "type": "string" },
                        "arguments": { "type": "object" }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: FunctionDefinition {
                name: "mcp_list_resources".to_string(),
                description: "List resources exposed by MCP servers.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "server_label": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            kind: "function".to_string(),
            function: FunctionDefinition {
                name: "mcp_read_resource".to_string(),
                description: "Read a specific MCP resource URI from a given server label.".to_string(),
                parameters: json!({
                    "type": "object",
                    "required": ["server_label", "uri"],
                    "properties": {
                        "server_label": { "type": "string" },
                        "uri": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
        },
    ]
}

#[derive(Debug, Default, Deserialize)]
pub struct ListToolsArgs {
    pub query: Option<String>,
    pub server_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallToolArgs {
    pub server_label: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListResourcesArgs {
    pub server_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadResourceArgs {
    pub server_label: String,
    pub uri: String,
}
