# aihelp

[![CI](https://github.com/Al-Sarraf-Tech/aihelp/actions/workflows/ci.yml/badge.svg)](https://github.com/Al-Sarraf-Tech/aihelp/actions/workflows/ci.yml)

> CI runs on self-hosted runners managed by the [Haskell Orchestrator](https://github.com/Al-Sarraf-Tech/Haskell-Orchestrator).

`aihelp` is a cross-platform Rust CLI for LM Studio (OpenAI-compatible API) with optional MCP tool discovery and tool-calling orchestration.

## Install

From source:

```bash
cargo install --path .
```

From release binaries:

- Linux: `aihelp-<tag>-x86_64-unknown-linux-gnu.tar.gz`
- Windows: `aihelp-<tag>-x86_64-pc-windows-msvc.zip`

Download from GitHub Releases: <https://github.com/Al-Sarraf-Tech/aihelp/releases>

Manpage source lives at `man/aihelp.1`.

## Quick Start

Linux:

```bash
aihelp "Hello can you hear me?"
ls | aihelp "what is in this directory?"
cat script.sh | aihelp "explain what this script does and any risky commands"
aihelp --mcp "find the right tool to search my docs for X, then summarize"
aihelp --no-stream "same query without streaming"
```

Windows PowerShell:

```powershell
aihelp "Hello can you hear me?"
Get-ChildItem | aihelp "what is in this directory?"
Get-Content .\script.ps1 | aihelp "what does this script do?"
aihelp --mcp "find docs about topic X and summarize"
aihelp --no-stream "same query without streaming"
```

## Setup Flow

First interactive run auto-launches setup wizard and stores `config.toml`.

Wizard includes:

- LM Studio endpoint suggestion and prompt.
- Model detection from `/v1/models` and default-model prompt.
- MCP default enable/disable prompt.
- Optional local MCP HTTP endpoint auto-discovery (common localhost ports).

Run setup again anytime:

```bash
aihelp --setup
```

## Discoverability Commands

- List common flags quickly: `aihelp --list-flags`
- List callable models from endpoint: `aihelp --list-models`
- List configured endpoints and status: `aihelp --list-endpoints`
- Switch default model on the fly: `aihelp --model <ID>`

`--model <ID>` behavior:

- With no question: validates and updates config default model, then exits.
- With a question: runs request and persists that model as default.

## Core Usage Flags

- `--endpoint <URL>`
- `--api-key <KEY>`
- `--model <ID>`
- `--stream`
- `--no-stream`
- `--max-stdin-bytes <N>`
- `--timeout-secs <N>`
- `--retries <N>`
- `--retry-backoff-ms <N>`
- `--json`
- `--quiet`
- `--print-model`
- `--dry-run`
- `--setup`
- `--list-flags`
- `--list-models`
- `--list-endpoints`

MCP flags:

- `--mcp`
- `--no-mcp`
- `--mcp-policy <read_only|allow_list|all>`
- `--mcp-max-tool-calls <N>`
- `--mcp-max-round-trips <N>`

`aihelp --help` includes an in-terminal manpage section with setup, model switching, MCP workflow, and troubleshooting reminders.

You can also read the roff manpage directly:

```bash
man ./man/aihelp.1
```

Streaming behavior:

- Streaming output is enabled by default.
- Use `--no-stream` for a single non-streaming response.
- Keep `--stream` for explicitness in scripts.

## Defaults

- Endpoint: `http://192.168.50.2:1234`
- Model: `openai/gpt-oss-20b`
- Config file:
  - Linux: `~/.config/aihelp/config.toml`
  - Windows: `%APPDATA%\aihelp\config.toml`

## Config Example

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

## Multi-Endpoint Support

Configure multiple LM Studio endpoints with automatic selection:

```toml
endpoint = "http://192.168.50.2:1234"
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

Strategies:

| Strategy | Behavior |
|---|---|
| `preferred` | Try endpoints in priority order, use first reachable (default) |
| `fallback` | Same as preferred — try in order, skip unreachable |
| `parallel_probe` | Probe all in parallel, pick first reachable by priority (alias: `round_robin`) |
| `model_route` | Route specific models to specific endpoints via `[model_routing]` |

Model routing example:

```toml
endpoint_strategy = "model_route"

[model_routing]
"small-model" = "local-arc"
"large-model" = "remote"
```

Override for a single run:

```bash
aihelp --endpoint http://127.0.0.1:1235 "question"
aihelp --endpoint local-arc "question"   # match by label
```

The setup wizard auto-prepends `http://` when a bare `host:port` is entered.

## Safety Defaults

- No shell-command execution from model output.
- No local file modifications from model output.
- MCP default policy: `read_only`.
- Write/exec-like tools blocked unless policy is loosened (`allow_list` or `all`).
- Empty query guard: fails fast with a clear error if no question and no stdin is provided.
- Stdin capped at 50 MiB to prevent OOM on unreasonable inputs; configurable via `--max-stdin-bytes`.
- Stdin is truncated at a valid UTF-8 boundary to avoid replacement characters in the prompt.
- Endpoint port validation: rejects bare hostnames or malformed URLs before any network attempt.

## Troubleshooting

LM Studio not reachable:

- Verify endpoint and network path.
- Check models endpoint directly: `curl <endpoint>/v1/models`

Model missing:

- `aihelp` fails fast when selected model is absent.
- Use `aihelp --list-models` then `aihelp --model <ID>`.

MCP server not found:

- Run `aihelp --setup` and enable MCP scan.
- Confirm mapped host ports and endpoint path (`/mcp`).
- If MCP is enabled but no servers are configured, `aihelp` now falls back to non-MCP mode for that run and warns on stderr.

MCP tool blocked:

- Keep `read_only` for safety by default.
- Override per run: `--mcp-policy allow_list` or `--mcp-policy all`.

Timeouts / intermittent latency:

- Increase timeout: `--timeout-secs 180`
- Increase retries: `--retries 3 --retry-backoff-ms 800`
- Bypass MCP for a quick baseline: `--no-mcp`

## CI/CD and Security

- CI workflow (`ci.yml`) runs compile sanity + full regression and security gate.
- Security workflow (`security.yml`) runs dependency audit.
- Release workflow (`release.yml`) builds and publishes Linux/Windows binaries on tags.

## Self-Hosted Runner (Optional)

Repo includes a Dockerized self-hosted runner setup for full regression/security:

- Assets: `ops/runner/`
- Host path target: `/docker/aihelp/runner`

Install:

```bash
bash ops/runner/install_to_docker_aihelp.sh
```

Start:

```bash
cd /docker/aihelp/runner
docker compose up -d
```

## CI/CD & Orchestration

This project is governed by the [Haskell Orchestrator](https://github.com/Al-Sarraf-Tech/Haskell-Orchestrator) — a Haskell-based multi-agent CI/CD governance framework for pre-push validation, code quality enforcement, and release management across the Al-Sarraf-Tech organization.
