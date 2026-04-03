use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::client::{ChatCompletionRequest, ChatMessage, OpenAiClient, ToolCall};
use crate::mcp::{
    virtual_tool_definitions, CallToolArgs, ListResourcesArgs, ListToolsArgs, McpBackend,
    ReadResourceArgs,
};
use crate::prompt::{build_user_message, StdinContext, SYSTEM_PROMPT};

/// Shared state for the `--debug-stream` per-token diagnostics.
struct DebugStreamState {
    enabled: bool,
    t0: Instant,
    last_token: Instant,
    token_count: usize,
}

impl DebugStreamState {
    fn new(enabled: bool) -> Self {
        let now = Instant::now();
        Self {
            enabled,
            t0: now,
            last_token: now,
            token_count: 0,
        }
    }

    /// Record a token and print live stats to stderr.  Call this from the
    /// `on_text_delta` callback.
    fn on_token(&mut self, delta: &str) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let gap = now.duration_since(self.last_token);
        self.last_token = now;
        self.token_count += 1;
        let elapsed = self.t0.elapsed();
        eprint!(
            "\r\x1b[K[stream] tok={} t={:.1}ms gap={:.1}ms len={}",
            self.token_count,
            elapsed.as_secs_f64() * 1000.0,
            gap.as_secs_f64() * 1000.0,
            delta.len(),
        );
    }

    /// Print the final summary line to stderr.
    fn finish(&mut self) {
        if !self.enabled {
            return;
        }
        self.enabled = false; // prevent double-print from Drop
        let secs = self.t0.elapsed().as_secs_f64();
        let tok_per_sec = if secs > 0.0 {
            self.token_count as f64 / secs
        } else {
            0.0
        };
        eprintln!(
            "\n[stream] done: {} tokens in {:.1}ms ({:.1} tok/s)",
            self.token_count,
            secs * 1000.0,
            tok_per_sec,
        );
    }
}

impl Drop for DebugStreamState {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug, Clone)]
pub struct AgentRunOptions {
    pub model: String,
    pub stream: bool,
    pub json: bool,
    pub dry_run: bool,
    pub quiet: bool,
    pub mcp_enabled: bool,
    pub mcp_max_tool_calls: usize,
    pub mcp_max_round_trips: usize,
    pub debug_stream: bool,
}

pub async fn run_agent(
    client: &OpenAiClient,
    mcp_backend: Option<&dyn McpBackend>,
    question: &str,
    stdin_context: Option<&StdinContext>,
    opts: &AgentRunOptions,
) -> Result<()> {
    let mut messages = vec![
        ChatMessage::system(SYSTEM_PROMPT),
        ChatMessage::user(build_user_message(question, stdin_context)),
    ];

    if !opts.mcp_enabled {
        run_single_turn(client, &messages, opts).await
    } else {
        run_mcp_loop(client, mcp_backend, &mut messages, opts).await
    }
}

async fn run_single_turn(
    client: &OpenAiClient,
    messages: &[ChatMessage],
    opts: &AgentRunOptions,
) -> Result<()> {
    let req = ChatCompletionRequest {
        model: opts.model.clone(),
        messages: messages.to_vec(),
        tools: None,
        tool_choice: None,
        stream: opts.stream,
    };

    if opts.dry_run {
        print_json(&client.dry_run_payload(&req))?;
        return Ok(());
    }

    if opts.stream {
        if opts.json {
            client
                .chat_completion_stream(
                    &req,
                    |_| Ok(()),
                    |chunk| print_json_line(&json!({ "event": "chunk", "data": chunk })),
                )
                .await
                .context("streaming chat completion failed")?;
            print_json_line(&json!({ "event": "done" }))?;
            return Ok(());
        }

        let mut dbg = DebugStreamState::new(opts.debug_stream);
        client
            .chat_completion_stream(
                &req,
                |delta| {
                    dbg.on_token(delta);
                    print!("{delta}");
                    flush_stdout()
                },
                |_| Ok(()),
            )
            .await
            .context("streaming chat completion failed")?;
        dbg.finish();
        println!();
        return Ok(());
    }

    let response = client
        .chat_completion(&req)
        .await
        .context("chat completion failed")?;

    if opts.json {
        print_json(&response.raw_json)?;
        return Ok(());
    }

    let text = response
        .response
        .assistant_content()
        .unwrap_or("".to_string());
    println!("{text}");
    Ok(())
}

