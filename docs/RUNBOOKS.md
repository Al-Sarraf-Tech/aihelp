# aihelp Runbooks

Operator playbook for diagnosing and recovering from aihelp failures. Each
entry maps a symptom (stderr message, JSON log line, or CLI exit) to probable
cause, diagnostic steps, and recovery actions.

Logs are written as one JSON object per line at
`<cfg.logging.log_dir>/aihelp.log.YYYY-MM-DD` (default
`${XDG_CACHE_HOME:-~/.cache}/aihelp/logs/`). Filter with `jq`:

```bash
jq 'select(.level == "ERROR" or .level == "WARN")' \
  ~/.cache/aihelp/logs/aihelp.log.$(date -I)
```

To raise the level for a single run:

```bash
RUST_LOG=debug aihelp "test"
```

Per-op tracing fields you can grep on: `op`, `url`, `model`, `attempt`,
`status`, `latency_ms`, `outcome`, `tool`, `server`. Outcomes are
`ok`, `retry`, `error`, `blocked`, `limit_hit`, `round_limit_hit`.

---

## LM Studio Unreachable (Endpoint Down)

**Symptom**

```
Error: model verification failed. Check that LM Studio is running and the
model is loaded.
Caused by: request to /v1/models failed
Caused by: error sending request for url (http://192.168.50.2:1234/v1/models)
```

JSON:

```json
{"level":"ERROR","fields":{"op":"list_models","url":"http://192.168.50.2:1234/v1/models","attempt":3,"latency_ms":6000,"error":"...","outcome":"error","message":"transport error, no more retries"}}
```

**Cause** LM Studio is not running, the network path is broken, the wrong
endpoint URL is configured, or all probed endpoints in the strategy list
are down.

**Diagnose**

```bash
curl -sS http://192.168.50.2:1234/v1/models | head
aihelp --list-endpoints
ss -tnp | grep 1234   # local LM Studio
ping -c 2 192.168.50.2
```

**Recovery**

- Start LM Studio (and load a model — empty `/v1/models` response is also
  treated as "no callable models").
- Override for a single run: `aihelp --endpoint http://127.0.0.1:1234 ...`
- Edit `~/.config/aihelp/config.toml` `[[endpoints]]` blocks and rerun
  `aihelp --setup` to reauto-detect.
- Multi-endpoint setups: `aihelp --list-endpoints` to see which are
  unreachable; the strategy (`preferred` / `fallback` / `parallel_probe` /
  `model_route`) determines what gets tried.

---

## Model Not Found / Wrong Model Loaded

**Symptom**

```
Error: model verification failed. Check that LM Studio is running and the
model is loaded.
Caused by: default model 'openai/gpt-oss-20b' not found in /v1/models.
Available model IDs: liquid/lfm2-24b-a2b, ibm/granite-4-h-tiny.
```

**Cause** The configured `model` field does not match any ID returned by
`<endpoint>/v1/models`. Either the wrong model is loaded in LM Studio, or
the config drifted (e.g. quantization variant changed model ID).

**Diagnose**

```bash
aihelp --list-models
curl -sS http://<endpoint>/v1/models | jq '.data[].id'
```

**Recovery**

- Switch persistently: `aihelp --model <id-from-list>` — also persists to
  `~/.config/aihelp/config.toml`.
- One-off: `aihelp --model <id> "question"` (also persists).
- Load the requested model in LM Studio.
- See `~/.cache/aihelp/logs/aihelp.log.$(date -I)` for the exact model
  string sent in the last `chat_completion` request:
  `jq 'select(.fields.op == "chat_completion") | .fields.model'`.

---

## Request Timeout / Retry Exhaustion

**Symptom**

```
Error: chat completion failed
Caused by: /v1/chat/completions returned 504 after 3 attempts: ...
         Try increasing --timeout-secs or --retries.
```

JSON:

```json
{"level":"WARN","fields":{"op":"chat_completion","attempt":1,"status":504,"outcome":"retry","message":"retryable status, retrying"}}
{"level":"WARN","fields":{"op":"chat_completion","attempt":2,"status":504,"outcome":"retry","message":"retryable status, retrying"}}
{"level":"ERROR","fields":{"op":"chat_completion","status":504,"latency_ms":120000,"outcome":"error","message":"non-success status"}}
```

**Cause** The model is genuinely slow (large context, large model, cold
start), the endpoint is overloaded, or a network proxy is interposing.
Retryable HTTP statuses (`408`, `429`, `5xx`) are auto-retried up to
`config.retry_attempts` times with exponential backoff capped at 10s.

**Diagnose**

