# CLAUDE.md — aihelp

Rust CLI tool for querying LM Studio models from the terminal, with optional MCP tool
discovery. Single binary. Config at `~/.config/aihelp/config.toml`.

Default endpoint: `http://192.168.50.2:1234`

---

## Organizational Directive (Claude Only)

> **This directive applies ONLY when Claude Code is in use — it is a standing operational policy, not a suggestion.**
>
> Claude operates in this repository as a structured internal engineering organization: single point of contact, adaptive team complexity (Tier 0–4), mandatory review on all work, batch processing, and parallelization where safe. Full directive: `~/.claude/CLAUDE.md`.

---

## Build and Run

```bash
cargo build --release          # build optimised binary
cargo install --path .         # install to ~/.cargo/bin/
aihelp --setup                 # interactive config wizard
aihelp --list-models           # list available models
aihelp "your question"         # basic query
cat file.sh | aihelp "explain this"   # pipe stdin
aihelp --mcp "query with MCP tools"   # use MCP toolchain
```

---

## Development Commands

```bash
cargo build                    # debug build
cargo test                     # all tests
cargo test <test_name>         # single test
cargo fmt --all --check        # format check (CI gate)
cargo clippy -- -D warnings    # lint (zero warnings, CI gate)
cargo run -- "question"        # run without installing
```

---

## CI Gate (must be clean before commit/push)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

---

## Source Layout

```
src/
  main.rs       # Entry point, CLI dispatch
  config.rs     # Config loading/saving (~/.config/aihelp/config.toml)
  client.rs     # LM Studio HTTP client (streaming + non-streaming)
  agent.rs      # Agentic loop for multi-turn reasoning
  mcp.rs        # MCP tool discovery and invocation
  prompt.rs     # Prompt construction helpers
  setup.rs      # Interactive setup wizard
  lib.rs        # Library re-exports
ops/            # CI runner setup
scripts/        # Utility helpers
man/            # Man page
tests/          # Integration tests (assert_cmd-based)
```

---

## MCP Policy

Controlled via config. Three modes: `read_only` (default), `allow_list`, `all`.
Do not default to `all` in any code path without explicit user config.

---

## Release Profile

`Cargo.toml` release profile: `codegen-units = 1`, `lto = true`, `strip = true`.
Do not weaken these for production releases.

---

## Conventions

- Use `anyhow::Result` for error propagation throughout; no `.unwrap()` in library code.
- Async runtime: `tokio` multi-thread. Do not add `std::thread::spawn` for I/O work.
- TLS via `rustls` (no OpenSSL dependency). Do not add `openssl` feature flags.
- Config is TOML. Do not introduce JSON or YAML config formats.
- All new CLI flags go through `clap` derive macros in `main.rs`.

---

## Validation

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
aihelp --list-models           # smoke test against live LM Studio
```

---

## Toolchain

| Tool | Path | Version |
|---|---|---|
| rustc | `/usr/bin/rustc` | 1.93.1 (Fedora dnf) |
| cargo | `/usr/bin/cargo` | 1.93.1 (Fedora dnf) |
| rustfmt | `/usr/bin/rustfmt` | 1.93.1 |
| rust-analyzer | `/usr/bin/rust-analyzer` | 1.93.1 |

Rust is system-installed via dnf, not rustup shims. `rustup` is present at `~/.rustup` but
its shims are not active — `/usr/bin/rustc` takes priority.
`~/.cargo/bin/` is in PATH for user-installed cargo tools (cargo-audit, cargo-deny, aihelp, cyberdeck).
