# aihelp

[![CI](https://github.com/Al-Sarraf-Tech/aihelp/actions/workflows/ci-rust.yml/badge.svg)](https://github.com/Al-Sarraf-Tech/aihelp/actions/workflows/ci-rust.yml)

> CI runs on self-hosted runners governed by the [Haskell Orchestrator](https://github.com/Al-Sarraf-Tech/Haskell-Orchestrator).

`aihelp` is a Rust CLI that sends natural-language questions to any OpenAI-compatible LM Studio endpoint, with optional MCP tool discovery and multi-turn tool-calling orchestration. Streaming is on by default. Stdin piping is first-class.

## Architecture

```
stdin / question
      │
      ▼
  aihelp (Rust, single binary)
      │
      ├── config.toml  (endpoint, model, MCP servers, strategy)
      │
      ├── endpoint selector  (preferred / fallback / parallel_probe / model_route)
      │         │
      │         ▼
      │    LM Studio  (OpenAI-compatible /v1/chat/completions, /v1/models)
      │
      └── MCP backend  (rmcp 1.0)
                │
                ├── HTTP transport  → MCP server over Streamable HTTP
                └── stdio transport → MCP server spawned as child process
```

When `--mcp` is active, the agent runs a multi-turn loop: the model calls virtual tools (`mcp_list_tools`, `mcp_call_tool`, `mcp_list_resources`, `mcp_read_resource`) and `aihelp` dispatches them to configured MCP servers, appending results as tool messages. The loop terminates when the model produces a final answer or a configured limit is reached.

MCP stdio child processes run with a stripped environment — only safe, non-secret vars (`PATH`, `HOME`, `LANG`, etc.) are forwarded.

## Installation

### Release binaries (Linux x86_64)

Download from [GitHub Releases](https://github.com/Al-Sarraf-Tech/aihelp/releases):

```bash
tar -xzf aihelp-v0.3.3-x86_64-unknown-linux-gnu.tar.gz
install -m 755 aihelp ~/.local/bin/
```

### From source

Requires Rust stable (1.70+):

```bash
cargo install --path .
```

Build profile: `lto = true`, `codegen-units = 1`, `strip = true`.

### Man page

```bash
man ./man/aihelp.1
```

## First-Run Setup

On first run with an interactive terminal, `aihelp` launches a setup wizard that:

1. Prompts for or confirms the LM Studio endpoint (default: `http://192.168.50.2:1234`).
2. Queries `/v1/models` and sets a default model.
3. Asks whether MCP should be enabled by default.
4. Scans common localhost ports for MCP HTTP endpoints.
5. Writes `~/.config/aihelp/config.toml` atomically.

Re-run setup at any time:

```bash
aihelp --setup
```

Non-interactive / CI mode (no prompts, MCP off by default):

```bash
AIHELP_NONINTERACTIVE=1 aihelp "question"
```

Override the config directory:

```bash
AIHELP_CONFIG_DIR=/path/to/dir aihelp "question"
```

## Usage

```bash
# Basic query
aihelp "what is the systemd unit for sshd?"

# Pipe stdin as context
ls -la | aihelp "what is in this directory?"
cat script.sh | aihelp "explain this script and flag risky commands"

# MCP tool-calling mode (requires configured MCP servers)
aihelp --mcp "search my docs for topic X and summarize"

# Disable streaming for one run
aihelp --no-stream "question"

# Use a specific model for one run and persist it as default
aihelp --model openai/gpt-oss-20b "question"

# Switch default model without asking a question
aihelp --model openai/gpt-oss-20b

# List available models from endpoint
aihelp --list-models

# List configured endpoints and reachability
aihelp --list-endpoints

# Print request payload without sending (debug)
aihelp --dry-run "question"
```

## Flags

### Core

| Flag | Default | Description |
|---|---|---|
| `--endpoint <URL>` | config | Override LM Studio base URL |
| `--api-key <KEY>` | config | Authorization bearer token |
| `--model <ID>` | config | Model ID; persists as default |
| `--stream` | on | Enable streaming (default) |
| `--no-stream` | — | Disable streaming for this run |
| `--max-stdin-bytes <N>` | 200000 | Max stdin bytes to include (hard cap 50 MiB) |
| `--timeout-secs <N>` | 120 | HTTP timeout |
| `--retries <N>` | 2 | Retry transient failures |
| `--retry-backoff-ms <N>` | 500 | Base retry backoff |
| `--json` | off | JSON / NDJSON output |
| `--quiet` | off | Suppress stderr diagnostics |
| `--print-model` | off | Print selected model to stderr |
| `--dry-run` | off | Print request payload, no API calls |
| `--debug-stream` | off | Per-token timestamps to stderr |

### Discovery

| Flag | Description |
|---|---|
| `--list-flags` | Print flag reference |
| `--list-models` | List models from endpoint |
| `--list-endpoints` | List endpoints and reachability |
| `--setup` | Run interactive setup wizard |

### MCP

| Flag | Default | Description |
|---|---|---|
| `--mcp` | — | Enable MCP tools for this run |
| `--no-mcp` | — | Disable MCP tools for this run |
| `--mcp-policy <policy>` | `read_only` | Tool allow policy |
| `--mcp-max-tool-calls <N>` | 8 | Max total tool calls in agent loop |
| `--mcp-max-round-trips <N>` | 6 | Max LLM round trips in agent loop |

`--mcp-policy` values:

| Policy | Behavior |
|---|---|
| `read_only` | Allow only tools whose name contains `read`, `list`, `get`, `fetch`, `search`, `query`, `inspect`, or `describe` — and does not contain write/exec keywords |
| `allow_list` | Allow only tools listed in `allowed_tools` per server |
| `all` | Allow all tools |

## Configuration

Config file location:

- Linux: `~/.config/aihelp/config.toml`
- Windows: `%APPDATA%\aihelp\config.toml`

Override: `AIHELP_CONFIG_DIR=/path`

### Full config example

```toml
endpoint = "http://192.168.50.2:1234"
model = "openai/gpt-oss-20b"
stream_by_default = true
max_stdin_bytes = 200000
timeout_secs = 120
retry_attempts = 2
retry_backoff_ms = 500

[mcp]
enabled_by_default = false
allow_policy = "read_only"
max_tool_calls = 8
max_round_trips = 6

[[mcp.servers]]
label = "mytools"
transport = "http"
endpoint = "http://127.0.0.1:7000/mcp"
allowed_tools = ["search_docs", "read_file"]
headers = { Authorization = "Bearer XYZ" }

[[mcp.servers]]
label = "internal"
transport = "stdio"
command = "node"
args = ["./path/to/mcp-server.js"]
allowed_tools = ["list_things"]
```

### Multi-endpoint configuration

```toml
endpoint_strategy = "fallback"

[[endpoints]]
label = "local-arc"
url = "http://192.168.50.5:1235"
priority = 0

[[endpoints]]
label = "remote"
url = "http://192.168.50.2:1234"
priority = 1
```

| Strategy | Behavior |
|---|---|
| `preferred` | Try endpoints in priority order; use first reachable (default) |
| `fallback` | Same as preferred |
| `parallel_probe` | Probe all in parallel; pick first reachable by priority. Alias: `round_robin` |
| `model_route` | Route specific models to specific endpoints via `[model_routing]` |

Model routing:

```toml
endpoint_strategy = "model_route"

[model_routing]
"small-model" = "local-arc"
"large-model" = "remote"
```

Override for a single run by URL or label:

```bash
aihelp --endpoint http://127.0.0.1:1235 "question"
aihelp --endpoint local-arc "question"
```

## Safety

- No shell execution or local file modification from model output.
- MCP default policy is `read_only`; write/exec tools are blocked unless policy is explicitly loosened.
- MCP stdio child processes receive a stripped environment — no API keys or tokens are forwarded.
- Stdin is hard-capped at 50 MiB and truncated at a valid UTF-8 boundary.
- Empty query with no stdin fails fast with a clear error.
- Endpoint port and URL are validated before any network attempt.
- If MCP is enabled but no servers are configured, `aihelp` warns and falls back to non-MCP mode for that run.

## Troubleshooting

**LM Studio not reachable:**

```bash
curl http://192.168.50.2:1234/v1/models
aihelp --endpoint http://127.0.0.1:1234 "test"
```

**Model missing from endpoint:**

```bash
aihelp --list-models
aihelp --model <ID>
```

**MCP server not connecting:**

- Run `aihelp --setup` to re-scan and reconfigure MCP servers.
- Verify the MCP endpoint path (typically `/mcp`) and port.

**MCP tool blocked:**

```bash
# Allow only listed tools
aihelp --mcp-policy allow_list --mcp "question"

# Allow all tools (use with caution)
aihelp --mcp-policy all --mcp "question"
```

**Slow or timing out:**

```bash
aihelp --timeout-secs 180 --retries 3 --retry-backoff-ms 800 "question"
# Bypass MCP for a quick baseline
aihelp --no-mcp "question"
```

**Stream diagnostics:**

```bash
aihelp --debug-stream "question"
```

## CI/CD

- `ci-rust.yml`: fmt check, clippy (`-D warnings`), tests, `cargo-audit`, `cargo-deny`, gitleaks. Releases Linux x86_64 binaries on `v*` tags after all gates pass.
- `orchestrator-scan.yml`: runs the Haskell Orchestrator scan on workflow file changes.
- CI runs on `Al-Sarraf-Tech/aihelp` (org repo). No CI on personal fork.

### Self-hosted runner

```bash
bash ops/runner/install_to_docker_aihelp.sh
cd /docker/aihelp/runner && docker compose up -d
```

Assets: `ops/runner/`. Target host path: `/docker/aihelp/runner`.

## Building and testing

```bash
cargo build --release
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```