```bash
RUST_LOG=debug aihelp --timeout-secs 300 "test prompt"
jq 'select(.fields.op == "chat_completion") | {ts: .timestamp, attempt: .fields.attempt, status: .fields.status, latency_ms: .fields.latency_ms, outcome: .fields.outcome}' \
  ~/.cache/aihelp/logs/aihelp.log.$(date -I)
```

**Recovery**

- Bump timeout for the run: `aihelp --timeout-secs 300 "..."`
- Bump retries: `aihelp --retries 4 --retry-backoff-ms 1000 "..."`
- Persist defaults in `~/.config/aihelp/config.toml`:
  `timeout_secs = 300`, `retry_attempts = 4`, `retry_backoff_ms = 1000`.
- If LM Studio cold-start latency is the cause, keep the model loaded
  (LM Studio "always loaded" toggle).
- If a corporate HTTP proxy is interposing, check `HTTP_PROXY`/`HTTPS_PROXY`
  env vars. Local + RFC 1918 endpoints already bypass system proxy
  automatically (see `is_local_endpoint` in `src/client.rs`).

---

## Streaming Stalls / SSE Buffer Errors

**Symptom**

```
Error: SSE buffer exceeded 16 MiB without a complete event delimiter.
The server may be sending malformed SSE data.
```

Or:

```
Error: failed to read SSE chunk. Stream timed out after 3 attempts.
Try increasing --timeout-secs or --retries.
```

**Cause** The server returned a non-SSE response despite `stream=true` (in
which case aihelp falls back to JSON parse — that path is logged at
`debug`); OR the server sent malformed SSE without `\n\n` delimiters; OR
the connection stalled mid-stream after some bytes were already emitted
(in which case retry is suppressed to avoid duplicating output).

**Diagnose**

```bash
aihelp --debug-stream "test"   # per-token timestamps to stderr
jq 'select(.fields.op == "chat_completion_stream") | {chunks: .fields.chunks, text_bytes: .fields.text_bytes, finish_reason: .fields.finish_reason, latency_ms: .fields.latency_ms}' \
  ~/.cache/aihelp/logs/aihelp.log.$(date -I)
```

**Recovery**

- Disable streaming for the run: `aihelp --no-stream "..."` — falls back
  to non-stream `chat_completion` which is more tolerant of weird servers.
- Bump timeout: `aihelp --timeout-secs 300 "..."`.
- If a custom OpenAI-compatible server is the endpoint, verify it sends
  proper SSE (`Content-Type: text/event-stream`, events delimited by
  `\n\n`, terminated by `data: [DONE]\n\n`).

---

## MCP Server Connection Failed

**Symptom**

```
Error: failed to initialize MCP backend
Caused by: failed to connect MCP HTTP server 'mempalace'
Caused by: MCP HTTP client handshake failed
```

JSON:

```json
{"level":"ERROR","fields":{"op":"mcp_connect","server":"mempalace","transport":"http","latency_ms":3000,"error":"...","outcome":"error","message":"connect failed"}}
```

**Cause** The MCP server URL is wrong, the server is down, the auth header
is invalid, or (for stdio) the command is missing from PATH.

**Diagnose**

```bash
# For HTTP MCP
curl -sS -H "Authorization: Bearer $TOKEN" http://localhost:8096/health

# For stdio MCP
which <command-name>
<command-name> --help

# Check what aihelp is actually trying
jq 'select(.fields.op == "mcp_connect")' \
  ~/.cache/aihelp/logs/aihelp.log.$(date -I)
```

**Recovery**

- Run `aihelp --setup` to re-detect MCP HTTP servers on common ports
  (7000-7003, 8000, 8080, 8081, 9000).
- Edit `~/.config/aihelp/config.toml` `[mcp.servers]` block. For HTTP:
  fix `endpoint`. For stdio: fix `command` (must be on PATH or absolute).
- Skip MCP for one run: `aihelp --no-mcp "..."`.
- Disable MCP by default: set `mcp.enabled_by_default = false` in config.

---

## MCP Tool Blocked by Allow Policy

**Symptom**

```
Error: MCP tool blocked by allow policy 'read_only' (server=mempalace,
tool=mempalace_kg_add)
```

JSON:

```json
{"level":"WARN","fields":{"op":"mcp_call_tool","policy":"read_only","server":"mempalace","tool":"mempalace_kg_add","outcome":"blocked","message":"MCP tool call blocked by allow policy"}}
```

**Cause** The model attempted a tool that the configured policy forbids.
Default `read_only` allows `read*`, `list*`, `get*`, `fetch*`, `search*`,
`query*`, `inspect*`, `describe*` and rejects names containing `write`,
`delete`, `remove`, `edit`, `update`, `create`, `exec`, `run`, `shell`,
`spawn`, or the standalone token `rm`.

**Recovery**