async fn run_mcp_loop(
    client: &OpenAiClient,
    mcp_backend: Option<&dyn McpBackend>,
    messages: &mut Vec<ChatMessage>,
    opts: &AgentRunOptions,
) -> Result<()> {
    let backend = mcp_backend.context("MCP enabled but backend is not initialized")?;

    let tools = virtual_tool_definitions();
    let mut tool_calls_executed = 0usize;
    let mut last_assistant_text = String::new();

    for round in 0..opts.mcp_max_round_trips {
        let request = ChatCompletionRequest {
            model: opts.model.clone(),
            messages: messages.clone(),
            tools: Some(tools.clone()),
            tool_choice: Some(Value::String("auto".to_string())),
            stream: false,
        };

        if opts.dry_run {
            print_json(&client.dry_run_payload(&request))?;
            return Ok(());
        }

        let response = client
            .chat_completion(&request)
            .await
            .map_err(|err| enrich_mcp_round_error(err, round + 1))?;

        let assistant_msg = response
            .response
            .first_assistant_message()
            .context("chat completion returned no assistant message")?
            .clone();

        if let Some(text) = &assistant_msg.content {
            last_assistant_text = text.clone();
        }

        let tool_calls = assistant_msg.tool_calls.clone().unwrap_or_default();

        // Always push the assistant message so the synthesis pass includes it.
        messages.push(ChatMessage::assistant(
            assistant_msg.content.clone(),
            if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.clone())
            },
        ));

        if tool_calls.is_empty() {
            if opts.stream {
                // Stream a final synthesis pass without tools so output can be incremental.
                let final_request = ChatCompletionRequest {
                    model: opts.model.clone(),
                    messages: messages.clone(),
                    tools: None,
                    tool_choice: None,
                    stream: true,
                };

                if opts.json {
                    client
                        .chat_completion_stream(
                            &final_request,
                            |_| Ok(()),
                            |chunk| print_json_line(&json!({ "event": "chunk", "data": chunk })),
                        )
                        .await
                        .context("final streaming synthesis failed")?;
                    print_json_line(&json!({
                        "event": "done",
                        "tool_calls_executed": tool_calls_executed,
                        "round_trips_used": round + 1
                    }))?;
                    return Ok(());
                }

                let mut dbg = DebugStreamState::new(opts.debug_stream);
                client
                    .chat_completion_stream(
                        &final_request,
                        |delta| {
                            dbg.on_token(delta);
                            print!("{delta}");
                            flush_stdout()
                        },
                        |_| Ok(()),
                    )
                    .await
                    .context("final streaming synthesis failed")?;
                dbg.finish();
                println!();
                return Ok(());
            }

            if opts.json {
                print_json(&response.raw_json)?;
                return Ok(());
            }

            println!("{}", assistant_msg.content.unwrap_or_default());
            return Ok(());
        }

        for call in tool_calls {
            if tool_calls_executed >= opts.mcp_max_tool_calls {
                emit_limit_hit(opts, &last_assistant_text, tool_calls_executed, round + 1)?;
                return Ok(());
            }

            tool_calls_executed += 1;

            let tool_call_id = if call.id.trim().is_empty() {
                format!("tool_call_{tool_calls_executed}")
            } else {
                call.id.clone()
            };

            let result = execute_virtual_tool(backend, &call).await;

            if opts.json {
                print_json_line(&json!({
                    "event": "tool_result",
                    "tool_call_id": tool_call_id,
                    "tool": call.function.name,
                    "data": result,
                }))?;
            }

            messages.push(ChatMessage::tool(tool_call_id, result.to_string()));
        }
    }

    emit_limit_hit(
        opts,
        &last_assistant_text,
        tool_calls_executed,
        opts.mcp_max_round_trips,
    )?;
    Ok(())
}

fn enrich_mcp_round_error(err: anyhow::Error, round: usize) -> anyhow::Error {
    let lower = err.to_string().to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        return err.context(format!(
            "chat completion failed at MCP round {round}. The request timed out. Try --timeout-secs <N>, --retries <N>, or run without MCP via --no-mcp."
        ));
    }
    err.context(format!("chat completion failed at MCP round {round}"))
}