- Per-run override: `aihelp --mcp-policy allow_list --mcp "..."` and
  populate `allowed_tools = ["..."]` for the server in config.
- Per-run wide-open: `aihelp --mcp-policy all --mcp "..."` (use with care
  — gives the model full tool access).
- Persist by editing `~/.config/aihelp/config.toml`:
  `mcp.allow_policy = "allow_list"` plus per-server `allowed_tools`.

---

## MCP Tool / Round-Trip Limit Reached

**Symptom**

```
MCP limits reached (tool calls: 8, rounds: 4). Output may be partial.
```

JSON:

```json
{"level":"WARN","fields":{"op":"run_mcp_loop","tool_calls_executed":8,"max_tool_calls":8,"round":4,"outcome":"limit_hit","message":"mcp tool-call limit reached"}}
```

**Cause** Defensive caps (`mcp.max_tool_calls = 8`, `mcp.max_round_trips
= 6`) tripped. Model is in a tool-spam loop or the task genuinely needs
more calls.

**Recovery**

- Per-run bump: `aihelp --mcp-max-tool-calls 24 --mcp-max-round-trips 12 "..."`
- Persist: edit `[mcp]` block in `~/.config/aihelp/config.toml`.
- If the model is looping on the same tool with the same args, switch to
  a stronger model (`aihelp --model <id>`) — small models are more prone
  to tool-spam loops.

---

## Config Parse Error

**Symptom**

```
Error: failed to load configuration
Caused by: failed to load existing config
Caused by: failed to parse config TOML: /home/.../config.toml
Caused by: TOML parse error at line 42, column 5
```

**Cause** Hand-edited `config.toml` produced invalid TOML, or a partial
write left the file truncated (atomic-write protection means this is rare,
but disk-full at write time could cause it).

**Diagnose**

```bash
cat ~/.config/aihelp/config.toml
toml-check ~/.config/aihelp/config.toml   # if toml-check is installed
```

**Recovery**

- Rerun `aihelp --setup` to regenerate the file.
- Hand-fix the TOML based on the line/column the parser cited.
- Last resort: `mv ~/.config/aihelp/config.toml{,.bad}` then run
  `aihelp --setup` for a fresh interactive setup.

---

## log_dir Unwritable / Disk Full

**Symptom** (on stderr, before the CLI runs)

```
aihelp: log_dir /mnt/nvmeINT/logs/aihelp not writable, JSON file logs
disabled: Permission denied (os error 13)
```

**Cause** The configured `logging.log_dir` is not writable by the running
user, or the underlying filesystem is full / read-only / unmounted.

**Recovery**

- Fail-open: aihelp continues without file logs; the CLI still works,
  output still goes to stdout/stderr. No remediation strictly needed for
  the immediate operation.
- Permanent fix: `chown $USER` the directory, or change `logging.log_dir`
  in `~/.config/aihelp/config.toml` to a writable path. Empty value falls
  back to `${XDG_CACHE_HOME:-~/.cache}/aihelp/logs/` which is per-user
  and almost always writable.

---

## Empty / Truncated Response from Server

**Symptom**

```
Error: server returned HTTP 200 with an empty body — the endpoint may be
misconfigured
```

Or:

```
Error: failed to parse /v1/models response
Caused by: error decoding response body: EOF while parsing a value at
line 1 column 0
```

**Cause** A reverse proxy in front of LM Studio (nginx, Cloudflare,
Tailscale Funnel) is buffering wrong, dropping the body, or returning an
HTML error page with HTTP 200.

**Diagnose**

```bash
curl -sSv http://<endpoint>/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"<id>","messages":[{"role":"user","content":"hi"}]}'
```

If the body is HTML or empty when `--list-models` succeeds, the proxy is
the culprit.

**Recovery**

- Bypass the proxy: point `aihelp --endpoint` directly at the LM Studio
  host (RFC 1918 / loopback addresses skip system proxy automatically).
- Disable proxy buffering for `/v1/chat/completions` (nginx:
  `proxy_buffering off`).
- If using Tailscale Funnel, switch to direct Tailscale (no Funnel) for
  long-running streaming responses.

---

## Health Check Procedure (Run After Any Infra Change)

```bash
# 1. Endpoint reachability
aihelp --list-endpoints

# 2. Model presence
aihelp --list-models

# 3. Fast non-streaming smoke
aihelp --no-stream --no-mcp "hi"

# 4. Streaming smoke
aihelp "hi"

# 5. MCP smoke (if MCP configured)
aihelp --mcp "what tools do you have?"

# 6. JSON log sanity
ls -lh ~/.cache/aihelp/logs/
jq 'select(.level == "ERROR")' ~/.cache/aihelp/logs/aihelp.log.$(date -I)
```

If any step fails, jump to the matching section above.