async fn execute_virtual_tool(backend: &dyn McpBackend, call: &ToolCall) -> Value {
    let args_json = match serde_json::from_str::<Value>(&call.function.arguments) {
        Ok(v) => v,
        Err(err) => {
            return json!({
                "error": format!("invalid tool arguments JSON for '{}': {err}", call.function.name)
            });
        }
    };

    match call.function.name.as_str() {
        "mcp_list_tools" => {
            let parsed: ListToolsArgs = serde_json::from_value(args_json).unwrap_or_default();
            match backend
                .list_tools(parsed.query.as_deref(), parsed.server_label.as_deref())
                .await
            {
                Ok(value) => value,
                Err(err) => json!({ "error": err.to_string() }),
            }
        }
        "mcp_call_tool" => {
            let parsed = serde_json::from_value::<CallToolArgs>(args_json);
            match parsed {
                Ok(args) => match backend
                    .call_tool(&args.server_label, &args.tool_name, args.arguments)
                    .await
                {
                    Ok(value) => value,
                    Err(err) => json!({ "error": err.to_string() }),
                },
                Err(err) => json!({ "error": format!("invalid mcp_call_tool args: {err}") }),
            }
        }
        "mcp_list_resources" => {
            let parsed: ListResourcesArgs = serde_json::from_value(args_json).unwrap_or_default();
            match backend.list_resources(parsed.server_label.as_deref()).await {
                Ok(value) => value,
                Err(err) => json!({ "error": err.to_string() }),
            }
        }
        "mcp_read_resource" => {
            let parsed = serde_json::from_value::<ReadResourceArgs>(args_json);
            match parsed {
                Ok(args) => match backend.read_resource(&args.server_label, &args.uri).await {
                    Ok(value) => value,
                    Err(err) => json!({ "error": err.to_string() }),
                },
                Err(err) => json!({ "error": format!("invalid mcp_read_resource args: {err}") }),
            }
        }
        other => json!({ "error": format!("unknown tool call requested by model: {other}") }),
    }
}

fn emit_limit_hit(
    opts: &AgentRunOptions,
    last_assistant_text: &str,
    tool_calls_executed: usize,
    round_trips_used: usize,
) -> Result<()> {
    let note = format!(
        "MCP limits reached (tool calls: {tool_calls_executed}, rounds: {round_trips_used}). Output may be partial."
    );

    if opts.json {
        print_json_line(&json!({
            "event": "limits_reached",
            "tool_calls_executed": tool_calls_executed,
            "round_trips_used": round_trips_used,
            "note": note,
            "partial_answer": last_assistant_text,
        }))?;
        return Ok(());
    }

    if !last_assistant_text.is_empty() {
        println!("{last_assistant_text}");
    }
    eprintln!("{note}");
    Ok(())
}

fn print_json(value: &Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to serialize JSON output")?
    );
    Ok(())
}

fn print_json_line(value: &Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(value).context("failed to serialize NDJSON line")?
    );
    Ok(())
}

/// Flush stdout, propagating broken-pipe as an error so the streaming
/// loop exits cleanly when the downstream consumer (e.g. `head`, `grep`)
/// closes the pipe.
fn flush_stdout() -> Result<()> {
    use std::io::{ErrorKind, Write};
    if let Err(e) = std::io::stdout().flush() {
        if e.kind() == ErrorKind::BrokenPipe {
            return Err(anyhow::anyhow!("stdout closed (broken pipe)"));
        }
        return Err(anyhow::Error::from(e).context("failed to flush stdout"));
    }
    Ok(())
}

pub trait ChatResponseHelper {
    fn first_assistant_message(&self) -> Option<&ChatMessage>;
    fn assistant_content(&self) -> Option<String>;
}

impl ChatResponseHelper for crate::client::ChatCompletionResponse {
    fn first_assistant_message(&self) -> Option<&ChatMessage> {
        self.choices.first().map(|c| &c.message)
    }

    fn assistant_content(&self) -> Option<String> {
        self.first_assistant_message()
            .and_then(|m| m.content.clone())
    }
}
